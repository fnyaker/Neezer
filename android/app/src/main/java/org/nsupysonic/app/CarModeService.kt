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
import android.view.WindowInsets
import android.view.WindowManager
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import org.json.JSONObject
import kotlin.math.abs

/**
 * The car mode: a foreground service that owns the overlay window and the
 * PlayerService binding, never the audio.
 *
 * The page still owns the queue and the sound (PlayerService/MediaSession are
 * untouched); this service is a remote control that also happens to draw on
 * top of the navigation app. It is a `specialUse|location|microphone`
 * foreground service because the overlay must outlive the activity while a GPS
 * app is in front — that is exactly the use Android has no narrower type for;
 * the two permission-guarded bits are added only while their grant exists.
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

        /**
         * The only door to the car settings: the notification action, the SPA
         * bridge and the overlay's ghost tile all end up here, so the service
         * can detach the overlay before the full-screen activity appears.
         */
        private const val ACTION_OPEN_SETTINGS = "org.nsupysonic.app.action.CAR_OPEN_SETTINGS"
        private const val EXTRA_MANUAL = "manual"

        const val NOTIFICATION_ID = 2

        /** A start the OS refused from the background: one tap is the way back. */
        private const val BLOCKED_NOTIFICATION_ID = 3
        private const val CHANNEL_ID = "car"

        private const val MAX_LAUNCH_ID = 200
        private const val MAX_LAUNCH_TITLE = 200
        // Quoted inside the expression below, so keep it under PlayerService.JS_MAX.
        private const val MAX_LAUNCH_PAYLOAD = 12 * 1024

        private val LAUNCH_KINDS = setOf("flow", "favorites", "playlist", "album", "artist", "mix", "track")

        private const val TICK_MS = 1_000L
        private const val REBIND_MS = 1_000L
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
            if (startForeground(context, intent)) return
            // A manual start from the tile/activity is normally allowed; if an
            // OEM refused it anyway, the notification is the explicit door.
        }

        /**
         * Start a foreground service, and report a refused background start
         * instead of throwing into a BroadcastReceiver.
         *
         * Android 12+ throws `ForegroundServiceStartNotAllowedException` when
         * the app is in the background (developer.android.com, "Restrictions on
         * starting a foreground service from the background"). The exemptions
         * this app relies on are the BOOT_COMPLETED / MY_PACKAGE_REPLACED
         * broadcasts and the SYSTEM_ALERT_WINDOW grant it already requires. If
         * an OEM refuses anyway, a plain `startService` is tried (allowed for
         * the lifetime of a broadcast receiver) and the notification is the
         * last, explicit fallback.
         */
        private fun startForeground(context: Context, intent: Intent): Boolean {
            return try {
                ContextCompat.startForegroundService(context, intent)
                true
            } catch (e: Exception) {
                // Android 12+ throws ForegroundServiceStartNotAllowedException
                // for a start from the background; Android 14 adds the
                // while-in-use SecurityException for a location/microphone
                // type. Both are expected here and the notification is the
                // explicit way back.
                val retryable = isBackgroundStartRestriction(e) || e is SecurityException
                val plain = if (retryable) {
                    try {
                        context.startService(
                            Intent(context, CarModeService::class.java)
                                .setAction(ACTION_START)
                                .putExtra(EXTRA_MANUAL, false)
                        )
                        true
                    } catch (_: Exception) {
                        false
                    }
                } else {
                    false
                }
                if (!plain) notifyStartBlocked(context)
                plain
            }
        }

        /** The Android 12+ background-start refusal, without a hard class link. */
        private fun isBackgroundStartRestriction(e: Exception): Boolean =
            Build.VERSION.SDK_INT >= 31 &&
                e is android.app.ForegroundServiceStartNotAllowedException

        /** The car service's notification channel (created once, idempotent). */
        fun ensureChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java) ?: return
            val channel = NotificationChannel(
                CHANNEL_ID,
                context.getString(R.string.car_channel),
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                setShowBadge(false)
                setSound(null, null)
            }
            nm.createNotificationChannel(channel)
        }

        /**
         * The OS refused a background start: post the one notification the
         * person can tap to start the mode (a notification action is an
         * allowed foreground start), instead of a crash or a silent nothing.
         */
        private fun notifyStartBlocked(context: Context) {
            try {
                if (Build.VERSION.SDK_INT >= 33 &&
                    ContextCompat.checkSelfPermission(
                        context, android.Manifest.permission.POST_NOTIFICATIONS
                    ) != android.content.pm.PackageManager.PERMISSION_GRANTED
                ) return
                if (!NotificationManagerCompat.from(context).areNotificationsEnabled()) return
                ensureChannel(context)
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
                val open = PendingIntent.getActivity(
                    context, 10,
                    Intent(context, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                    flags
                )
                val start = PendingIntent.getForegroundService(
                    context, 14,
                    Intent(context, CarModeService::class.java)
                        .setAction(ACTION_START)
                        .putExtra(EXTRA_MANUAL, true),
                    flags
                )
                val notification = NotificationCompat.Builder(context, CHANNEL_ID)
                    .setSmallIcon(R.drawable.ic_car)
                    .setContentTitle(context.getString(R.string.car_start_blocked))
                    .setContentText(context.getString(R.string.car_start_blocked_text))
                    .setContentIntent(open)
                    .setOngoing(false)
                    .setAutoCancel(true)
                    .setOnlyAlertOnce(true)
                    .setCategory(NotificationCompat.CATEGORY_REMINDER)
                    .setPriority(NotificationCompat.PRIORITY_LOW)
                    .addAction(
                        R.drawable.ic_car,
                        context.getString(R.string.car_start_blocked_action),
                        start
                    )
                    .build()
                NotificationManagerCompat.from(context)
                    .notify(BLOCKED_NOTIFICATION_ID, notification)
            } catch (_: Exception) {
                // Notifications denied: the tile and the settings screen remain.
            }
        }

        /**
         * The car settings screen is going away. `openSettingsNow` detached the
         * card and suppressed it; this clears the suppression through the one
         * ACTION_REFRESH path, so returning to a navigation app reattaches
         * exactly once — including when the trigger had not seen a new event
         * (coming back inside the grace).
         */
        fun settingsClosed(context: Context) {
            if (!running) return
            try {
                context.startService(
                    Intent(context, CarModeService::class.java).setAction(ACTION_REFRESH)
                )
            } catch (_: Exception) {
                // The service died meanwhile: nothing is drawing.
            }
        }

        fun stop(context: Context) {
            try {
                context.stopService(Intent(context, CarModeService::class.java))
            } catch (_: Exception) {
            }
        }

        /**
         * The one door to the car settings, from anywhere: the notification's
         * Réglages action goes through the service, the SPA bridge and the
         * overlay's ghost tile come through [CarHost.openSettings], and both
         * end in the same place.
         *
         * Running: ask the service to detach the overlay and launch the
         * activity itself, so no card is left above the settings eating
         * touches. Not running: nothing is drawing, launch directly.
         *
         * Returns whether the launch was accepted: the SPA bridge hands that
         * boolean back to the page, so "nothing happened" is never a success.
         */
        fun openSettings(context: Context): Boolean {
            if (!running) {
                val opened = launchSettingsActivity(context)
                if (!opened) settingsMissingToast(context)
                return opened
            }
            return try {
                val started = context.startService(
                    Intent(context, CarModeService::class.java).setAction(ACTION_OPEN_SETTINGS)
                ) != null
                if (started) {
                    true
                } else {
                    val opened = launchSettingsActivity(context)
                    if (!opened) settingsMissingToast(context)
                    opened
                }
            } catch (_: Exception) {
                // The service died between isRunning() and here, or an OEM
                // refuses background starts: there is no overlay left to
                // detach, so opening the activity is still correct.
                val opened = launchSettingsActivity(context)
                if (!opened) settingsMissingToast(context)
                opened
            }
        }

        /** Start the full-screen car settings; false when no activity answers. */
        private fun launchSettingsActivity(context: Context): Boolean = try {
            context.startActivity(
                Intent(context, CarSettingsActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            )
            true
        } catch (_: Exception) {
            false
        }

        private fun settingsMissingToast(context: Context) {
            val show = Runnable {
                try {
                    Toast.makeText(context, R.string.car_settings_missing, Toast.LENGTH_SHORT).show()
                } catch (_: Exception) {
                }
            }
            // The SPA bridge may call this from its own thread; a Toast needs
            // a Looper, and the page already shows its own failure message.
            if (Looper.myLooper() == Looper.getMainLooper()) show.run()
            else Handler(Looper.getMainLooper()).post(show)
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
            return startForeground(context, intent)
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
    private var manual = false
    private var dismissed = false
    /** The full-screen car settings is on top: never draw over it. */
    private var suppressOverlay = false
    private var moving = true
    private var speed = 0.0
    private var passenger = false

    /** The navigation app in front, keyed position lookup (null = manual/BT). */
    private var gpsPackage: String? = null
    /** Where the window sits right now, in dp from the start edge and bottom. */
    private var liveSlot: CarSlot? = null
    /** A drag is in flight: a prefs echo must not reset the window. */
    private var dragging = false

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
        override fun onDrive(state: DriveState) {
            main.post { handleDrive(state) }
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
        ensureChannel(this)
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
        if (intent == null) {
            // START_STICKY brought us back after the OS killed the process:
            // no manual start, no stale dismissal. The trigger's seed decides
            // whether the card comes back, and the service keeps watching.
            manual = false
            dismissed = false
            suppressOverlay = false
            if (!goForeground()) {
                stopSelf()
                return START_NOT_STICKY
            }
            syncTrigger()
            bindPlayer()
            if (!carPrefs.gpsTrigger && !carPrefs.bluetoothTrigger) {
                // The process was killed after a manual start with no trigger
                // configured: nothing can restore that card, so do not park a
                // foreground notification that draws nothing.
                stopEverything()
                return START_NOT_STICKY
            }
            updateNotification()
            return START_STICKY
        }
        when (intent.action) {
            ACTION_STOP -> {
                // A startForegroundService caller owes Android a
                // startForeground even when the first thing it does is stop.
                if (!foregrounded) goForeground()
                stopEverything()
                return START_NOT_STICKY
            }
            ACTION_OPEN_SETTINGS -> {
                // The notification's Réglages action and the SPA bridge. The
                // service may have been restarted by this very intent: make it
                // foreground first (a notification action is an allowed start),
                // then detach and open. openSettingsNow is idempotent.
                val wasForegrounded = foregrounded
                if (!goForeground()) {
                    if (!launchSettingsActivity(this)) {
                        toast(R.string.car_settings_missing)
                    }
                    stopSelf()
                    return START_NOT_STICKY
                }
                openSettingsNow()
                // This intent created the service (stale notification): do not
                // park a foreground notification with no overlay behind the
                // settings screen. A running mode keeps its service.
                if (!wasForegrounded) stopEverything()
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
                // ONE attach, through the same rule the trigger uses: the card
                // is wanted, nothing suppresses it, and it is not already up.
                if (CarVisibility.shouldAttach(
                        visible = trigger?.visible == true,
                        suppressed = suppressOverlay,
                        dismissed = dismissed,
                        attached = overlayAttached,
                    )
                ) attachOverlay()
                overlay?.refreshStyle()
                updateOverlayParams()
                if (overlayAttached) renderOverlay()
                updateNotification()
                if (!carPrefs.gpsTrigger && !carPrefs.bluetoothTrigger && !manual) {
                    stopEverything()
                }
                return START_STICKY
            }
            else -> {
                manual = intent.getBooleanExtra(EXTRA_MANUAL, true)
                dismissed = false
                suppressOverlay = false
                if (!goForeground()) {
                    stopSelf()
                    return START_NOT_STICKY
                }
                syncTrigger()
                bindPlayer()
                if (manual) {
                    // The tile/settings start shows the card once. From the
                    // first drive state the visibility rule takes over, so a
                    // manual start can never park the card over a non-GPS app.
                    attachOverlay()
                }
                updateNotification()
                return START_STICKY
            }
        }
    }

    /** Started-only service: the car UI talks to it through its own actions. */
    override fun onBind(intent: Intent?): IBinder? = null

    // -- window -----------------------------------------------------------------

    private fun buildParams(size: CarSize, slot: CarSlot): WindowManager.LayoutParams {
        val ultra = size == CarSize.ULTRA
        // The window is EXACTLY the painted surface, never a full-width strip:
        // a transparent band inside the window still receives the touch and
        // eats the GPS app's taps underneath.
        val width = if (ultra) {
            dpF(CarPlacement.ULTRA_SIZE_DP)
        } else {
            (screenWidthPx() - dpF(CarPlacement.SIDE_DP * 2f)).coerceAtLeast(dp(1))
        }
        val height = if (ultra) {
            dpF(CarPlacement.ULTRA_SIZE_DP)
        } else {
            WindowManager.LayoutParams.WRAP_CONTENT
        }
        val params = WindowManager.LayoutParams(
            width,
            height,
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY,
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
                WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL,
            PixelFormat.TRANSLUCENT
        )
        // With BOTTOM, y is the distance from the bottom edge (AOSP
        // Gravity.apply: bottom - height - y); with END, x is the distance
        // from the right edge. The slot is stored from the START edge for every
        // size (so a rotation keeps it in the same place); applySlot converts
        // it for ULTRA's END gravity.
        params.gravity =
            if (ultra) Gravity.BOTTOM or Gravity.END else Gravity.BOTTOM or Gravity.START
        applySlot(params, size, slot)
        // The window is fully opaque and its content does the transparency:
        // the 40–80 % setting only fills the card (CarOverlayRoot), so text
        // and the lemon play button stay at 100 %. Android 12's untrusted-touch
        // rule caps passing a touch THROUGH a window at 0.8; this window now
        // covers exactly the card/button and handles its own touches, so it
        // passes nothing and alpha 1 does not block the map next to it.
        params.alpha = 1f
        // ULTRA is a round button with transparent corners: blurring a 76 dp
        // square behind it would paint a frosted square around the disc.
        if (!ultra && Build.VERSION.SDK_INT >= 31) {
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

    /** Put [slot] (dp, from the START edge / bottom) on [params]. */
    private fun applySlot(
        params: WindowManager.LayoutParams,
        size: CarSize,
        slot: CarSlot,
    ) {
        val startPx = dpF(slot.xDp)
        params.y = dpF(slot.bottomDp)
        params.x = if (size == CarSize.ULTRA) {
            (screenWidthPx() - startPx - dpF(CarPlacement.ULTRA_SIZE_DP)).coerceAtLeast(0)
        } else {
            startPx
        }
    }

    /**
     * Where the window should be for the current navigation app and size: the
     * person's saved slot, else the prototype's default, always clamped to the
     * safe area of the CURRENT display (rotation, split screen, a stored slot
     * from a wider phone).
     */
    private fun currentSlot(
        size: CarSize,
        windowHeightDp: Float = windowHeightDp(size),
    ): CarSlot {
        val base = carPrefs.position(gpsPackage, size)
            ?: CarPlacement.defaults(size, screenWidthDp())
        return CarPlacement.clamp(
            slot = base,
            windowWidthDp = CarPlacement.widthDp(size, screenWidthDp()),
            windowHeightDp = windowHeightDp,
            screenWidthDp = screenWidthDp(),
            screenHeightDp = screenHeightDp(),
            insets = screenInsetsDp(),
        )
    }

    /** The measured card height in dp, 0 before the first layout. */
    private fun windowHeightDp(size: CarSize): Float {
        if (size == CarSize.ULTRA) return CarPlacement.ULTRA_SIZE_DP
        val root = overlay ?: return 0f
        if (root.height <= 0) return 0f
        return root.height / density()
    }

    /**
     * Re-clamp after a layout or a size change: the card's measured height is
     * only known now, and a slot dragged high on a tall screen must not put the
     * card under the status bar on a shorter one. No-op while a drag runs (the
     * finger owns the position) and when the clamp changed nothing.
     */
    private fun enforceBounds() {
        if (!overlayAttached || dragging) return
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { enforceBounds() }
            return
        }
        val root = overlay ?: return
        val heightDp = windowHeightDp(desiredSize)
        if (heightDp <= 0f) return
        val slot = currentSlot(desiredSize, heightDp)
        val shown = liveSlot
        if (shown != null && abs(shown.xDp - slot.xDp) < 0.5f &&
            abs(shown.bottomDp - slot.bottomDp) < 0.5f
        ) return
        liveSlot = slot
        val lp = root.layoutParams as? WindowManager.LayoutParams ?: return
        applySlot(lp, desiredSize, slot)
        try {
            windowManager.updateViewLayout(root, lp)
        } catch (_: Exception) {
            detachOverlay()
        }
    }

    /**
     * Full display width in pixels. The overlay is laid out against the
     * display frame, not this service's (empty) window, so the card can use
     * "screen − 16 dp" even when no activity is alive.
     */
    private fun screenWidthPx(): Int {
        if (Build.VERSION.SDK_INT >= 30) {
            return windowManager.currentWindowMetrics.bounds.width()
        }
        val size = android.graphics.Point()
        @Suppress("DEPRECATION")
        windowManager.defaultDisplay.getRealSize(size)
        return size.x
    }

    private fun screenHeightPx(): Int {
        if (Build.VERSION.SDK_INT >= 30) {
            return windowManager.currentWindowMetrics.bounds.height()
        }
        val size = android.graphics.Point()
        @Suppress("DEPRECATION")
        windowManager.defaultDisplay.getRealSize(size)
        return size.y
    }

    private fun density(): Float =
        resources.displayMetrics.density.takeIf { it > 0f } ?: 1f

    private fun screenWidthDp(): Float = screenWidthPx() / density()

    private fun screenHeightDp(): Float = screenHeightPx() / density()

    /** Status bar, navigation bar and cutout, in dp; 0 below Android 11. */
    private fun screenInsetsDp(): CarInsets {
        if (Build.VERSION.SDK_INT < 30) return CarInsets()
        return try {
            val insets = windowManager.currentWindowMetrics.windowInsets.getInsets(
                WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout()
            )
            val d = density()
            CarInsets(insets.left / d, insets.top / d, insets.right / d, insets.bottom / d)
        } catch (_: Exception) {
            CarInsets()
        }
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
        val slot = currentSlot(desiredSize)
        liveSlot = slot
        try {
            windowManager.addView(root, buildParams(desiredSize, slot))
            overlayAttached = true
            overlayBlockedToastShown = false
            startTicker()
            root.post { enforceBounds() }
            updateNotification()
        } catch (_: Exception) {
            // Permission revoked between the check and addView, a stale token,
            // or an OEM refusing the window: no overlay, no crash. The
            // notification carries the Réglages action.
            overlay = null
            overlayAttached = false
            liveSlot = null
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
        liveSlot = null
        dragging = false
        updateNotification()
    }

    private fun updateOverlayParams() {
        if (!overlayAttached) return
        if (dragging) return
        if (Looper.myLooper() != Looper.getMainLooper()) {
            main.post { updateOverlayParams() }
            return
        }
        val root = overlay ?: return
        val slot = currentSlot(desiredSize)
        liveSlot = slot
        try {
            windowManager.updateViewLayout(root, buildParams(desiredSize, slot))
            root.post { enforceBounds() }
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
            trigger = CarTrigger(this, carPrefs, triggerListener).also { it.start() }
        } else {
            triggerVisible = false
            // No trigger watches anything: only an explicit manual start keeps
            // the card (and the service) alive.
            if (!manual) detachOverlay()
        }
    }

    private fun handleDrive(state: DriveState) {
        this.moving = state.moving
        this.speed = if (state.speedKmh.isFinite()) {
            state.speedKmh.coerceIn(0.0, MAX_SPEED)
        } else {
            0.0
        }
        val becameVisible = state.visible && !triggerVisible
        val packageChanged = state.gpsPackage != gpsPackage
        gpsPackage = state.gpsPackage
        triggerVisible = state.visible
        if (state.visible) {
            // A new arrival clears a manual close; a repeated "still visible"
            // tick does not reopen what the person deliberately closed.
            if (becameVisible) {
                dismissed = false
                // The settings screen was on top and a navigation app is in
                // front again: whatever Activity resumed (MainActivity sends
                // ACTION_REFRESH too), the overlay may come back now.
                suppressOverlay = false
            }
            if (!suppressOverlay && !dismissed) {
                if (!overlayAttached) {
                    attachOverlay()
                } else if (packageChanged) {
                    // Maps → Waze: the new app's own saved place, live.
                    updateOverlayParams()
                }
            }
        } else if (CarVisibility.shouldDetach(state.decided, state.visible, overlayAttached)) {
            // THE rule: no configured navigation app is in front, no card —
            // `manual` gave it one showing, not a permanent home.
            passenger = false
            detachOverlay()
        } else if (!state.decided && packageChanged && overlayAttached) {
            // The first state of a new trigger has no answer yet: leave the
            // manual card where it is until the seed comes back.
            updateOverlayParams()
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

    /**
     * The foreground type this service wants right now.
     *
     * `specialUse` is what lets the overlay stay up over the navigation app;
     * `location` is what keeps the speed updates coming once Android 10+ sees
     * the activity behind Maps (without it, a location update is refused or
     * the location indicator disappears); `microphone` is added only while the
     * mic grant exists, so the search keyboard's voice input is not cut off by
     * the overlay's own foreground service. Android 14 needs every permission
     * and accepts the union; below 34 only `location`/`microphone` exist, and
     * only when the runtime grant is actually there.
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
        val settingsIntent = PendingIntent.getService(
            this, 11,
            Intent(this, CarModeService::class.java).setAction(ACTION_OPEN_SETTINGS),
            flags
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
            val service = player ?: run {
                toast(R.string.car_favorites_unavailable)
                return@runMain
            }
            // __nsCarFav returns the new state, or is absent: only claim a
            // favourite changed when the page actually said so.
            service.evalJs("window.__nsCarFav && window.__nsCarFav()") { ok ->
                runMain { if (!ok) toast(R.string.car_favorites_unavailable) }
            }
        }
    }

    override fun setSize(size: CarSize) {
        runMain {
            desiredSize = size
            if (size == CarSize.ULTRA) desiredScreen = CarScreen.PLAYER
            overlay?.setSize(size)
            if (overlayAttached) {
                // A new size has its own saved slot (and its own measured
                // height): re-clamp after the layout, not against the old one.
                updateOverlayParams()
                overlay?.post { enforceBounds() }
            }
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
            // Keep triggerVisible as it is: the next callback is a repeat of
            // "still visible", which must not reopen what was just closed. A
            // real leave-and-return resets `dismissed`.
            dismissed = true
            suppressOverlay = false
            detachOverlay()
        }
    }

    /**
     * Live drag from the card's move handle: pixels since the last call. The
     * slot stays in dp (start edge / bottom), is clamped to the safe area
     * before every window update, and is persisted on [endOverlayMove] for
     * THIS navigation app and size only.
     */
    override fun moveOverlayBy(dxPx: Int, dyPx: Int) {
        runMain {
            if (!overlayAttached) return@runMain
            val root = overlay ?: return@runMain
            val lp = root.layoutParams as? WindowManager.LayoutParams ?: return@runMain
            val base = liveSlot ?: currentSlot(desiredSize)
            val d = density()
            dragging = true
            val heightDp = windowHeightDp(desiredSize)
            val moved = CarPlacement.clamp(
                slot = CarSlot(base.xDp + dxPx / d, base.bottomDp - dyPx / d),
                windowWidthDp = CarPlacement.widthDp(desiredSize, screenWidthDp()),
                windowHeightDp = heightDp,
                screenWidthDp = screenWidthDp(),
                screenHeightDp = screenHeightDp(),
                insets = screenInsetsDp(),
            )
            liveSlot = moved
            applySlot(lp, desiredSize, moved)
            try {
                windowManager.updateViewLayout(root, lp)
            } catch (_: Exception) {
                detachOverlay()
            }
        }
    }

    override fun endOverlayMove() {
        runMain {
            if (!dragging) return@runMain
            dragging = false
            val slot = liveSlot ?: return@runMain
            carPrefs.setPosition(gpsPackage, desiredSize, slot)
            // The preference listener re-applies it (and re-clamps after the
            // rotation/size change); write once, no second update here.
        }
    }

    override fun launch(kind: String, id: String, title: String, payload: String?) {
        runMain started@{
            if (kind !in LAUNCH_KINDS || id.length > MAX_LAUNCH_ID ||
                title.length > MAX_LAUNCH_TITLE
            ) {
                toast(R.string.car_no_player)
                return@started
            }
            val service = player ?: run {
                toast(R.string.car_no_player)
                return@started
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
            val label = title.ifBlank { id }.ifBlank { getString(R.string.car_mode) }
            // __nsCarPlay answers a boolean: the screen changes and the toast
            // appears only when the page really started something. On false (or
            // no hook at all) the overlay stays where it is and says why.
            var answered = false
            service.evalJs(js) { ok ->
                runMain answer@{
                    if (answered) return@answer
                    answered = true
                    if (!ok) {
                        toast(
                            if (kind == "favorites") R.string.car_favorites_launch_failed
                            else R.string.car_no_player
                        )
                        return@answer
                    }
                    desiredScreen = CarScreen.PLAYER
                    overlay?.setScreen(CarScreen.PLAYER)
                    // One toast, worded for what was launched: the playlist
                    // panel no longer adds its own "Playlist lancée".
                    //
                    // "favorites" is asynchronous on the page (it fetches
                    // /api/me/favorites before it has a queue), so `true` here
                    // only means "accepted, loading". The PAGE owns that
                    // wording — "Chargement des favoris…" then "Lecture des
                    // favoris", or the honest failure — and the service stays
                    // silent, so the person never gets "Playlist lancée" for a
                    // launch that had not happened yet (or for an empty list).
                    if (kind == "favorites") return@answer
                    toast(
                        if (kind == "playlist") {
                            getString(R.string.car_playlist_launched, label)
                        } else {
                            getString(R.string.car_launched, label)
                        }
                    )
                }
            }
        }
    }

    override fun openSettings() {
        runMain { openSettingsNow() }
    }

    /**
     * One implementation for every entry point (notification, bridge, ghost
     * tile): hide the card first, then open the full-screen settings. A
     * failure puts the overlay back under the trigger's control instead of
     * leaving the person with a blank tap.
     */
    private fun openSettingsNow() {
        suppressOverlay = true
        dismissed = false
        detachOverlay()
        if (!launchSettingsActivity(this)) {
            suppressOverlay = false
            toast(R.string.car_settings_missing)
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
        // New display bounds/insets: the stored slot is re-clamped (a card
        // dragged to the far right of a wide phone stays reachable).
        overlay?.refreshStyle()
        updateOverlayParams()
        overlay?.post { enforceBounds() }
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

    private fun dpF(value: Float): Int = CarStyle.dp(this, value)
}
