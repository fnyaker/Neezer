package org.nsupysonic.app

/**
 * What the app's player does ON ITS OWN while the activity is stopped.
 *
 * Everything below was a phone bug with one cause: a command, a failure or a
 * network need was left to the PAGE, and the page is exactly the part that a
 * locked screen throttles and then freezes (developer.chrome.com, "timer
 * throttling in Chrome 88"; the Page Lifecycle API: a frozen page runs
 * nothing). Media3's model is the opposite — the player that owns the playlist
 * answers the command, whoever asks (developer.android.com, MediaSession) — so
 * next/previous already ran in the app ([NativeAudio.skip]); play, pause and
 * seek, a title that cannot be played, and the link a download yields to were
 * still routed to a renderer that may be gone.
 *
 * The decisions live here, away from Android, so a JVM test holds them
 * (NativeRoutingTest): the rule is small, and the failure it prevents is
 * expensive (silence until the phone is unlocked).
 */
enum class TransportTarget {
    /** The app's own player answers it (its state tells the page afterwards). */
    APP,

    /** It goes to the page, as it always did. */
    PAGE,
}

/** What the app does with a title it could not play while its activity is stopped. */
enum class FaultAction {
    /** The page is shown (or was never stopped): its own recovery ladder decides. */
    PAGE,

    /** A bad moment on the network: reopen the same source, a few times. */
    RETRY,

    /** A verdict about this TITLE: consume it and start the next one. */
    ADVANCE,

    /** Nothing left to play: the session and the notification must say "paused". */
    STALL,
}

object NativeRouting {
    /** The transport commands the app's player can apply itself. */
    val OPS = setOf("play", "pause", "seek")

    /**
     * Who answers a MediaSession command (the notification, a headset, the lock
     * screen, the car card).
     *
     * [hasSlot] is false when the page plays through its own `<audio>` (the
     * setting is off, or an older SPA): the command must still reach the page,
     * which is then the only one that can apply it.
     */
    fun transport(foreground: Boolean, hasSlot: Boolean, cmd: String): TransportTarget =
        if (!foreground && hasSlot && cmd in OPS) TransportTarget.APP else TransportTarget.PAGE

    /**
     * Whether the 4 Hz position tick is worth sending to the page. Four states a
     * second through `evaluateJavascript` into a frozen renderer queue up until
     * the phone is unlocked, and the page's own clock does not need them: a
     * frozen page reads none of it, and a throttled one interpolates the
     * position from the last state it knows. Nothing else rides on the tick: the
     * trimmed end is checked on the app's own clock (`checkTrim`) either way.
     */
    fun tickToPage(foreground: Boolean): Boolean = foreground

    /**
     * What to do with [NativeAudio]'s `onPlayerError` while the activity is
     * stopped.
     *
     * An error with no next track stopped the WHOLE named list: a 404 on track 5
     * of 20 left fifteen titles unplayed until the phone was unlocked, while the
     * page's own policy (`Player.svelte#recoverPlayback` / `probeCurrent`) is to
     * skip the title that cannot be played. A title costs one title, not the
     * queue — but a transient outage is not a verdict about twenty of them, so
     * only a NON-retryable error (a 4xx, a decode error, a refused source)
     * advances; a retryable one that outlived its budget stalls, as before.
     * [skipsLeft] bounds a run of consecutive failures (a broken server answering
     * 404 to everything) so the app cannot burn the whole list in a second.
     */
    fun fault(
        foreground: Boolean,
        retryable: Boolean,
        retriesLeft: Boolean,
        hasNext: Boolean,
        repeatOne: Boolean,
        skipsLeft: Boolean,
    ): FaultAction = when {
        foreground -> FaultAction.PAGE
        retryable -> if (retriesLeft) FaultAction.RETRY else FaultAction.STALL
        !hasNext || repeatOne || !skipsLeft -> FaultAction.STALL
        else -> FaultAction.ADVANCE
    }
}

/**
 * Whether the app's own player needs the network, for the download gate
 * (`YieldGate`): the same rule the page applies to its own elements
 * (`lib/netyield.js#elementWants`, WANT_S / ENOUGH_S), on the app's side of the
 * bridge.
 *
 * It has to live in the app: the page's lease is a `setInterval` renewal plus
 * the element's events, and behind a locked screen both stop — so the lease
 * expired after 45 s and the downloads went back to full speed on the same link
 * the native player was streaming from. The player that pulls the stream is in
 * the app's process and keeps its own lease, renewed on the 250 ms tick from the
 * buffer ExoPlayer reports, and lets it go once 30 s are in hand.
 */
object LinkNeed {
    /** Under this many seconds ahead, the player needs the link. */
    const val WANT_S = 15.0

    /** ...and at this many it lets go again (the two thresholds stop it flapping). */
    const val ENOUGH_S = 30.0

    /**
     * [active] is the player PULLING: it was told to play and is preparing or reading
     * (ExoPlayer's STATE_BUFFERING / STATE_READY). An idle player — before a source, or
     * after a fatal error — holds nothing, or the gate would stay closed for good.
     */
    fun wanted(active: Boolean, network: Boolean, aheadS: Double, held: Boolean): Boolean {
        if (!active || !network) return false
        if (aheadS.isNaN()) return true
        return aheadS < if (held) ENOUGH_S else WANT_S
    }
}
