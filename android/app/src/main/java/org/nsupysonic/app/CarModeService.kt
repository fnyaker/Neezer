package org.nsupysonic.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.SharedPreferences
import android.content.pm.ServiceInfo
import android.content.res.Configuration
import android.graphics.PixelFormat
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.view.Gravity
import android.view.WindowManager
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import org.json.JSONObject

/**
 * The car mode: a foreground service that owns the overlay window and the
 * PlayerService binding, never the audio.
 *
 * The page still owns the queue and the sound (PlayerService/MediaSession are
 * untouched); this service is a remote control that also happens to draw on
 * top of the navigation app. It is a `specialUse` foreground service because
 * the overlay must outlive the activity while a GPS app is in front — that is
 * exactly the use Android has no narrower type for.
 *
 * Auto-start is deliberately narrow: triggers on, overlay granted, the
 * configured server present, notifications usable. A quick-settings tile or
 * the settings screen starts it manually even when those are missing; the
 * overlay then explains what permission is blocking it.
 */
class CarModeService : Service(), CarHost {

    companion object {
        private const val ACTION_START = "org.nsupysonic.app.action.CAR_START"
        private const val ACTION_STOP = "org.nsupysonic.app.action.CAR_STOP"
        private const val ACTION_REFRESH = "org.nsupysonic.app.action.CAR_REFRESH"
        private const val EXTRA_MANUAL = "manual"

        const val NOTIFICATION_ID = 2
        private const val CHANNEL_ID = "car"

        private const val MAX_LAUNCH_ID = 200
        private const val MAX_LAUNCH_TITLE = 200
        // Quoted inside the expression below, so keep it under PlayerService.JS_MAX.
        private const val MAX_LAUNCH_PAYLOAD = 12 * 1024

        private val LAUNCH_KINDS = setOf("flow", "playlist", "album", "artist", "mix", "track")

        private const val TICK_MS = 1_000L
        private const val REBIND_MS = 1_000L
        private const val BOTTOM_DP = 92
        private const val BLUR_DP = 18
        private const val MAX_SPEED = 400.0
        private const val MAX_SEEK = 24.0 * 60 * 60

        @Volatile
        private var running = false

        /** True while a CarModeService instance is alive (the quick tile reads it). */
        fun isRunning(): Boolean = running

        /** Start (or bring forward) the car mode for the person who asked. */
        fun start(context: Context) {
            val intent = Intent(context, CarModeService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_MANUAL, true)
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (_: Exception) {
                // Background FGS-start restriction: the tile/activity is the
                // normal caller and is allowed; a failure leaves the mode off.
                try {
                    context.startService(intent)
                } catch (_: Exception) {
                }
            }
        }

        fun stop(context: Context) {
            try {
                context.stopService(Intent(context, CarModeService::class.java))
            } catch (_: Exception) {
            }
        }

        /**
         * Start only when the person's own settings say the mode has something
         * to watch. Returns false without starting anything (no zombie
         * notification) when a trigger is off, the overlay is not granted, the
         * server is not configured, or notifications would hide the one way to
         * stop it.
         */
        fun startIfEnabled(context: Context): Boolean {
            if (running) {
                // Re-apply permissions/settings changed while the service was
                // up (returning from the car settings screen, a permission
                // grant, a trigger switch).
                try {
                    context.startService(
                        Intent(context, CarModeService::class.java).setAction(ACTION_REFRESH)
                    )
                } catch (_: Exception) {
                }
                return true
            }
            if (!autoStartAllowed(context)) return false
            val intent = Intent(context, CarModeService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_MANUAL, false)
            return try {
                ContextCompat.startForegroundService(context, intent)
                true
            } catch (_: Exception) {
                false
            }
        }

        private fun autoStartAllowed(context: Context): Boolean {
            if (!Prefs(context).configured) return false
            val car = CarPrefs(context)
            if (!car.gpsTrigger && !car.bluetoothTrigger) return false
            if (!CarPermissions.hasOverlay(context)) return false
            // A GPS trigger without usage access can never fire. Bluetooth may
            // still be the reason the person is here, so only refuse when it is
            // the only configured trigger.
            if (car.gpsTrigger && !CarPermissions.hasUsageStats(context) &&
                !car.bluetoothTrigger
            ) return false
            // A foreground service whose notification is hidden has no visible
            // "Arrêter"; the tile remains, but auto-starting into that state is
            // not the person's choice.
            return NotificationManagerCompat.from(context).areNotificationsEnabled()
        }
    }

