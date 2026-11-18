package fr.neezer.mobile.feature.onboarding

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import fr.neezer.mobile.bridge.NzBridge
import fr.neezer.mobile.bridge.NzNavigator
import fr.neezer.mobile.designsystem.EmptyState
import uniffi.nz_ffi.AppLink
import uniffi.nz_ffi.OnboardingStep
import uniffi.nz_ffi.Screens

/**
 * Bienvenue — lot MB5b (docs/agents/mobile.md, feature inventory).
 *
 * The screen itself is [OnboardingScreen], a pure function of the model's state;
 * this entry only wires it to the app: the core's screens, this lot's rules, and
 * the shell's route. It keeps the signature MB0 wrote, so `NavGraph.kt` never
 * changes.
 */
@Composable
fun OnboardingEntry(nav: NzNavigator, modifier: Modifier = Modifier) {
    OnboardingFlow(
        initialStep = null,
        onDone = { nav.go(AppLink.Home) },
        modifier = modifier,
    )
}

/**
 * Invitation — lot MB5b (docs/agents/mobile.md, feature inventory).
 *
 * The page is [InviteScreen]. Once the account is created, this entry shows the
 * ONBOARDING FLOW ITSELF: `NzNavigator` speaks `AppLink`s and MB2's enum has no
 * `Onboarding` link, so a route change is not available to this module — and a
 * lot writes only its own files. The `Onboarding` route still exists for a shell
 * that opens it directly (after a restore); both draw the same screen.
 */
@Composable
fun InviteEntry(code: String, nav: NzNavigator, modifier: Modifier = Modifier) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzInviteService) }
    val scope = rememberCoroutineScope()
    var flow by remember(code) { mutableStateOf<OnboardingStep?>(null) }
    val openApp: () -> Unit = { nav.go(AppLink.Home) }

    if (flow != null) {
        OnboardingFlow(
            initialStep = flow,
            onDone = openApp,
            modifier = modifier,
        )
        return
    }

    val model = remember(service, scope, code) {
        service?.let {
            InviteModel(
                service = it,
                scope = scope,
                code = code,
                onCreated = { step, _ ->
                    if (step == OnboardingStep.DONE) openApp() else flow = step
                },
                onOpenApp = openApp,
            )
        }
    }
    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }
    LaunchedEffect(model) { model.start() }
    DisposableEffect(model) { onDispose { model.leave() } }

    val state by model.state.collectAsState()
    val feedback = remember(state) {
        InviteFeedback(
            usernameError = state.fieldErrors[InviteModel.Field.Username]
                ?: if (state.submitted || state.touched.contains(InviteModel.Field.Username)) {
                    model.usernameError()
                } else {
                    ""
                },
            passwordError = state.fieldErrors[InviteModel.Field.Password]
                ?: if (state.submitted || state.touched.contains(InviteModel.Field.Password)) {
                    model.passwordError()
                } else {
                    ""
                },
            confirmError = state.fieldErrors[InviteModel.Field.Confirm]
                ?: if (state.submitted || state.touched.contains(InviteModel.Field.Confirm)) {
                    model.confirmError()
                } else {
                    ""
                },
            strength = model.strength(),
            score = model.score(),
            missing = model.missing(),
            rules = service?.rules ?: InviteFeedback().rules,
            expiry = model.expiryLabel(),
        )
    }
    InviteScreen(
        state = state,
        feedback = feedback,
        actions = InviteActions(
            username = model::onUsername,
            password = model::onPassword,
            confirm = model::onConfirm,
            blur = model::blur,
            submit = model::submit,
            toggleShowPassword = model::toggleShowPassword,
            retry = model::inspect,
            openApp = model::openApp,
            useInvitation = model::useInvitation,
        ),
        modifier = modifier,
    )
}

/** The onboarding flow itself: the artists step, then the account step. */
@Composable
private fun OnboardingFlow(
    initialStep: OnboardingStep?,
    onDone: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val core = NzBridge.core
    val screens: Screens? = remember(core) { core?.screens() }
    val service = remember(screens) { screens?.let(::NzOnboardingService) }
    val scope = rememberCoroutineScope()
    val model = remember(service, scope) {
        service?.let {
            OnboardingModel(
                service = it,
                rules = NzOnboardingRules,
                scope = scope,
                onDone = onDone,
            )
        }
    }
    if (model == null) {
        Column(modifier.fillMaxSize()) {
            EmptyState(
                title = "Le noyau de l'application n'est pas disponible",
                body = NzBridge.failure ?: "Redémarre l'application.",
            )
        }
        return
    }
    // The step the redeem named is painted from the visiting state before the
    // first read: an account created with "account" next must not flash the
    // artists page.
    LaunchedEffect(model, initialStep) {
        if (initialStep == OnboardingStep.ACCOUNT) model.gotoAccountForStart()
        model.start()
    }
    DisposableEffect(model) { onDispose { model.leave() } }

    val state by model.state.collectAsState()
    OnboardingScreen(
        state = state,
        actions = OnboardingActions(
            toggle = model::toggle,
            retryRelated = model::retryRelated,
            query = model::onQuery,
            submitSearch = model::submitSearch,
            retrySearch = model::retrySearch,
            clearSearch = model::clearSearch,
            more = model::loadMore,
            retry = model::start,
            retryFeed = model::retryFeed,
            pass = model::pass,
            finish = model::finish,
            retryAction = { state.actionError?.retry?.invoke() },
            loadOptions = model::loadOptions,
            open = model::open,
            closeForm = model::closeForm,
            link = model::link,
            startImport = model::startImport,
            saveImport = model::saveImport,
            retryImport = model::retryImport,
            refreshImport = model::refreshImport,
            accountDone = model::accountDone,
            skip = model::skip,
            unskip = model::unskip,
            gotoArtists = model::gotoArtists,
            dismissError = model::dismissActionError,
        ),
        modifier = modifier,
    )
}
