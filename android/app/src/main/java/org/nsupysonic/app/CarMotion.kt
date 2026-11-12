package org.nsupysonic.app

import android.annotation.SuppressLint
import android.content.Context
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import com.google.android.gms.location.FusedLocationProviderClient
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import kotlin.math.roundToInt

/**
 * How fast the car is going, and whether it is "moving" in the sense the car
 * UI cares about.
 *
 * The fused provider is asked for one fix a second at high accuracy, which is
 * the interval a navigation app uses for a speed readout
 * (developer.android.com/develop/sensors-and-location/location/change-location-settings).
 * A last-known fix seeds the first second, but only when it is recent enough to
 * be about this drive. A phone without Google Play services, or a fused request
 * that says nothing for [FALLBACK_AFTER_MS], falls back to the platform GPS;
 * once on that path we stay there (no flapping between two providers).
 *
 * Three rules, and they are the ones the car UI depends on:
 *
 * - at or above [CarMotionState.MOVING_KMH] the car is moving at once;
 * - below it, it keeps saying "moving" for [CarMotionState.STOP_GRACE_MS] — a
 *   red light is not a parked car;
 * - an UNKNOWN speed always means moving: no fix yet, a fix without a speed
 *   (approximate location), no fix for [CarMotionState.UNKNOWN_AFTER_MS], or no
 *   location permission at all. A GPS that failed must never unlock the
 *   keyboard of someone driving.
 *
 * Everything is delivered on the main thread. [start] and [stop] are
 * idempotent and may be called from any thread.
 */
class CarMotion(context: Context, private val listener: Listener) {

    /** Main thread; [speedKmh] is the last measured speed (0 before one), [moving] is the safe answer. */
    fun interface Listener {
        fun onMotion(speedKmh: Double, moving: Boolean)
    }

    private val app = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val locationManager = app.getSystemService(Context.LOCATION_SERVICE) as? LocationManager
    private val state = CarMotionState()

    private var started = false
    private var fused: FusedLocationProviderClient? = null
    private var fusedOn = false
    private var platformOn = false
    // A provider that refused once is not retried every second: without GMS
    // there is nothing to retry, and a disabled GPS would only throw again.
    private var providerFailed = false
    private var hadPermission = false
    // Last live fused callback (not a last-known fix): what the 3 s fallback
    // measures. elapsedRealtime, so a clock change cannot fake it.
    private var lastLiveFusedAt = 0L
    private var lastEmittedKmh = Int.MIN_VALUE
    private var lastEmittedMoving = true

    /** The last measured speed in km/h; 0 until a fix carries one. */
    val speedKmh: Double get() = state.speedKmh

    /** True unless a measured speed stayed below 5 km/h for five seconds. */
    val moving: Boolean get() = state.moving

    private val tick = object : Runnable {
        override fun run() {
            if (!started) return
            main.postDelayed(this, UPDATE_MS)
            val now = SystemClock.elapsedRealtime()
            if (!CarPermissions.hasLocation(app)) {
                // Revoked (or never granted) while we run: stop the requests
                // rather than let them throw, and say "moving" until it is back.
                if (fusedOn || platformOn) stopLocation()
                hadPermission = false
                state.forget()
                publish()
                return
            }
            if (!hadPermission) {
                hadPermission = true
                providerFailed = false
            }
            if (!fusedOn && !platformOn && !providerFailed) startLocation()
            if (fusedOn && now - lastLiveFusedAt >= FALLBACK_AFTER_MS) {
                Log.i(TAG, "no fused fix for $FALLBACK_AFTER_MS ms: platform GPS")
                startPlatform()
            }
            state.silence(now)
            publish()
        }
    }