    private lateinit var carPrefs: CarPrefs
    private lateinit var windowManager: WindowManager
    private val main = Handler(Looper.getMainLooper())

    private var overlay: CarOverlayRoot? = null
    private var overlayAttached = false
    private var desiredSize = CarSize.FULL
    private var desiredScreen = CarScreen.PLAYER

    private var player: PlayerService? = null
    private var playerBound = false
    @Volatile private var lastState: PlayerService.State? = null

    private var trigger: CarTrigger? = null
    private var triggerVisible = false
    private var lastVisible = false
    private var manual = false
    private var dismissed = false
    /** The full-screen car settings is on top: never draw over it. */
    private var suppressOverlay = false
    private var moving = true
    private var speed = 0.0
    private var passenger = false

    private var foregrounded = false
    private var foregroundType = -1
    private var destroying = false
    private var notifKey = ""
    private var overlayBlockedToastShown = false

    private var lastGps = false
    private var lastBt = false
    private var lastPackages: Set<String> = emptySet()
    private var carSp: SharedPreferences? = null

    private val prefListener =
        SharedPreferences.OnSharedPreferenceChangeListener { _, _ ->
            main.post { onCarPrefsChanged() }
        }

    private val triggerListener = object : CarTrigger.Listener {
        override fun onDrive(visible: Boolean, moving: Boolean, speedKmh: Double) {
            main.post { handleDrive(visible, moving, speedKmh) }
        }
    }

    private val ticker = object : Runnable {
        override fun run() {
            if (!overlayAttached) return
            renderOverlay()
            main.postDelayed(this, TICK_MS)
        }
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val bound = (binder as? PlayerService.LocalBinder)?.service ?: return
            player = bound
            bound.onState { state -> main.post { onPlayerState(state) } }
            bound.state()?.let { onPlayerState(it) }
            updateNotification()
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            player?.onState(null)
            player = null
            // The player (and the audio) is gone: do not keep drawing its last
            // track as if it were still playable.
            lastState = null
            if (overlayAttached) renderOverlay()
            // The binding itself stays alive: the player process may come back.
            updateNotification()
        }

        override fun onBindingDied(name: ComponentName?) {
            player = null
            if (playerBound) {
                try {
                    unbindService(this)
                } catch (_: Exception) {
                }
                playerBound = false
            }
            if (!destroying) {
                main.postDelayed({ if (!destroying) bindPlayer() }, REBIND_MS)
            }
        }

