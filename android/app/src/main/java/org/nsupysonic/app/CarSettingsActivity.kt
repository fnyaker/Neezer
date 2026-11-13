package org.nsupysonic.app

import android.Manifest
import android.animation.ArgbEvaluator
import android.animation.ValueAnimator
import android.content.Context
import android.content.Intent
import android.content.res.ColorStateList
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.text.TextUtils
import android.util.AttributeSet
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.TextView
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlin.math.max
import kotlin.math.roundToInt

/**
 * Réglages → Mode voiture, full screen in the app: what triggers the overlay,
 * which apps count as navigation, the keyboard and its letter limit, the
 * pinned playlists, the card's position per navigation app and the bar's
 * opacity. The prototype's cards, plus the permissions the Android side
 * actually needs (shown only while one is missing) and the gesture inversion,
 * which had no other home.
 *
 * Nothing here confirms before acting: pinning, unpinning and moving a pin are
 * one tap and a large toast, never a dialog. Nothing here plays either — the
 * pins are [CarPrefs] data, read by [CarPlaylistsView] and the overlay service.
 *
 * The playlist catalogue is the server's `/api/me/playlists`, fetched natively
 * through [CarPlaylistSource] with the WebView's session cookie: the same
 * origin rule, pin and redirect discipline as every other native fetch, and no
 * new endpoint. The URL is built from the server root, so the SPA's `/app`
 * path can never end up in front of `/api`. Offline, the pinned rows already
 * in [CarPrefs] still paint, are announced as local, and can still be
 * un-pinned; the status line names the cause (session, offline, timeout, TLS,
 * server error, unexpected page) and offers one retry.
 */
class CarSettingsActivity : AppCompatActivity() {

    private lateinit var prefs: Prefs
    private lateinit var car: CarPrefs

    private lateinit var content: LinearLayout
    private lateinit var permissionsCard: LinearLayout
    private lateinit var pinsBox: LinearLayout
    private lateinit var pinsStatus: TextView
    private lateinit var pinCount: TextView
    private lateinit var positionsBox: LinearLayout
    private lateinit var gpsFlow: CarFlow
    private lateinit var layoutFlow: CarFlow
    private lateinit var letterFlow: CarFlow
    private lateinit var layoutHint: TextView

    private enum class Catalogue {
        IDLE,
        LOADING,
        READY,
        NO_SERVER,
        NO_SESSION,
        OFFLINE,
        TIMEOUT,
        TLS,
        SERVER,
        UNEXPECTED,
    }

    private class CatalogueRow(
        val id: String,
        val title: String,
        val nbTracks: Int,
        val stale: Boolean,
        val cover: String?,
    )

    private class GpsApp(val label: String, val packages: Set<String>)

    private var catalogue = Catalogue.IDLE
    private var serverItems: List<CarPlaylistSource.Item> = emptyList()
    private var loadSeq = 0
    private var lastAttemptAt = 0L
    private var lastSuccessAt = 0L
    private val permissionRows = mutableListOf<Pair<View, () -> Boolean>>()

    // The Android runtime permissions (location for the speed, the microphone
    // for the voice search, Bluetooth to react to the car). The overlay and
    // the usage access are special grants: they open their own settings page.
    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { result ->
            if (result.values.any { !it }) toast("Autorisation refusée — le mode voiture s'en passera")
            refreshPermissions()
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        prefs = Prefs(this)
        car = CarPrefs(this)
        // The whole screen is Sillon's ink; the status and navigation bars go
        // with it, with light icons (the default on a dark theme).
        window.statusBarColor = CarStyle.BG
        window.navigationBarColor = CarStyle.BG
        window.decorView.systemUiVisibility = 0
        title = "Mode voiture"

        val scroll = ScrollView(this).apply {
            isFillViewport = true
            isVerticalScrollBarEnabled = false
            setBackgroundColor(CarStyle.BG)
        }
        content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(d(16), d(16), d(16), d(40))
        }
        scroll.addView(
            content,
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        )
        setContentView(scroll)

