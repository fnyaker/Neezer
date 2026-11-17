package fr.neezer.mobile.playback

import android.content.Intent
import androidx.media3.session.MediaLibraryService
import androidx.media3.session.MediaSession

/**
 * The app's media service (docs/agents/mobile.md: "Media3 `MediaLibraryService` +
 * a `SimpleBasePlayer` facade (`NzPlayer`, main looper) whose state is the Rust
 * snapshot and whose commands go to Rust").
 *
 * MB0 declares the service so the manifest, the foreground type and the two
 * intent filters exist; it answers NO session yet, which is honest (nothing can
 * play) and keeps `aapt2`/lint able to check the whole component graph.
 *
 * MB3c fills: `onGetSession` returns the session built on `NzPlayer`, the
 * notification and its channel, audio focus (one request, transient loss pauses,
 * duck), becoming-noisy, and the battery-exemption offer written once at play and
 * cleared at a clean pause.
 */
class NzMediaService : MediaLibraryService() {

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaLibrarySession? {
        // MB3c: the session over `NzPlayer`. A null session is legal, and the
        // platform simply sees a service with nothing to play yet.
        return null
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // The engine keeps playing behind a locked screen; a swiped-away task is
        // not a stop (the notification is the only door out).
        super.onTaskRemoved(rootIntent)
    }
}
