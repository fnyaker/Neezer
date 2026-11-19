package fr.neezer.mobile.feature.party

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent

/**
 * Handing the link out, the way a phone does it: the clipboard, or the system
 * share sheet.
 *
 * `PartySheet.svelte` copies to `navigator.clipboard` and calls `navigator.share`
 * when the browser has it, falling back to the copy when the person cancels —
 * this is the same two paths on Android, and the sheet's own rule is kept: a
 * cancelled share falls back to the copy only when the platform has no share
 * sheet at all, never when the person backed out.
 */
class AndroidPartyShare(private val context: Context) : PartyShare {
    override fun copy(text: String): Boolean = try {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        clipboard?.setPrimaryClip(ClipData.newPlainText(PartyCopy.INVITE_TITLE, text))
        clipboard != null
    } catch (_: Exception) {
        false
    }

    override fun share(title: String, text: String, url: String): Boolean {
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, title)
            putExtra(Intent.EXTRA_TEXT, "$text\n$url")
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        return try {
            context.startActivity(Intent.createChooser(intent, title).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        } catch (_: Exception) {
            false
        }
    }
}