    /** Start measuring; safe to call twice. Main thread (or posted there). */
    fun start() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { start() }
            return
        }
        if (started) return
        started = true
        hadPermission = false
        providerFailed = false
        state.forget()
        lastEmittedKmh = Int.MIN_VALUE
        lastEmittedMoving = true
        publish() // the safe answer until the first fix
        if (CarPermissions.hasLocation(app)) {
            hadPermission = true
            startLocation()
        }
        main.postDelayed(tick, UPDATE_MS)
    }

    /** Stop measuring; safe to call twice. */
    fun stop() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { stop() }
            return
        }
        if (!started) return
        started = false
        main.removeCallbacks(tick)
        stopLocation()
        state.forget()
        hadPermission = false
        providerFailed = false
    }

    private fun startLocation() {
        if (!started || fusedOn || platformOn) return
        if (!CarPermissions.hasLocation(app)) return
        startFused()
    }

    /**
     * The fused provider. `LocationRequest.create()` and the Looper overload are
     * deprecated in play-services-location 21, but they exist in both the 20.x
     * and 21.x lines — this lot must compile against whatever version the car
     * service lot adds, not pin one. A silent or failing client falls back to
     * the platform GPS; nothing here is required to be on the main thread by
     * the client, but its callbacks are (we pass [main]'s looper).
     */
    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun startFused() {
        try {
            val client = LocationServices.getFusedLocationProviderClient(app)
            fused = client
            val request = LocationRequest.create().apply {
                setPriority(LocationRequest.PRIORITY_HIGH_ACCURACY)
                setInterval(UPDATE_MS)
                setFastestInterval(UPDATE_MS)
                setSmallestDisplacement(0f)
            }
            client.requestLocationUpdates(request, fusedCallback, main.looper)
            fusedOn = true
            lastLiveFusedAt = SystemClock.elapsedRealtime()
            // A recent fix makes the first second useful; an older one is the
            // previous drive's speed and is refused by useFix.
            client.lastLocation
                .addOnSuccessListener { location ->
                    if (location != null) useFix(location, live = false)
                }
                .addOnFailureListener { if (started && !platformOn) startPlatform() }
        } catch (t: Throwable) {
            // No Google Play services, a stub provider, or SecurityException:
            // the platform GPS is the documented fallback.
            Log.i(TAG, "fused location unavailable: ${t.javaClass.simpleName}")
            fusedOn = false
            startPlatform()
        }
    }

    /**
     * The platform GPS, for a phone without GMS or a fused provider that went
     * silent. The listener overrides the three callbacks that only have default
     * implementations on Android 11+: on API 26 they are abstract on the
     * platform's interface, and the framework does call them
     * (developer.android.com/reference/android/location/LocationListener).
     */
    @SuppressLint("MissingPermission")
    @Suppress("DEPRECATION")
    private fun startPlatform() {
        if (!started || platformOn) return
        fusedOn = false
        try {
            fused?.removeLocationUpdates(fusedCallback)
        } catch (_: Exception) {
        }
        val manager = locationManager ?: run {
            providerFailed = true
            return
        }
        val provider = when {
            manager.allProviders.contains(LocationManager.GPS_PROVIDER) -> LocationManager.GPS_PROVIDER
            manager.allProviders.contains(LocationManager.NETWORK_PROVIDER) -> LocationManager.NETWORK_PROVIDER
            else -> null
        }
        if (provider == null) {
            providerFailed = true
            return
        }
        try {
            manager.requestLocationUpdates(provider, UPDATE_MS, 0f, platformListener, main.looper)
            platformOn = true
            manager.getLastKnownLocation(provider)?.let { useFix(it, live = false) }
        } catch (_: SecurityException) {
            providerFailed = true
            state.forget()
            publish()
        } catch (_: Exception) {
            providerFailed = true
        }
    }

    private fun stopLocation() {
        if (fusedOn) {
            try {
                fused?.removeLocationUpdates(fusedCallback)
            } catch (_: Exception) {
            }
            fusedOn = false
        }
        if (platformOn) {
            try {
                locationManager?.removeUpdates(platformListener)
            } catch (_: Exception) {
            }
            platformOn = false
        }
        fused = null
    }

    private fun useFix(location: Location, live: Boolean) {
        if (!started) return
        val now = SystemClock.elapsedRealtime()
        if (live) {
            lastLiveFusedAt = now
        } else if (!fresh(location, now)) {
            return
        }
        // Location.getSpeed() is metres per second; a fix that carries none
        // (approximate or network location) says nothing about the speed.
        val kmh = if (location.hasSpeed() && location.speed.isFinite() && location.speed >= 0f) {
            kmhFromMps(location.speed)
        } else {
            null
        }
        state.onFix(kmh, now)
        publish()
    }

    /** A last-known fix older than [LAST_FIX_MAX_AGE_MS] is not this drive. */
    private fun fresh(location: Location, now: Long): Boolean {
        val at = location.elapsedRealtimeNanos / 1_000_000L
        return at > 0L && now - at in 0L..LAST_FIX_MAX_AGE_MS
    }

    /** Emit only when what the car UI shows changed: a whole km/h, or moving. */
    private fun publish() {
        val speed = state.speedKmh
        val moving = state.moving
        val kmh = speed.roundToInt()
        if (kmh == lastEmittedKmh && moving == lastEmittedMoving) return
        lastEmittedKmh = kmh
        lastEmittedMoving = moving
        listener.onMotion(speed, moving)
    }

    private val fusedCallback = object : LocationCallback() {
        override fun onLocationResult(result: LocationResult) {
            for (location in result.locations) useFix(location, live = true)
        }
    }

    @Suppress("DEPRECATION")
    private val platformListener = object : LocationListener {
        override fun onLocationChanged(location: Location) = useFix(location, live = true)

        // Default on Android 11+, abstract on the platform's interface below
        // that: an API 26 phone would call this through the interface and meet
        // an AbstractMethodError if the override were missing.
        @Deprecated("Required by the platform below API 30")
        override fun onStatusChanged(provider: String?, status: Int, extras: Bundle?) {}

        override fun onProviderEnabled(provider: String) {}

        override fun onProviderDisabled(provider: String) {}
    }

    companion object {
        private const val TAG = "CarMotion"

        /** One fix a second, and how often the fallback/permission state is checked. */
        const val UPDATE_MS = 1_000L

        /** No live fused fix for this long: use the platform GPS instead. */
        const val FALLBACK_AFTER_MS = 3_000L

        /** A last-known fix older than this belongs to a previous drive. */
        const val LAST_FIX_MAX_AGE_MS = 30_000L
    }
}

