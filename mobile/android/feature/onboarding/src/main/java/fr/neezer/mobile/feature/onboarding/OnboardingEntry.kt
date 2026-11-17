package fr.neezer.mobile.feature.onboarding

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.NzPlaceholder

/**
 * Bienvenue — lot MB5b (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB5b replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun OnboardingEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Bienvenue",
        body = "Les artistes à suivre, le lien de compte et l'import arrivent avec le lot MB5b.",
        modifier = modifier,
    )
}

/**
 * Invitation — lot MB5b (docs/agents/mobile.md, feature inventory).
 *
 * MB0 draws the route's name and what will live here; MB5b replaces this body
 * and keeps the signature, so `NavGraph.kt` never changes.
 */
@Composable
fun InviteEntry(code: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    NzPlaceholder(
        title = "Invitation",
        body = "Le code d'invitation est lu ici ; la page publique et l'inscription arrivent avec le lot MB5b.",
        modifier = modifier,
    )
}
