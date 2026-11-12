package org.nsupysonic.app

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import java.util.Locale

/**
 * The car's voice search: French, one sentence, "Joue Angerfist".
 *
 * [SpeechRecognizer] is a one-shot API and its javadoc is explicit: every method
 * must run on the main application thread, the caller must hold RECORD_AUDIO,
 * and the instance must be destroyed when no longer needed. This wrapper keeps
 * one recognizer, recreates it after [destroy] (so a re-attached overlay can
 * speak again), and never claims to listen when the permission or the recognizer
 * is missing.
 *
 * Two platform requirements live OUTSIDE this file and belong to the service
 * lot: apps targeting Android 11+ need the `<queries>` intent for
 * `android.speech.RecognitionService`, and a foreground service that records
 * must declare the `microphone` type (plus FOREGROUND_SERVICE_MICROPHONE,
 * started while an activity is visible on Android 14+). Without them the OS
 * answers ERROR_INSUFFICIENT_PERMISSIONS even though [CarPermissions] says the
 * runtime grant is present; [onFailure] reports that instead of pretending.
 *
 * The recognizer returns "Joue Angerfist" (or "Joue-moi ...", "Mets ...",
 * "Lance ..."), so the leading verb is stripped before [onText] sees the query.
 * A bare command ("Joue") becomes an empty string and is reported as "Rien
 * entendu" by the caller through [onFailure].
 */
class CarSpeech(context: Context, private val onText: (String) -> Unit) {
    private val appContext = context.applicationContext
    private var recognizer: SpeechRecognizer? = null
    private var listening = false

    /** The best partial hypothesis, for the query field while the person speaks. */
    var onPartial: ((String) -> Unit)? = null

    /** One short, actionable French sentence; the caller shows it as a toast. */
    var onFailure: ((String) -> Unit)? = null

    /** Mirrors the session so the caller can stop its microphone pulse. */
    var onState: ((Boolean) -> Unit)? = null

    val isListening: Boolean
        get() = listening

    /**
     * Ask the recognizer to listen. Returns false when the session never began
     * (no permission, no recognition service, a refused service); [onFailure]
     * already carries the reason. Must be called on the main thread.
     */
    fun start(): Boolean {
        if (listening) return false
        if (!CarPermissions.hasMicrophone(appContext)) {
            fail("Micro non autorisé")
            return false
        }
        val available = try {
            SpeechRecognizer.isRecognitionAvailable(appContext)
        } catch (_: Exception) {
            false
        }
        if (!available) {
            fail("Reconnaissance vocale indisponible")
            return false
        }
        val engine = recognizer ?: createRecognizer()
        if (engine == null) {
            fail("Reconnaissance vocale indisponible")
            return false
        }
        listening = true
        onState?.invoke(true)
        return try {
            engine.startListening(intent())
            true
        } catch (_: SecurityException) {
            // RECORD_AUDIO granted but the OS still refuses: the overlay is
            // running while the app is backgrounded and the car service has no
            // microphone foreground-service type. Not retried here.
            destroyRecognizer()
            fail("Micro refusé — vérifie l'autorisation")
            false
        } catch (_: Exception) {
            destroyRecognizer()
            fail("Recherche vocale impossible")
            false
        }
    }

    /**
     * Cancel the current session. The recognizer stays alive for the next
     * [start]; the terminal ERROR_CLIENT it emits is ignored because [listening]
     * is already false. Main thread only.
     */
    fun stop() {
        if (!listening) return
        listening = false
        onState?.invoke(false)
        runCatching { recognizer?.cancel() }
    }

    /**
     * Release the recognizer. Safe to call repeatedly: [start] creates a fresh
     * instance afterwards. Main thread only.
     */
    fun destroy() {
        stop()
        destroyRecognizer()
    }

    private fun createRecognizer(): SpeechRecognizer? = try {
        SpeechRecognizer.createSpeechRecognizer(appContext).also {
            it.setRecognitionListener(listener)
            recognizer = it
        }
    } catch (_: Exception) {
        null
    }

    private fun destroyRecognizer() {
        val engine = recognizer ?: return
        recognizer = null
        runCatching { engine.destroy() }
    }

    private fun fail(message: String) {
        listening = false
        onState?.invoke(false)
        onFailure?.invoke(message)
    }

