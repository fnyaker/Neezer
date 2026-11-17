package fr.neezer.mobile.bridge

import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.nzErrorText

/**
 * The French sentence for any failure, and never a code (rule 3).
 *
 * `nz_error_text` is Rust's single register for every `NzError` variant, so the
 * UI must not re-word them: one place decides what "hors ligne" reads like, and
 * a new variant is a compile-time gap there, not a missing string here.
 */
object NzErrors {
    /** `e` in the person's words; unknown throwables keep their own message. */
    fun text(e: Throwable): String = when (e) {
        is NzException -> nzErrorText(e)
        else -> e.message?.takeIf { it.isNotBlank() } ?: e::class.java.simpleName
    }
}
