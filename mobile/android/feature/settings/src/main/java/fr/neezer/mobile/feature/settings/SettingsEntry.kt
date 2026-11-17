package fr.neezer.mobile.feature.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.Sillon
import fr.neezer.mobile.feature.admin.AdminSections
import fr.neezer.mobile.feature.audiofx.AudioFxSections
import uniffi.nz_ffi.nzVersion

/**
 * Réglages, the person's own sections (lot MB16) and, behind [isAdmin], the
 * server's (MB17).
 *
 * MB0 draws the section list and — this is the point of the skeleton — the Aide
 * section, which shows `nz_version()` READ FROM RUST. That one line proves the
 * whole chain end to end: Kotlin → JNA → `libnz_ffi.so` → the crate's version.
 * Every other section is a placeholder naming its lot.
 */
@Composable
fun SettingsEntry(
    section: String?,
    nav: NzNavigator,
    modifier: Modifier = Modifier,
    isAdmin: Boolean = false,
) {
    var current by remember(section) { mutableStateOf(section) }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = Sillon.space, vertical = Sillon.gap),
        verticalArrangement = Arrangement.spacedBy(Sillon.gap),
    ) {
        Text("Réglages", style = MaterialTheme.typography.headlineSmall, modifier = Modifier.padding(vertical = Sillon.gap))

        for (id in SECTIONS) {
            SectionRow(id = id, selected = current == id) { current = if (current == id) null else id }
        }
        if (isAdmin) {
            for (id in ADMIN_SECTIONS) {
                SectionRow(id = id, selected = current == id) { current = if (current == id) null else id }
            }
        }

        HorizontalDivider(color = MaterialTheme.colorScheme.outline)

        when (val open = current) {
            null -> Text(
                text = "Choisis une section.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(vertical = Sillon.gap),
            )

            "Aide" -> HelpSection()

            "Lecture & son" -> {
                AudioFxSections()
                SectionNote(
                    "Qualité, normalisation, fondu et silence sont décrits par le lot MB12 ; les autres réglages d'écoute arrivent avec le lot MB16.",
                )
            }

            "Serveur" -> {
                if (isAdmin) {
                    AdminSections()
                } else {
                    MissingSection(open)
                }
            }

            else -> MissingSection(open)
        }
    }
}

/** Aide (MB16): the two versions, and the diagnostic log's path. */
@Composable
private fun HelpSection() {
    // Read ONCE: the call crosses into Rust, and a recomposition must not.
    val rustVersion = remember { runCatching { nzVersion() }.getOrElse { "indisponible (${it.message})" } }
    val appVersion = remember { NzBridge.version ?: "indisponible" }

    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text("Aide", style = MaterialTheme.typography.titleMedium)
        Text(
            text = "Version de l'application : ${appVersion}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = "Version du noyau (Rust) : $rustVersion",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = "Le journal de diagnostic et l'export arrivent avec le lot MB16.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun MissingSection(id: String) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(id, style = MaterialTheme.typography.titleMedium)
        SectionNote("Cette section arrive avec son lot (docs/agents/mobile.md, inventaire des fonctionnalités).")
    }
}

@Composable
private fun SectionNote(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun SectionRow(id: String, selected: Boolean, onClick: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(vertical = 10.dp),
    ) {
        Text(
            text = id,
            style = MaterialTheme.typography.bodyLarge,
            color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
        )
        if (selected) {
            Text("ouvert", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** The SPA's section ids (`routes/Settings`), in its order. */
private val SECTIONS = listOf(
    "Lecture & son",
    "Apparence",
    "Animations",
    "Écoute",
    "Bibliothèque",
    "Téléchargements",
    "Découverte",
    "Sources",
    "Compte",
    "Aide",
)

/** MB17's sections: only an administrator sees them. */
private val ADMIN_SECTIONS = listOf(
    "Utilisateurs",
    "Serveur",
)