        build()
        refreshPermissions()
        // The catalogue is read from onResume: onCreate is always followed by
        // one, and that is also the hook that must retry after a failure or
        // after a minute away.
    }

    override fun onResume() {
        super.onResume()
        // Back from the overlay/usage-access settings, and pins that may have
        // changed in the playlist page while this screen was behind it.
        refreshPermissions()
        if (::pinsBox.isInitialized) {
            seedFromCache()
            renderPins()
            refreshCatalogue()
        }
    }

    override fun onPause() {
        super.onPause()
        // Leaving the settings (back to the navigation app): the service
        // detached and suppressed the card to show this screen. Clear the
        // suppression through the ONE refresh path so returning to a
        // navigation app reattaches exactly once — including a return inside
        // the transient grace, where no new foreground event was needed. A
        // permission screen on top (not finishing) keeps the card suppressed.
        if (isFinishing) CarModeService.settingsClosed(this)
    }

    /**
     * The overlay's own read is process-wide: reusing it here paints the
     * covers and counts without a request. It is only a minute old, and
     * [refreshCatalogue] re-reads as soon as it is not.
     */
    private fun seedFromCache() {
        if (catalogue != Catalogue.IDLE || serverItems.isNotEmpty()) return
        val cached = CarPlaylistSource.fresh(SystemClock.elapsedRealtime()) ?: return
        serverItems = cached.items
        lastSuccessAt = cached.at
        catalogue = Catalogue.READY
    }

    override fun onDestroy() {
        loadSeq++ // an answer in flight belongs to a screen that is gone
        super.onDestroy()
    }

    // -- layout --------------------------------------------------------------

    private fun build() {
        content.addView(
            header(),
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            ).apply { bottomMargin = d(14) }
        )

        addCard(triggerCard())
        addCard(buildPermissionsCard())
        addCard(positionCard())
        addCard(keyboardCard())
        addCard(pinsCard())
        addCard(opacityCard())
    }

    private fun header(): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        val back = iconButton(44, CarStyle.S1, CarStyle.LN, 1f, "chevronLeft", 20, CarStyle.TX, "Retour") {
            finish()
        }
        row.addView(back, LinearLayout.LayoutParams(d(44), d(44)))

        val titles = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val kicker = CarStyle.monoText(this, "RÉGLAGES", 11f, CarStyle.DIM).apply {
            letterSpacing = 0.08f
        }
        titles.addView(kicker)
        val title = CarStyle.text(this, "Mode voiture", 30f, CarStyle.TX, display = true).apply {
            typeface = Typeface.create(CarType.display(this@CarSettingsActivity), Typeface.BOLD)
            letterSpacing = -0.03f
        }
        titles.addView(title, topWrap(2))
        row.addView(
            titles,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                leftMargin = d(12)
            }
        )
        return row
    }

    // -- Déclenchement -------------------------------------------------------

    private fun triggerCard(): LinearLayout {
        val card = newCard()
        addStack(card, cardTitle("Déclenchement"))

        addStack(
            card,
            toggleRow(
                "Se placer sur l'app GPS au premier plan",
                "Le mode apparaît quand une app GPS passe devant, et disparaît avec elle.",
                car.gpsTrigger,
            ) { car.gpsTrigger = it }
        )

        addStack(card, monoLabel("APPS GPS SURVEILLÉES"), 14)
        gpsFlow = CarFlow(this).apply { gap = d(8) }
        renderGpsApps()
        addStack(card, gpsFlow, 8)

        addStack(
            card,
            toggleRow(
                "Aussi au Bluetooth de la voiture",
                "Sert quand le déclenchement GPS est désactivé (ou sans accès aux statistiques d'usage) : le mode apparaît avec le Bluetooth de la voiture.",
                car.bluetoothTrigger,
            ) { car.bluetoothTrigger = it },
            14
        )
        return card
    }

    private fun renderGpsApps() {
        gpsFlow.removeAllViews()
        val selected = car.gpsPackages
        for (app in GPS_APPS) {
            val on = app.packages.any { it in selected }
            gpsFlow.addView(chip(app.label, on) {
                val next = selected.toMutableSet()
                if (on) next.removeAll(app.packages) else next.addAll(app.packages)
                car.gpsPackages = next
                renderGpsApps()
            })
        }
    }

    // -- Position ------------------------------------------------------------

    /**
     * Where the card was dragged, per navigation app and per size. The card is
     * moved from the overlay itself (the FULL header's move handle, or a long
     * press + drag on the ULTRA disc); this screen only explains it and forgets
     * one app or all of them — no dialog, one tap and a toast.
     */
    private fun positionCard(): LinearLayout {
        val card = newCard()
        addStack(card, cardTitle("Position"))
        val hint = CarStyle.text(
            this,
            "Fais glisser la carte en mode Grand pour la placer ; elle est mémorisée " +
                "pour chaque app GPS. En mode Ultra, appui long puis glisse sur le rond.",
            13f,
            CarStyle.DIM,
        ).apply { setLineSpacing(0f, 1.4f) }
        addStack(card, hint, 6)

        positionsBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        addStack(card, positionsBox, 12)
        renderPositions()
        return card
    }

    private fun renderPositions() {
        positionsBox.removeAllViews()
        positionsBox.addView(
            chip("Réinitialiser la position", false) {
                car.clearPositions()
                toast(getString(R.string.car_position_reset))
                renderPositions()
            },
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                d(40)
            )
        )
        val packages = car.positionedPackages()
        if (packages.isEmpty()) {
            positionsBox.addView(
                CarStyle.text(
                    this,
                    "Aucune position enregistrée — la carte reprend sa place par défaut.",
                    13f,
                    CarStyle.DIM,
                ),
                topWrap(10)
            )
            return
        }
        for (pkg in packages.sortedBy { gpsLabel(it) }) {
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
            }
            val name = gpsLabel(pkg)
            val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
            column.addView(CarStyle.text(this, name, 15f, CarStyle.TX))
            if (name != pkg) {
                column.addView(CarStyle.monoText(this, pkg, 11f, CarStyle.DIM), topWrap(2))
            }
            row.addView(column, weight())
            row.addView(
                chip("Réinitialiser", false) {
                    car.clearPosition(pkg)
                    toast(getString(R.string.car_position_reset_app, name))
                    renderPositions()
                },
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    d(40)
                ).apply { leftMargin = d(12) }
            )
            positionsBox.addView(row, topWrap(8))
        }
    }

    private fun gpsLabel(pkg: String): String =
        GPS_APPS.firstOrNull { pkg in it.packages }?.label ?: pkg

    // -- Autorisations -------------------------------------------------------

    private fun buildPermissionsCard(): LinearLayout {
        permissionsCard = newCard()

        fun spec(
            label: String,
            hint: String,
            granted: () -> Boolean,
            ask: () -> Unit,
        ) {
            val row = permissionRow(label, hint, ask)
            permissionRows.add(row to granted)
            addStack(permissionsCard, row)
        }

        addStack(permissionsCard, cardTitle("Autorisations"))
        spec(
            "Afficher par-dessus les autres apps",
            "Pour que la barre apparaisse sans quitter le GPS.",
            { CarPermissions.hasOverlay(this) },
        ) { openPermissionSettings(CarPermissions.overlayIntent(this)) }
        spec(
            "Accès aux statistiques d'usage",
            "Pour détecter l'app GPS qui passe au premier plan.",
            { CarPermissions.hasUsageStats(this) },
        ) { openPermissionSettings(CarPermissions.usageIntent()) }
        spec(
            "Localisation",
            "Pour savoir si tu roules (la vitesse règle le clavier).",
            { CarPermissions.hasLocation(this) },
        ) { askPermissions(CarPermissions.locationPermissions) }
        spec(
            "Bluetooth",
            "Pour réagir à la connexion de la voiture.",
            { CarPermissions.hasBluetoothConnect(this) },
        ) {
            if (Build.VERSION.SDK_INT >= 31)
                askPermissions(arrayOf(Manifest.permission.BLUETOOTH_CONNECT))
        }
        spec(
            "Microphone",
            "Pour la recherche vocale en roulant.",
            { CarPermissions.hasMicrophone(this) },
        ) { askPermissions(arrayOf(Manifest.permission.RECORD_AUDIO)) }
        return permissionsCard
    }

    private fun permissionRow(label: String, hint: String, ask: () -> Unit): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        row.addView(
            textBlock(label, hint),
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        )
        row.addView(
            chip("Autoriser", true) { ask() },
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                d(40)
            ).apply { leftMargin = d(12) }
        )
        return row
    }

    private fun refreshPermissions() {
        var missing = false
        for ((row, granted) in permissionRows) {
            val ok = try {
                granted()
            } catch (_: Exception) {
                false
            }
            row.visibility = if (ok) View.GONE else View.VISIBLE
            if (!ok) missing = true
        }
        if (::permissionsCard.isInitialized)
            permissionsCard.visibility = if (missing) View.VISIBLE else View.GONE
    }

    private fun askPermissions(permissions: Array<String>) {
        val missing = permissions.filter {
            ContextCompat.checkSelfPermission(this, it) != android.content.pm.PackageManager.PERMISSION_GRANTED
        }
        if (missing.isEmpty()) {
            refreshPermissions()
            return
        }
        try {
            permissionLauncher.launch(missing.toTypedArray())
        } catch (_: Exception) {
            toast("Demande d'autorisation impossible")
            refreshPermissions()
        }
    }

    private fun openPermissionSettings(intent: Intent) {
        try {
            startActivity(intent)
        } catch (_: Exception) {
            toast("Réglage d'autorisation introuvable sur cet appareil")
        }
    }

    // -- Clavier -------------------------------------------------------------

    private fun keyboardCard(): LinearLayout {
        val card = newCard()
        addStack(card, cardTitle("Clavier"))

        addStack(card, monoLabel("DISPOSITION DES TOUCHES"), 14)
        layoutFlow = CarFlow(this).apply { gap = d(8) }
        renderLayouts()
        addStack(card, layoutFlow, 8)

        layoutHint = CarStyle.text(this, "", 13f, CarStyle.DIM).apply {
            setLineSpacing(0f, 1.4f)
        }
        addStack(card, layoutHint, 10)
        updateLayoutHint()

        addStack(card, monoLabel("LETTRES MAX EN ROULANT"), 14)
        letterFlow = CarFlow(this).apply { gap = d(8) }
        renderLetters()
        addStack(card, letterFlow, 8)

        addStack(card, divider(), 16)
        addStack(card, monoLabel("GESTES"), 14)
        addStack(
            card,
            toggleRow(
                "Inverser gauche et droite",
                "Pour les habitudes RTL : le glissé gauche passe au titre suivant.",
                car.invertSwipe,
            ) { car.invertSwipe = it },
            8
        )
        return card
    }

    private fun renderLayouts() {
        layoutFlow.removeAllViews()
        for ((id, label) in LAYOUTS) {
            layoutFlow.addView(chip(label, car.keyboardLayout == id) {
                car.keyboardLayout = id
                renderLayouts()
                updateLayoutHint()
            })
        }
    }

    private fun updateLayoutHint() {
        layoutHint.text = when (car.keyboardLayout) {
            "auto" -> "Suit le clavier du téléphone : AZERTY, QWERTY ou QWERTZ selon ta langue. Si tu changes de langue, le mode voiture suit."
            "abc" -> "Ordre alphabétique 7 × 4 : touches plus larges, plus facile à retrouver sans regarder."
            else -> "Disposition fixe, quelle que soit la langue du clavier du téléphone. Touches plus étroites qu'en ABC."
        }
    }

    private fun renderLetters() {
        letterFlow.removeAllViews()
        for (n in 2..4) {
            letterFlow.addView(chip("$n lettres", car.maxLetters == n) {
                car.maxLetters = n
                renderLetters()
            })
        }
    }

    // -- Playlists épinglées -------------------------------------------------

    private fun pinsCard(): LinearLayout {
        val card = newCard()
        val head = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        head.addView(cardTitle("Playlists du mode voiture"), weight())
        pinCount = CarStyle.monoText(this, "", 12f, CarStyle.AC).apply {
            gravity = Gravity.END or Gravity.CENTER_VERTICAL
        }
        head.addView(
            pinCount,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            ).apply { leftMargin = d(8) }
        )
        addStack(card, head)

        val hint = CarStyle.text(
            this,
            "Épingle celles que tu veux voir en roulant. « Favoris » reste toujours en premier. Tu peux aussi épingler depuis la page d'une playlist dans l'app. Les épinglées restent sur cet appareil, même sans serveur.",
            13f,
            CarStyle.DIM,
        ).apply { setLineSpacing(0f, 1.4f) }
        addStack(card, hint, 6)

        pinsStatus = CarStyle.monoText(this, "", 11f, CarStyle.DIM).apply {
            setPadding(0, d(4), 0, 0)
        }
        addStack(card, pinsStatus, 6)

        pinsBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        addStack(card, pinsBox, 4)
        renderPins()
        return card
    }

    private fun renderPins() {
        pinsBox.removeAllViews()
        val pins = car.pins
        val tiles = pins.size + 1
        pinCount.text = if (tiles == 1) "1 tuile" else "$tiles tuiles"

        val rows = rowsForSettings(pins)
        if (rows.isEmpty()) {
            pinsBox.addView(
                CarStyle.text(
                    this,
                    "Aucune playlist pour l'instant. Crée-en une dans l'app, puis épingle-la ici.",
                    13f,
                    CarStyle.DIM,
                ).apply { setLineSpacing(0f, 1.4f) },
                topWrap(8)
            )
        } else {
            for ((index, row) in rows.withIndex()) {
                pinsBox.addView(
                    pinRow(row, pins, index < COVER_ROWS),
                    LinearLayout.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT,
                        ViewGroup.LayoutParams.WRAP_CONTENT
                    ).apply { topMargin = d(4) }
                )
            }
        }
        renderCatalogueStatus()
    }

    private fun rowsForSettings(pins: List<PinnedPlaylist>): List<CatalogueRow> {
        val rows = mutableListOf<CatalogueRow>()
        val seen = HashSet<String>()
        // Pinned first, in the car's own order: "monter" must move a row the
        // person can see, or the reorder is blind. Then the catalogue, minus
        // what is already pinned. A pin the server no longer lists stays at
        // the top (marked) so it can be detached.
        val hasCatalogue = catalogue == Catalogue.READY || serverItems.isNotEmpty()
        for (pin in pins) {
            if (!seen.add(pin.id)) continue
            val fresh = CarPlaylistSource.find(serverItems, pin.id)
            // A pin stored under the Deezer id matches the list's UUID row:
            // remember both ids so the catalogue loop below does not add the
            // same playlist a second time.
            if (fresh != null) {
                seen.add(fresh.id)
                fresh.deezerId?.let { seen.add(it) }
            }
            rows.add(
                CatalogueRow(
                    pin.id,
                    fresh?.title ?: pin.title.ifBlank { "Playlist" },
                    fresh?.nbTracks ?: pin.nbTracks,
                    hasCatalogue && fresh == null,
                    fresh?.cover,
                )
            )
        }
        for (item in serverItems) {
            if (!seen.add(item.id)) continue
            rows.add(CatalogueRow(item.id, item.title, item.nbTracks, false, item.cover))
        }
        return rows
    }

    private fun pinRow(row: CatalogueRow, pins: List<PinnedPlaylist>, withCover: Boolean): View {
        val line = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(d(4), d(6), d(4), d(6))
        }
        val cover = CarCoverView(this).apply {
            seed = row.id
            sizeDp = 40
            // The pinned rows (first in the list) get the real sleeve; the
            // rest of the catalogue stays on its grooves so opening a 500-row
            // settings screen never fans out 500 image requests.
            coverUrl = if (withCover) CarCovers.resolve(prefs.baseUrl(), row.cover) else ""
        }
        line.addView(cover, LinearLayout.LayoutParams(d(40), d(40)))

        val text = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val title = CarStyle.text(this, row.title, 16f, CarStyle.TX).apply {
            typeface = Typeface.create(CarType.body(this@CarSettingsActivity), Typeface.BOLD)
            maxLines = 1
            ellipsize = TextUtils.TruncateAt.END
        }
        text.addView(title)
        if (row.stale) {
            text.addView(
                CarStyle.monoText(this, "introuvable sur le serveur", 10f, CarStyle.WARN),
                topWrap(1)
            )
        }
        line.addView(
            text,
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply {
                leftMargin = d(10)
            }
        )

        val pinAt = pins.indexOfFirst { it.id == row.id }
        val pinned = pinAt >= 0
        if (pinned && pinAt > 0) {
            val up = iconButton(40, Color.TRANSPARENT, Color.TRANSPARENT, 0f, "chevronUp", 16, CarStyle.TX, "Monter") {
                car.movePin(row.id, -1)
                renderPins()
            }
            line.addView(
                up,
                LinearLayout.LayoutParams(d(40), d(40)).apply { rightMargin = d(4) }
            )
        }
        val pin = iconButton(
            44,
            if (pinned) 0x2ED2FF3C else Color.TRANSPARENT,
            Color.TRANSPARENT,
            0f,
            "pin",
            20,
            if (pinned) CarStyle.AC else CarStyle.DIM,
            if (pinned) "Détacher de la voiture" else "Épingler pour la voiture",
        ) { togglePin(row, pinned) }
        (pin.getChildAt(0) as? CarIconView)?.filled = pinned
        line.addView(pin, LinearLayout.LayoutParams(d(44), d(44)))
        return line
    }

    private fun togglePin(row: CatalogueRow, pinned: Boolean) {
        if (pinned) {
            car.unpin(row.id)
            toast("« ${row.title} » retirée de la voiture")
        } else if (car.pin(PinnedPlaylist(row.id, row.title, row.nbTracks))) {
            toast("« ${row.title} » épinglée pour la voiture")
        } else {
            // The 24-pin ceiling (CarPrefs.MAX_PINS): say why, never claim a
            // pin that was refused.
            toast(getString(R.string.car_pin_limit))
        }
        renderPins()
    }

    private fun renderCatalogueStatus() {
        val retry = when (catalogue) {
            Catalogue.NO_SESSION,
            Catalogue.OFFLINE,
            Catalogue.TIMEOUT,
            Catalogue.TLS,
            Catalogue.SERVER,
            Catalogue.UNEXPECTED
            -> true
            else -> false
        }
        var text = ""
        var color = CarStyle.DIM
        when (catalogue) {
            Catalogue.IDLE -> Unit
            Catalogue.LOADING -> text = "Chargement des playlists…"
            Catalogue.READY -> if (serverItems.isEmpty()) {
                text = "Aucune playlist sur ce compte. Les épinglées restent sur cet appareil."
            }
            Catalogue.NO_SERVER -> {
                text = "Aucun serveur configuré. Les épinglées restent sur cet appareil."
            }
            Catalogue.NO_SESSION -> {
                text = "Session absente — connecte-toi dans l'app. " +
                    "Les épinglées restent sur cet appareil · réessayer"
            }
            Catalogue.OFFLINE -> {
                text = "Hors ligne — les épinglées restent sur cet appareil · réessayer"
                color = CarStyle.WARN
            }
            Catalogue.TIMEOUT -> {
                text = "Le serveur ne répond pas (délai dépassé) · réessayer"
                color = CarStyle.WARN
            }
            Catalogue.TLS -> {
                text = "Certificat refusé — les épinglées restent sur cet appareil · réessayer"
                color = CarStyle.WARN
            }
            Catalogue.SERVER -> {
                text = "Le serveur a répondu par une erreur · réessayer"
                color = CarStyle.WARN
            }
            Catalogue.UNEXPECTED -> {
                text = "Réponse inattendue du serveur (page de connexion ?) · réessayer"
                color = CarStyle.WARN
            }
        }
        pinsStatus.visibility = if (text.isEmpty()) View.GONE else View.VISIBLE
        pinsStatus.text = text
        pinsStatus.setTextColor(color)
        pinsStatus.isClickable = retry
        pinsStatus.isFocusable = retry
        pinsStatus.foreground = if (retry) ripple(6f) else null
        pinsStatus.setOnClickListener(if (retry) View.OnClickListener { refreshCatalogue(true) } else null)
        pinsStatus.contentDescription =
            if (retry) "$text — toucher pour réessayer" else null
    }

    /**
     * Read (or re-read) the catalogue. On return to the screen a successful
     * read younger than 60 s is reused; a failure is retried, at most once
     * every 2 s so a flapping screen cannot hammer the server.
     */
    private fun refreshCatalogue(force: Boolean = false) {
        val now = SystemClock.elapsedRealtime()
        val freshSuccess = catalogue == Catalogue.READY && lastSuccessAt != 0L &&
            now - lastSuccessAt < 60_000L
        if (catalogue == Catalogue.LOADING) return
        if (!force && freshSuccess) return
        if (!force && lastAttemptAt != 0L && now - lastAttemptAt < 2_000L) return

        val url = CarPlaylistSource.endpoint(prefs)
        if (url == null) {
            catalogue = Catalogue.NO_SERVER
            renderCatalogueStatus()
            return
        }
        val seq = ++loadSeq
        lastAttemptAt = now
        catalogue = Catalogue.LOADING
        renderCatalogueStatus()
        CarPlaylistSource.IO.execute {
            val outcome = carListOutcome { CarPlaylistSource.load(prefs, url) }
            runOnUiThread {
                if (seq != loadSeq || isFinishing || isDestroyed) return@runOnUiThread
                when (val result = outcome) {
                    is CarListOutcome.Ok -> {
                        serverItems = result.items
                        // The overlay's own cache must see the same read, so
                        // opening the panel does not cost a second request.
                        CarPlaylistSource.remember(result.items)
                        rememberCounts(result.items)
                        lastSuccessAt = SystemClock.elapsedRealtime()
                        catalogue = Catalogue.READY
                    }
                    is CarListOutcome.Fail -> {
                        if (result.failure is CarListFailure.NoSession) {
                            // A 401 is not a stale menu: the cache must not
                            // survive a session that is gone.
                            CarPlaylistSource.forget()
                        }
                        catalogue = when (result.failure) {
                            is CarListFailure.NoSession -> Catalogue.NO_SESSION
                            is CarListFailure.Timeout -> Catalogue.TIMEOUT
                            is CarListFailure.Tls -> Catalogue.TLS
                            is CarListFailure.Http -> Catalogue.SERVER
                            is CarListFailure.Unexpected -> Catalogue.UNEXPECTED
                            else -> Catalogue.OFFLINE
                        }
                    }
                }
                renderPins()
            }
        }
    }

    /** A successful read refreshes the pinned titles/counts; the order stays. */
    private fun rememberCounts(items: List<CarPlaylistSource.Item>) {
        var changed = false
        val next = car.pins.map { pin ->
            val fresh = CarPlaylistSource.find(items, pin.id) ?: return@map pin
            if (fresh.title == pin.title && fresh.nbTracks == pin.nbTracks) pin
            else {
                changed = true
                PinnedPlaylist(pin.id, fresh.title, fresh.nbTracks)
            }
        }
        if (changed) car.pins = next
    }

    // -- Opacité -------------------------------------------------------------

    private fun opacityCard(): LinearLayout {
        val card = newCard()
        val head = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        head.addView(cardTitle("Opacité de la barre"), weight())
        val value = CarStyle.monoText(this, "", 14f, CarStyle.AC)
        head.addView(
            value,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            ).apply { leftMargin = d(8) }
        )
        addStack(card, head)

        val hint = CarStyle.text(
            this,
            "Plus la barre est opaque, moins on voit la route derrière. Android refuse les touches au-delà de 80 % : le mode voiture s'arrête là.",
            13f,
            CarStyle.DIM,
        ).apply { setLineSpacing(0f, 1.4f) }
        addStack(card, hint, 6)

        val initial = (car.overlayAlpha * 100f).roundToInt().coerceIn(40, 80)
        value.text = "$initial %"
        val bar = SeekBar(this).apply {
            min = 40
            max = 80
            progress = initial
            progressTintList = ColorStateList.valueOf(CarStyle.AC)
            progressBackgroundTintList = ColorStateList.valueOf(CarStyle.LN)
            thumbTintList = ColorStateList.valueOf(CarStyle.AC)
            splitTrack = false
            contentDescription = "Opacité de la barre, de 40 à 80 pour cent"
            setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                override fun onProgressChanged(seekBar: SeekBar?, progress: Int, fromUser: Boolean) {
                    car.overlayAlpha = progress / 100f
                    value.text = "$progress %"
                }

                override fun onStartTrackingTouch(seekBar: SeekBar?) = Unit
                override fun onStopTrackingTouch(seekBar: SeekBar?) = Unit
            })
        }
        addStack(card, bar, 10)

        val ends = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        ends.addView(CarStyle.monoText(this, "40 %", 10f, CarStyle.DIM))
        ends.addView(View(this), weight())
        ends.addView(CarStyle.monoText(this, "80 %", 10f, CarStyle.DIM))
        addStack(card, ends, 2)
        return card
    }

    // -- small factories -----------------------------------------------------

    private fun newCard(): LinearLayout = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(d(16), d(16), d(16), d(16))
        background = CarStyle.roundRect(
            this@CarSettingsActivity, CarStyle.S1, CarStyle.LN, 20f, 1f
        )
    }

    private fun addCard(card: View) {
        content.addView(
            card,
            LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            ).apply { bottomMargin = d(14) }
        )
    }

    private fun addStack(parent: LinearLayout, child: View, top: Int = 12) {
        val lp = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        if (parent.childCount > 0) lp.topMargin = d(top)
        parent.addView(child, lp)
    }

    private fun cardTitle(text: String): TextView =
        CarStyle.text(this, text, 17f, CarStyle.TX).apply {
            typeface = Typeface.create(CarType.body(this@CarSettingsActivity), Typeface.BOLD)
        }

    private fun monoLabel(text: String): TextView =
        CarStyle.monoText(this, text, 11f, CarStyle.DIM).apply {
            letterSpacing = 0.08f
        }

    private fun textBlock(title: String, hint: String): View {
        val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        column.addView(CarStyle.text(this, title, 15f, CarStyle.TX))
        column.addView(
            CarStyle.text(this, hint, 13f, CarStyle.DIM).apply { setLineSpacing(0f, 1.35f) },
            topWrap(2)
        )
        return column
    }

    private fun toggleRow(
        title: String,
        hint: String,
        initial: Boolean,
        onChange: (Boolean) -> Unit,
    ): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        row.addView(
            textBlock(title, hint),
            LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        )
        val toggle = CarToggleView(this).apply {
            contentDescription = title
            setChecked(initial, false)
            setOnCheckedChangeListener(onChange)
        }
        row.addView(
            toggle,
            LinearLayout.LayoutParams(d(52), d(30)).apply { leftMargin = d(12) }
        )
        return row
    }

    private fun chip(label: String, on: Boolean, onClick: () -> Unit): TextView =
        CarStyle.text(this, label, 14f, if (on) CarStyle.ONAC else CarStyle.TX).apply {
            gravity = Gravity.CENTER
            includeFontPadding = false
            minimumHeight = d(40)
            setPadding(d(14), 0, d(14), 0)
            typeface = Typeface.create(
                CarType.body(this@CarSettingsActivity),
                if (on) Typeface.BOLD else Typeface.NORMAL
            )
            background = CarStyle.roundRect(
                this@CarSettingsActivity,
                if (on) CarStyle.AC else Color.TRANSPARENT,
                if (on) CarStyle.AC else CarStyle.LN,
                20f,
                1f,
            )
            foreground = ripple(20f, if (on) 0x33000000 else 0x33FFFFFF)
            isClickable = true
            isFocusable = true
            setOnClickListener { onClick() }
        }

    private fun iconButton(
        sizeDp: Int,
        fill: Int,
        stroke: Int,
        strokeDp: Float,
        icon: String,
        iconDp: Int,
        tint: Int,
        description: String,
        onClick: () -> Unit,
    ): FrameLayout {
        val box = FrameLayout(this).apply {
            background = CarStyle.roundRect(
                this@CarSettingsActivity,
                fill,
                stroke,
                sizeDp / 2f,
                if (stroke != Color.TRANSPARENT) strokeDp else 0f,
            )
            foreground = ripple(sizeDp / 2f)
            contentDescription = description
            isClickable = true
            isFocusable = true
            setOnClickListener { onClick() }
        }
        val glyph = CarIconView(this).apply {
            name = icon
            this.sizeDp = iconDp
            color = tint
        }
        box.addView(
            glyph,
            FrameLayout.LayoutParams(d(iconDp), d(iconDp), Gravity.CENTER)
        )
        return box
    }

    private fun divider(): View = View(this).apply {
        setBackgroundColor(CarStyle.LN)
        layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT,
            d(1)
        )
    }

    private fun topWrap(top: Int): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply { topMargin = d(top) }

    private fun weight(): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)

    private fun d(value: Int): Int = CarStyle.dp(this, value.toFloat())
    private fun d(value: Float): Int = CarStyle.dp(this, value)

    private fun ripple(radiusDp: Float, color: Int = 0x33FFFFFF): RippleDrawable {
        val mask = GradientDrawable().apply {
            shape = GradientDrawable.RECTANGLE
            cornerRadius = d(radiusDp).toFloat()
            setColor(Color.WHITE)
        }
        return RippleDrawable(ColorStateList.valueOf(color), null, mask)
    }

    /** A large, Sillon-coloured toast — no dialog, and readable at arm's length. */
    private fun toast(message: String) {
        val label = CarStyle.text(this, message, 16f, CarStyle.ONAC).apply {
            gravity = Gravity.CENTER
            maxLines = 3
            maxWidth = d(320)
        }
        val box = FrameLayout(this).apply {
            background = CarStyle.roundRect(
                this@CarSettingsActivity, CarStyle.AC, Color.TRANSPARENT, 18f, 0f
            )
            setPadding(d(18), d(14), d(18), d(14))
            addView(
                label,
                FrameLayout.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT
                )
            )
        }
        Toast(this).apply {
            duration = Toast.LENGTH_SHORT
            view = box
            setGravity(Gravity.BOTTOM or Gravity.CENTER_HORIZONTAL, 0, d(80))
        }.show()
    }

    companion object {
        /** At most this many rows of the settings list fetch a real sleeve. */
        private const val COVER_ROWS = 12

        private val LAYOUTS = listOf(
            "auto" to "Auto (système)",
            "azerty" to "AZERTY",
            "qwerty" to "QWERTY",
            "qwertz" to "QWERTZ",
            "abc" to "ABC",
        )

        /**
         * The prototype's five navigation apps. OsmAnd has two package names
         * (free and +): one chip controls both, and the defaults in
         * [CarPrefs.DEFAULT_GPS] stay coherent whichever build is installed.
         */
        private val GPS_APPS = listOf(
            GpsApp("Google Maps", setOf("com.google.android.apps.maps")),
            GpsApp("Waze", setOf("com.waze")),
            GpsApp("OsmAnd", setOf("net.osmand", "net.osmand.plus")),
            GpsApp("Organic Maps", setOf("app.organicmaps")),
            GpsApp("Sygic", setOf("com.sygic.aura")),
        )
    }

    /**
     * A wrapping row of chips. The design wraps them onto as many lines as the
     * width needs; fixed "3 per row" would clip at a large font scale, and a
     * horizontal scroller would hide the remaining apps from a thumb at rest.
     */
    private class CarFlow(context: Context) : ViewGroup(context) {
        var gap: Int = 0

        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val widthSize = View.MeasureSpec.getSize(widthMeasureSpec)
            val available = (widthSize - paddingLeft - paddingRight).coerceAtLeast(0)
            var x = 0
            var rowHeight = 0
            var total = 0
            for (i in 0 until childCount) {
                val child = getChildAt(i)
                if (child.visibility == GONE) continue
                child.measure(
                    View.MeasureSpec.makeMeasureSpec(available, View.MeasureSpec.AT_MOST),
                    View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED)
                )
                val w = child.measuredWidth
                if (x > 0 && x + gap + w > available) {
                    total += rowHeight + gap
                    x = 0
                    rowHeight = 0
                }
                if (x > 0) x += gap
                x += w
                rowHeight = max(rowHeight, child.measuredHeight)
            }
            total += rowHeight
            setMeasuredDimension(
                View.resolveSize(widthSize, widthMeasureSpec),
                View.resolveSize(total + paddingTop + paddingBottom, heightMeasureSpec)
            )
        }

        override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
            val available = (r - l - paddingLeft - paddingRight).coerceAtLeast(0)
            var x = paddingLeft
            var y = paddingTop
            var rowHeight = 0
            for (i in 0 until childCount) {
                val child = getChildAt(i)
                if (child.visibility == GONE) continue
                val w = child.measuredWidth
                val h = child.measuredHeight
                if (x > paddingLeft && x + gap + w > paddingLeft + available) {
                    x = paddingLeft
                    y += rowHeight + gap
                    rowHeight = 0
                }
                child.layout(x, y, x + w, y + h)
                x += w + gap
                rowHeight = max(rowHeight, h)
            }
        }

        override fun generateDefaultLayoutParams(): ViewGroup.LayoutParams =
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )

        override fun generateLayoutParams(attrs: AttributeSet?): ViewGroup.LayoutParams =
            ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.WRAP_CONTENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
    }

    /**
     * The prototype's 52 × 30 switch, drawn (the platform's own would be the
     * one default-looking widget in a screen that is otherwise entirely
     * Sillon). It reports itself as a Switch to accessibility services and
     * animates its knob in 140 ms.
     */
    private class CarToggleView(context: Context) : View(context) {
        private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        private val evaluator = ArgbEvaluator()
        private var anim = 0f
        private var animator: ValueAnimator? = null
        private var listener: ((Boolean) -> Unit)? = null

        var checked: Boolean = false
            private set

        init {
            isClickable = true
            isFocusable = true
        }

        fun setOnCheckedChangeListener(callback: (Boolean) -> Unit) {
            listener = callback
        }

        fun setChecked(value: Boolean, animate: Boolean) {
            if (checked == value) return
            checked = value
            animator?.cancel()
            if (!animate) {
                anim = if (value) 1f else 0f
                invalidate()
                return
            }
            animator = ValueAnimator.ofFloat(anim, if (value) 1f else 0f).apply {
                duration = 140
                addUpdateListener {
                    anim = it.animatedValue as Float
                    invalidate()
                }
                start()
            }
        }

        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            setMeasuredDimension(d(52), d(30))
        }

        override fun onDraw(canvas: Canvas) {
            val w = width.toFloat()
            val h = height.toFloat()
            if (w <= 0f || h <= 0f) return
            paint.style = Paint.Style.FILL
            paint.color = evaluator.evaluate(anim, CarStyle.LN, CarStyle.AC) as Int
            canvas.drawRoundRect(0f, 0f, w, h, h / 2f, h / 2f, paint)
            paint.color = evaluator.evaluate(anim, CarStyle.DIM, CarStyle.ONAC) as Int
            val radius = d(12).toFloat()
            val cx = d(3).toFloat() + d(22).toFloat() * anim + radius
            canvas.drawCircle(cx, h / 2f, radius, paint)
        }

        override fun onTouchEvent(event: MotionEvent): Boolean {
            if (!isEnabled) return false
            return when (event.actionMasked) {
                MotionEvent.ACTION_DOWN -> {
                    isPressed = true
                    true
                }
                MotionEvent.ACTION_UP -> {
                    isPressed = false
                    performClick()
                    true
                }
                MotionEvent.ACTION_CANCEL -> {
                    isPressed = false
                    true
                }
                else -> super.onTouchEvent(event)
            }
        }

        override fun performClick(): Boolean {
            super.performClick()
            setChecked(!checked, true)
            listener?.invoke(checked)
            return true
        }

        override fun onInitializeAccessibilityNodeInfo(info: AccessibilityNodeInfo) {
            super.onInitializeAccessibilityNodeInfo(info)
            info.className = "android.widget.Switch"
            info.isCheckable = true
            info.isChecked = checked
            info.isClickable = true
        }

        override fun onDetachedFromWindow() {
            animator?.cancel()
            animator = null
            super.onDetachedFromWindow()
        }

        private fun d(value: Int): Int = CarStyle.dp(context, value.toFloat())
    }
}
