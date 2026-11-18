package fr.neezer.mobile.feature.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import fr.neezer.mobile.designsystem.Icon
import fr.neezer.mobile.designsystem.NzButton
import fr.neezer.mobile.designsystem.NzButtonKind
import fr.neezer.mobile.designsystem.NzGlyph
import fr.neezer.mobile.designsystem.NzMinTouchTarget
import fr.neezer.mobile.designsystem.NzRadius
import fr.neezer.mobile.designsystem.NzSheet
import fr.neezer.mobile.designsystem.NzSpacing
import fr.neezer.mobile.designsystem.Sillon
import uniffi.nz_ffi.AccessData

/**
 * `WhoPicker.svelte`: who may use one feature of one account.
 *
 * The SPA's own model — `*` is everyone, an empty set is nobody, and a set of
 * people and groups is exactly that — so what is saved here is what the server
 * already understands (`PUT /api/access/grants`'s `subjects`). Checking
 * *Tout le monde* clears the named set, because both at once would be two
 * answers to one question.
 */
@Composable
internal fun WhoSheet(
    target: GrantTarget,
    access: AccessData?,
    onDismiss: () -> Unit,
    onConfirm: (List<String>) -> Unit,
) {
    val colors = Sillon.colors
    var everyone by remember(target) { mutableStateOf(target.subjects.contains("*")) }
    var chosen by remember(target) {
        mutableStateOf(target.subjects.filter { it != "*" })
    }
    NzSheet(visible = true, onDismiss = onDismiss) {
        Text(
            text = "Qui peut utiliser « ${target.featureLabel} » ?",
            style = Sillon.type.titleSmall,
            color = colors.tx,
        )
        Text(
            text = "Sans personne, ce service reste là pour ce qui est déjà archivé, mais il n'est plus interrogé pour les autres.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
        ChoiceRow(
            label = "Tout le monde",
            selected = everyone,
            onClick = {
                everyone = true
                chosen = emptyList()
            },
        )
        val people = access?.users.orEmpty()
        val groups = access?.groups.orEmpty()
        if (people.isEmpty() && groups.isEmpty()) {
            Text(
                text = "Aucune personne à nommer pour l'instant.",
                style = Sillon.type.bodySmall,
                color = colors.dim,
            )
        }
        LazyColumn(Modifier.fillMaxWidth().height(260.dp)) {
            items(people.size + groups.size) { index ->
                val isGroup = index >= people.size
                val id = if (isGroup) groups[index - people.size].id else people[index].id
                val name = if (isGroup) groups[index - people.size].name else people[index].name
                ChoiceRow(
                    label = name,
                    detail = if (isGroup) "Groupe" else null,
                    selected = !everyone && id in chosen,
                    enabled = !everyone,
                    onClick = {
                        chosen = if (id in chosen) chosen - id else chosen + id
                    },
                )
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(NzSpacing.s)) {
            NzButton(
                text = "Enregistrer",
                onClick = { onConfirm(if (everyone) listOf("*") else chosen) },
            )
            NzButton(text = "Annuler", onClick = onDismiss, kind = NzButtonKind.Ghost)
        }
        Text(
            text = "L'accès que vous donnez vaut pour ce compte : la personne utilise alors VOTRE compte, jamais le sien.",
            style = Sillon.type.bodySmall,
            color = colors.dim,
        )
    }
}

/** One line of the picker: a box, a name, and the kind when it is a group. */
@Composable
private fun ChoiceRow(
    label: String,
    selected: Boolean,
    enabled: Boolean = true,
    detail: String? = null,
    onClick: () -> Unit,
) {
    val colors = Sillon.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(NzMinTouchTarget)
            .clickable(enabled = enabled, role = Role.Checkbox, onClick = onClick)
            .semantics { contentDescription = label },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NzSpacing.m),
    ) {
        Box(
            Modifier
                .size(22.dp)
                .background(
                    if (selected) colors.ac else androidx.compose.ui.graphics.Color.Transparent,
                    RoundedCornerShape(6.dp),
                )
                .then(
                    if (selected) Modifier else Modifier.background(colors.s2, RoundedCornerShape(6.dp)),
                ),
            contentAlignment = Alignment.Center,
        ) {
            if (selected) Icon(NzGlyph.Check, size = 14.dp, tint = colors.onac)
        }
        Text(label, style = Sillon.type.body, color = if (enabled) colors.tx else colors.dim)
        if (detail != null) {
            Text(detail, style = Sillon.type.labelMono, color = colors.dim)
        }
    }
}

/** The sheet's own circle, kept for a caller that wants the picker's glyph. */
@Composable
internal fun WhoGlyph() {
    Box(
        Modifier
            .size(24.dp)
            .padding(2.dp)
            .background(Sillon.colors.s2, CircleShape),
    )
}