    private fun intent(): Intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
        putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
        // The car UI is French; a recognition service that has no French model
        // answers ERROR_LANGUAGE_NOT_SUPPORTED, which messageFor names.
        putExtra(RecognizerIntent.EXTRA_LANGUAGE, "fr-FR")
        putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
        putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 3)
    }

    private val listener = object : RecognitionListener {
        override fun onReadyForSpeech(params: Bundle?) = Unit

        override fun onBeginningOfSpeech() = Unit

        override fun onRmsChanged(rmsdB: Float) = Unit

        override fun onBufferReceived(buffer: ByteArray?) = Unit

        override fun onEndOfSpeech() = Unit

        override fun onError(error: Int) {
            // A cancel() arrives here as ERROR_CLIENT after stop() cleared the
            // flag: not a failure the person should see.
            if (!listening) return
            listening = false
            onState?.invoke(false)
            if (error == SpeechRecognizer.ERROR_SERVER_DISCONNECTED) {
                // The recognition service died; a fresh instance is needed for
                // the next sentence.
                destroyRecognizer()
            }
            onFailure?.invoke(messageFor(error))
        }

        override fun onResults(results: Bundle?) {
            if (!listening) return
            listening = false
            onState?.invoke(false)
            val raw = results
                ?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                ?.firstOrNull { it.isNotBlank() }
                ?: run {
                    onFailure?.invoke("Rien entendu")
                    return
                }
            val query = stripCommand(raw)
            if (query.isBlank()) {
                onFailure?.invoke("Rien entendu")
            } else {
                onText(query)
            }
        }

        override fun onPartialResults(partialResults: Bundle?) {
            if (!listening) return
            val raw = partialResults
                ?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)
                ?.firstOrNull { it.isNotBlank() }
                ?: return
            onPartial?.invoke(raw.trim())
        }

        override fun onEvent(eventType: Int, params: Bundle?) = Unit
    }

    /** "Joue Angerfist" -> "Angerfist", keeping the artist's own capitalisation. */
    private fun stripCommand(raw: String): String {
        val text = raw.trim().trimEnd('.', ',', '!', '?', ';', ':')
        val lower = text.lowercase(Locale.FRENCH)
        if (lower in BARE_COMMANDS) return ""
        val match = COMMAND.find(lower)
        if (match != null && match.range.first == 0) {
            return text.substring(match.range.last + 1).trim()
        }
        return text
    }

    private fun messageFor(error: Int): String = when (error) {
        SpeechRecognizer.ERROR_AUDIO -> "Micro indisponible"
        SpeechRecognizer.ERROR_CLIENT -> "Recherche vocale interrompue"
        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS ->
            "Micro refusé — vérifie l'autorisation"
        SpeechRecognizer.ERROR_NETWORK,
        SpeechRecognizer.ERROR_NETWORK_TIMEOUT,
        SpeechRecognizer.ERROR_SERVER,
        SpeechRecognizer.ERROR_SERVER_DISCONNECTED -> "Réseau indisponible pour la voix"
        SpeechRecognizer.ERROR_NO_MATCH,
        SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "Rien entendu"
        SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "Reconnaissance occupée"
        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED,
        SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE -> "Français indisponible pour la voix"
        SpeechRecognizer.ERROR_TOO_MANY_REQUESTS -> "Trop de demandes — réessaie"
        else -> "Recherche vocale impossible"
    }

    companion object {
        /**
         * The verb the person speaks, then what to search for. The match is done
         * on the lowercased text, so "Écoute" and "écoute" both work without
         * relying on Java's ASCII-only case folding. The separator accepts the
         * comma or colon a recognizer likes to insert after the verb.
         */
        private val COMMAND = Regex(
            "^(?:joue(?:[- ]moi)?|mets(?:[- ]moi)?|lance(?:[- ]moi)?|" +
                "écoute|ecoute|je veux écouter|je veux ecouter|" +
                "fais(?:[- ]moi)? écouter)[\\s,:;-]+",
        )

        private val BARE_COMMANDS = setOf(
            "joue", "joue-moi", "joue moi",
            "mets", "mets-moi", "mets moi",
            "lance", "lance-moi", "lance moi",
            "écoute", "ecoute", "je veux écouter", "je veux ecouter",
        )
    }
}