/**
 * The speed thresholds and the stop hysteresis, with no Android type in sight:
 * [CarMotionTest] holds them on the JVM. Unknown always means moving.
 */
internal class CarMotionState {

    @Volatile
    var moving: Boolean = true
        private set

    @Volatile
    var speedKmh: Double = 0.0
        private set

    var lastFixAt: Long = 0L
        private set

    /** When the measured speed first went below [MOVING_KMH]; null when it is not. */
    private var lowSince: Long? = null

    /**
     * A fix at [now] (elapsedRealtime ms). [kmh] is null when the fix carries no
     * speed; that is an unknown speed, not a stopped car.
     */
    fun onFix(kmh: Double?, now: Long) {
        lastFixAt = now
        if (kmh == null || !kmh.isFinite() || kmh < 0.0) {
            lowSince = null
            moving = true
            return
        }
        speedKmh = kmh
        if (kmh >= MOVING_KMH) {
            lowSince = null
            moving = true
            return
        }
        val since = lowSince
        if (since == null) {
            lowSince = now
            moving = true
            return
        }
        if (now - since >= STOP_GRACE_MS) moving = false
    }

    /** No fix for [UNKNOWN_AFTER_MS]: the speed is unknown, and unknown means moving. */
    fun silence(now: Long) {
        if (lastFixAt == 0L) {
            moving = true
            return
        }
        if (now - lastFixAt >= UNKNOWN_AFTER_MS) {
            lowSince = null
            moving = true
        }
    }

    /** Forget the measurement (start, stop, permission lost). */
    fun forget() {
        moving = true
        speedKmh = 0.0
        lastFixAt = 0L
        lowSince = null
    }

    companion object {
        /** At or above this the car is moving; below it the stop grace applies. */
        const val MOVING_KMH = 5.0

        /** A red light keeps the "moving" answer for this long after the car stopped. */
        const val STOP_GRACE_MS = 5_000L

        /** No fix for this long: the speed is unknown, and unknown means moving. */
        const val UNKNOWN_AFTER_MS = 5_000L
    }
}

/** Location.getSpeed() is metres per second; the whole car UI speaks km/h. */
internal fun kmhFromMps(mps: Float): Double = mps.toDouble() * 3.6