        override fun onNullBinding(name: ComponentName?) {
            player = null
            playerBound = false
            lastState = null
            updateNotification()
        }
    }

    override fun onCreate() {
        super.onCreate()
        running = true
        carPrefs = CarPrefs(this)
        windowManager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        createChannel()
        bindPlayer()

        // The settings screen writes the same "car" file; a trigger/opacity
        // change must take effect without restarting the service.
        carSp = getSharedPreferences("car", Context.MODE_PRIVATE).also {
            it.registerOnSharedPreferenceChangeListener(prefListener)
        }
        syncTrigger()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        when (intent?.action) {
            ACTION_STOP -> {
                // A startForegroundService caller owes Android a
                // startForeground even when the first thing it does is stop.
                if (!foregrounded) goForeground()
                stopEverything()
                return START_NOT_STICKY
            }
            ACTION_REFRESH -> {
                if (!goForeground()) {
                    stopSelf()
                    return START_NOT_STICKY
                }
                suppressOverlay = false
                syncTrigger()
                // Returning from the car settings: re-read the Bluetooth sink,
                // the foreground GPS package, the speed permission.
                trigger?.refresh()
                if (!overlayAttached && !dismissed &&
                    (manual || triggerVisible || trigger?.visible == true)
                ) attachOverlay()
                overlay?.refreshStyle()
                updateOverlayParams()
                if (overlayAttached) renderOverlay()
                updateNotification()
                if (!carPrefs.gpsTrigger && !carPrefs.bluetoothTrigger && !manual) {
                    stopEverything()
                }
                return START_NOT_STICKY
            }
            else -> {
                manual = if (intent == null) false else intent.getBooleanExtra(EXTRA_MANUAL, true)
                dismissed = false
                suppressOverlay = false
                if (!goForeground()) {
                    stopSelf()
                    return START_NOT_STICKY
                }
                syncTrigger()
                bindPlayer()
                if (manual) attachOverlay()
                updateNotification()
                return START_NOT_STICKY
            }
        }
    }

    /** Started-only service: the car UI talks to it through its own actions. */
    override fun onBind(intent: Intent?): IBinder? = null

    // -- window -----------------------------------------------------------------

    private fun buildParams(size: CarSize): WindowManager.LayoutParams {
        val ultra = size == CarSize.ULTRA
        val params = WindowManager.LayoutParams(
            if (ultra) WindowManager.LayoutParams.WRAP_CONTENT
            else WindowManager.LayoutParams.MATCH_PARENT,
            WindowManager.LayoutParams.WRAP_CONTENT,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT
        )
        params.gravity =
            if (ultra) Gravity.BOTTOM or Gravity.END else Gravity.BOTTOM or Gravity.START
        params.x = 0
        params.y = if (ultra) 0 else dp(BOTTOM_DP)
        // Android 12's untrusted-touch rule is evaluated on the WINDOW's alpha:
        // keeping it inside 40–80 % is what lets the map behind stay usable.
        params.alpha = carPrefs.overlayAlpha.coerceIn(0.40f, 0.80f)
        if (Build.VERSION.SDK_INT >= 31) {
            try {
                if (windowManager.isCrossWindowBlurEnabled) {
                    params.flags = params.flags or
                        WindowManager.LayoutParams.FLAG_BLUR_BEHIND
                    params.blurBehindRadius = dp(BLUR_DP)
                }
            } catch (_: Exception) {
                // A device that reports blur but refuses the radius: the card
                // stays translucent, just not frosted.
            }
        }
        return params
    }

    private fun attachOverlay() {
        if (overlayAttached) return
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { attachOverlay() }
            return
        }
        if (!CarPermissions.hasOverlay(this)) {
            if (!overlayBlockedToastShown) {
                overlayBlockedToastShown = true
                toast(R.string.car_overlay_blocked)
            }
            updateNotification()
            return
        }
        val root = CarOverlayRoot(this, this, carPrefs)
        overlay = root
        root.refreshStyle()
        root.setSize(desiredSize)
        root.setScreen(desiredScreen)
        renderOverlay()
        try {
            windowManager.addView(root, buildParams(desiredSize))
            overlayAttached = true
            overlayBlockedToastShown = false
            startTicker()
            updateNotification()
        } catch (_: Exception) {
            // Permission revoked between the check and addView, a stale token,
            // or an OEM refusing the window: no overlay, no crash. The
            // notification carries the Réglages action.
            overlay = null
            overlayAttached = false
            stopTicker()
            updateNotification()
        }
    }

    private fun detachOverlay() {
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { detachOverlay() }
            return
        }
        stopTicker()
        val root = overlay
        overlay = null
        if (overlayAttached && root != null) {
            try {
                windowManager.removeViewImmediate(root)
            } catch (_: Exception) {
            }
        }
        overlayAttached = false
        updateNotification()
    }

    private fun updateOverlayParams() {
        if (!overlayAttached) return
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { updateOverlayParams() }
            return
        }
        val root = overlay ?: return
        try {
            windowManager.updateViewLayout(root, buildParams(desiredSize))
        } catch (_: Exception) {
            detachOverlay()
        }
    }

    // -- trigger / state ---------------------------------------------------------

    private fun syncTrigger() {
        if (!::carPrefs.isInitialized) return
        val gps = carPrefs.gpsTrigger
        val bluetooth = carPrefs.bluetoothTrigger
        val packages = carPrefs.gpsPackages
        if (gps == lastGps && bluetooth == lastBt && packages == lastPackages &&
            (trigger != null || (!gps && !bluetooth))
        ) return

        lastGps = gps
        lastBt = bluetooth
        lastPackages = packages
        trigger?.stop()
        trigger = null
        if (gps || bluetooth) {
            lastVisible = false
            trigger = CarTrigger(this, carPrefs, triggerListener).also { it.start() }
        } else {
            triggerVisible = false
            lastVisible = false
            if (!manual) detachOverlay()
        }
    }

    private fun handleDrive(visible: Boolean, moving: Boolean, speedKmh: Double) {
        this.moving = moving
        this.speed = if (speedKmh.isFinite()) speedKmh.coerceIn(0.0, MAX_SPEED) else 0.0
        val becameHidden = !visible && lastVisible
        lastVisible = visible
        if (visible) {
            // A new arrival clears a manual close; a repeated "still visible"
            // tick does not reopen what the person deliberately closed.
            if (!triggerVisible) dismissed = false
            triggerVisible = true
            if (!suppressOverlay && !dismissed && !overlayAttached) attachOverlay()
        } else if (becameHidden) {
            triggerVisible = false
            passenger = false
            if (!manual) detachOverlay()
        }
        if (overlayAttached) renderOverlay()
        updateNotification()
    }

    private fun onPlayerState(state: PlayerService.State) {
        lastState = state
        if (overlayAttached) renderOverlay()
        updateNotification()
    }

    private fun renderOverlay() {
        val root = overlay ?: return
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { renderOverlay() }
            return
        }
        root.render(now(), moving, speed, passenger)
    }

    private fun startTicker() {
        main.removeCallbacks(ticker)
        main.postDelayed(ticker, TICK_MS)
    }

    private fun stopTicker() {
        main.removeCallbacks(ticker)
    }

    private fun bindPlayer() {
        if (playerBound) return
        try {
            playerBound = bindService(
                Intent(this, PlayerService::class.java),
                connection,
                Context.BIND_AUTO_CREATE
            )
        } catch (_: Exception) {
            playerBound = false
        }
    }

    // -- notification ------------------------------------------------------------

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java) ?: return
        val channel = NotificationChannel(
            CHANNEL_ID,
            getString(R.string.car_channel),
            NotificationManager.IMPORTANCE_LOW
        ).apply {
            setShowBadge(false)
            setSound(null, null)
        }
        nm.createNotificationChannel(channel)
    }

    /**
     * The foreground type this service wants right now.
     *
     * `specialUse` is what lets the overlay stay up over the navigation app;
     * `location` is what keeps the speed updates coming once Android 10+ sees
     * the activity behind Maps (without it, a location update is refused or
     * the location indicator disappears). Android 14 needs both permissions
     * and accepts the union; below 34 only `location` exists, and only when
     * the runtime grant is actually there.
     */
    private fun foregroundTypes(): Int {
        if (Build.VERSION.SDK_INT >= 34) {
            var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            if (CarPermissions.hasLocation(this)) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
            }
            if (CarPermissions.hasMicrophone(this)) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            }
            return types
        }
        if (Build.VERSION.SDK_INT >= 30) {
            var types = 0
            if (CarPermissions.hasLocation(this)) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
            }
            if (CarPermissions.hasMicrophone(this)) {
                types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            }
            return types
        }
        if (Build.VERSION.SDK_INT >= 29) {
            return if (CarPermissions.hasLocation(this)) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION
            } else 0
        }
        return 0
    }

    private fun startForegroundWith(type: Int): Boolean = try {
        ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), type)
        foregroundType = type
        true
    } catch (_: Exception) {
        false
    }

    private fun goForeground(): Boolean {
        val types = foregroundTypes()
        if (foregrounded) {
            // A permission was granted (or revoked) while the service ran:
            // update the type, but never tear the overlay down if the wider
            // type is refused.
            if (types == foregroundType) return true
            if (startForegroundWith(types)) return true
            return true
        }
        if (startForegroundWith(types)) {
            foregrounded = true
            notifKey = notificationKey()
            return true
        }
        // A location grant from a background start can be refused even with
        // the permission; the overlay itself does not depend on it.
        val fallback =
            if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0
        if (fallback != types && startForegroundWith(fallback)) {
            foregrounded = true
            notifKey = notificationKey()
            return true
        }
        // FGS-start restriction (Android 12+) or a revoked FGS permission:
        // the service does not park a half-started notification.
        foregrounded = false
        return false
    }

    private fun notificationKey(): String {
        val s = lastState
        return listOf(
            overlayAttached,
            CarPermissions.hasOverlay(this),
            if (s?.active == true) "${s.title}|${s.artist}" else "",
            s?.playing == true
        ).joinToString("|")
    }

    private fun updateNotification() {
        if (!foregrounded || destroying) return
        val key = notificationKey()
        if (key == notifKey) return
        notifKey = key
        try {
            NotificationManagerCompat.from(this)
                .notify(NOTIFICATION_ID, buildNotification())
        } catch (_: SecurityException) {
            // Notifications denied: the overlay and the tile still work.
        }
    }

    private fun buildNotification(): Notification {
        val s = lastState
        val text = when {
            !CarPermissions.hasOverlay(this) -> getString(R.string.car_notification_permission)
            s != null && s.active && s.title.isNotBlank() -> listOf(s.title, s.artist)
                .filter { it.isNotBlank() }
                .joinToString(" · ")
            overlayAttached -> getString(R.string.car_notification_active)
            else -> getString(R.string.car_notification_text)
        }

        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        val contentIntent = PendingIntent.getActivity(
            this, 10,
            Intent(this, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            flags
        )
        val settingsIntent = PendingIntent.getActivity(
            this, 11, Intent(this, CarSettingsActivity::class.java), flags
        )
        val stopIntent = PendingIntent.getService(
            this, 12,
            Intent(this, CarModeService::class.java).setAction(ACTION_STOP),
            flags
        )

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_car)
            .setContentTitle(getString(R.string.car_notification_title))
            .setContentText(text)
            .setContentIntent(contentIntent)
            .setOngoing(true)
            .setAutoCancel(false)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setShowWhen(false)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .addAction(
                R.drawable.ic_shortcut_settings,
                getString(R.string.car_notification_settings),
                settingsIntent
            )
            .addAction(
                R.drawable.ic_car_stop,
                getString(R.string.car_notification_stop),
                stopIntent
            )
            .build()
    }

    // -- CarHost -----------------------------------------------------------------

    override val prefs: CarPrefs
        get() = carPrefs

    override fun now(): CarNow? = lastState?.takeIf { it.active }?.let { s ->
        CarNow(
            title = s.title,
            artist = s.artist,
            album = s.album,
            cover = s.cover,
            playing = s.playing,
            position = s.position,
            duration = s.duration,
            favorite = null, // the page owns the favourites
        )
    }

    // The trigger is the one source of truth while it exists; the last
    // callback's values are the fallback when no trigger is configured
    // (manual tile start), where "unknown" is safely "moving".
    override fun isDriving(): Boolean = trigger?.moving ?: moving

    override fun speedKmh(): Double = trigger?.speedKmh ?: speed

    override fun isPassenger(): Boolean = passenger

    override fun setPassenger(value: Boolean) {
        runMain {
            passenger = value
            renderOverlay()
        }
    }

    override fun togglePlay() {
        val service = player
        if (service == null) {
            toast(R.string.car_no_player)
            return
        }
        service.transport("toggle")
    }

    override fun next() {
        val service = player
        if (service == null) {
            toast(R.string.car_no_player)
            return
        }
        service.transport("next")
    }

    override fun previous() {
        val service = player
        if (service == null) {
            toast(R.string.car_no_player)
            return
        }
        service.transport("prev")
    }

    override fun seekTo(seconds: Double) {
        if (!seconds.isFinite() || seconds < 0) return
        val service = player
        if (service == null) {
            toast(R.string.car_no_player)
            return
        }
        service.transport("seek", seconds.coerceIn(0.0, MAX_SEEK))
    }

    override fun toggleFavorite() {
        runMain {
            val sent = player?.evalJs("window.__nsCarFav && window.__nsCarFav()") == true
            if (!sent) toast(R.string.car_favorites_unavailable)
        }
    }

    override fun setSize(size: CarSize) {
        runMain {
            desiredSize = size
            if (size == CarSize.ULTRA) desiredScreen = CarScreen.PLAYER
            overlay?.setSize(size)
            if (overlayAttached) updateOverlayParams()
        }
    }

    override fun setScreen(screen: CarScreen) {
        runMain {
            desiredScreen = screen
            overlay?.setScreen(screen)
        }
    }

    override fun closeOverlay() {
        runMain {
            manual = false
            // Keep triggerVisible/lastVisible as they are: the next callback
            // is a repeat of "still visible", which must not reopen what was
            // just closed. A real leave-and-return resets `dismissed`.
            dismissed = true
            suppressOverlay = false
            detachOverlay()
        }
    }

    override fun launch(kind: String, id: String, title: String, payload: String?) {
        runMain {
            if (kind !in LAUNCH_KINDS || id.length > MAX_LAUNCH_ID ||
                title.length > MAX_LAUNCH_TITLE
            ) {
                toast(R.string.car_no_player)
                return@runMain
            }
            val payloadJs = payload
                ?.takeIf { it.isNotBlank() && it.length <= MAX_LAUNCH_PAYLOAD }
                ?.let { JSONObject.quote(it) }
                ?: "null"
            // The hook is the SPA's (lib/car.js); quoting every argument keeps
            // a title or a payload from becoming code in the WebView.
            val js = "window.__nsCarPlay && window.__nsCarPlay(" +
                "${JSONObject.quote(kind)}, ${JSONObject.quote(id)}, " +
                "${JSONObject.quote(title)}, $payloadJs)"
            if (player?.evalJs(js) != true) {
                toast(R.string.car_no_player)
                return@runMain
            }
            desiredScreen = CarScreen.PLAYER
            overlay?.setScreen(CarScreen.PLAYER)
            toast(getString(R.string.car_launched, title.ifBlank { id }.ifBlank {
                getString(R.string.car_mode)
            }))
        }
    }

    override fun openSettings() {
        runMain {
            // A full-screen activity under an overlay would be unusable;
            // MainActivity's next onResume sends ACTION_REFRESH and reattaches
            // it if a trigger (or the manual start) still wants it.
            suppressOverlay = true
            dismissed = false
            detachOverlay()
            try {
                startActivity(
                    Intent(this, CarSettingsActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            } catch (_: Exception) {
                suppressOverlay = false
                toast(R.string.car_settings_missing)
            }
        }
    }

    override fun toast(message: String) {
        runMain {
            val root = overlay
            if (root != null && overlayAttached) {
                root.showToast(message)
            } else {
                try {
                    Toast.makeText(this, message, Toast.LENGTH_SHORT).show()
                } catch (_: Exception) {
                }
            }
        }
    }

    // -- lifecycle ---------------------------------------------------------------

    private fun onCarPrefsChanged() {
        if (destroying || !::carPrefs.isInitialized) return
        overlay?.refreshStyle()
        updateOverlayParams()
        syncTrigger()
        trigger?.refresh()
        if (!carPrefs.gpsTrigger && !carPrefs.bluetoothTrigger && !manual) {
            // Nothing left to watch, nobody asked for the overlay by hand.
            stopEverything()
            return
        }
        if (overlayAttached) renderOverlay()
        updateNotification()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (!overlayAttached) return
        overlay?.refreshStyle()
        updateOverlayParams()
        renderOverlay()
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // The WebView (and with it the queue) went with the task; an overlay
        // over the map with a dead player is worse than no overlay.
        stopEverything()
        super.onTaskRemoved(rootIntent)
    }

    private fun stopEverything() {
        manual = false
        triggerVisible = false
        dismissed = true
        detachOverlay()
        trigger?.stop()
        trigger = null
        player?.onState(null)
        if (playerBound) {
            try {
                unbindService(connection)
            } catch (_: Exception) {
            }
            playerBound = false
        }
        player = null
        if (foregrounded) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregrounded = false
            foregroundType = -1
        }
        stopSelf()
    }

    override fun onDestroy() {
        destroying = true
        running = false
        main.removeCallbacksAndMessages(null)
        trigger?.stop()
        trigger = null
        detachOverlay()
        player?.onState(null)
        if (playerBound) {
            try {
                unbindService(connection)
            } catch (_: Exception) {
            }
            playerBound = false
        }
        player = null
        carSp?.unregisterOnSharedPreferenceChangeListener(prefListener)
        carSp = null
        if (foregrounded) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregrounded = false
            foregroundType = -1
        }
        super.onDestroy()
    }

    // -- helpers -----------------------------------------------------------------

    private fun runMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block()
        else main.post(block)
    }

    private fun toast(resId: Int) = toast(getString(resId))

    private fun dp(value: Int): Int = CarStyle.dp(this, value.toFloat())
}
