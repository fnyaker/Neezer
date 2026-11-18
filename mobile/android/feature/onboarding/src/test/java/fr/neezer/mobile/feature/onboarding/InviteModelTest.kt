package fr.neezer.mobile.feature.onboarding

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.nz_ffi.InviteField
import uniffi.nz_ffi.NzException
import uniffi.nz_ffi.OnboardingStep

/**
 * The invitation page's state machine, on the JVM over its port.
 *
 * The page's own rules: a malformed token costs no request, a gone link is the
 * invalid state (no retry), the form validates before sending, and the server's
 * refusal goes under the field it names.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class InviteModelTest {
    private fun TestScope.model(
        service: FakeInviteService = FakeInviteService(),
        online: Boolean = true,
        created: (OnboardingStep, String) -> Unit = { _, _ -> },
        openApp: () -> Unit = {},
    ) = InviteModel(
        service = service,
        scope = backgroundScope,
        code = OnboardingFixtures.token(),
        online = { online },
        describe = { it.message ?: "erreur" },
        onCreated = created,
        onOpenApp = openApp,
    )

    @Test
    fun a_malformed_token_costs_no_request_and_says_it() = runTest {
        val service = FakeInviteService()
        service.tokenOk = false
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(InviteModel.Phase.Invalid, model.state.value.phase)
        assertTrue(service.calls.isEmpty())
    }

    @Test
    fun a_gone_link_is_the_invalid_state_without_a_retry() = runTest {
        val service = FakeInviteService()
        service.info = OnboardingFixtures.invite(valid = false)
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(InviteModel.Phase.Invalid, model.state.value.phase)
        assertNull(model.state.value.failure)
    }

    @Test
    fun a_link_the_server_does_not_know_is_invalid_too() = runTest {
        val service = FakeInviteService()
        service.inspectError = NzException.Server(status = 404u, detail = "gone")
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(InviteModel.Phase.Invalid, model.state.value.phase)
    }

    @Test
    fun a_failed_check_names_its_cause_and_retries() = runTest {
        val service = FakeInviteService()
        service.inspectError = NzException.Server(status = 500u, detail = "boom")
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(InviteModel.Phase.Error, model.state.value.phase)
        assertEquals("Impossible de vérifier l'invitation", model.state.value.failureTitle)
        assertTrue(model.state.value.failure!!.contains("Ton lien n'est pas en cause"))
    }

    @Test
    fun a_rate_limited_check_says_so() = runTest {
        val service = FakeInviteService()
        service.inspectError = NzException.NotReady(retryAfterS = 60u)
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals("Trop de tentatives", model.state.value.failureTitle)
    }

    @Test
    fun a_valid_link_opens_the_form_with_the_drafts_name() = runTest {
        val service = FakeInviteService()
        service.info = OnboardingFixtures.invite(draftName = "elodie")
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals(InviteModel.Phase.Ready, model.state.value.phase)
        assertEquals("elodie", model.state.value.username)
        assertEquals("Invitation", model.state.value.label)
        assertEquals("Expire le 21 novembre 2026 à 18:00.", model.expiryLabel())
    }

    @Test
    fun typing_a_name_folds_it_without_touching_the_edges() = runTest {
        val model = model()
        model.start()
        runCurrent()
        model.onUsername(" Élodie ")
        assertEquals(" elodie ", model.state.value.username)
    }

    @Test
    fun the_form_validates_before_sending() = runTest {
        val service = FakeInviteService()
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.onUsername("elodie")
        model.onPassword("court")
        model.onConfirm("court")
        model.submit()
        runCurrent()
        assertTrue(service.calls.isEmpty())
        assertEquals("12 caractères minimum.", model.state.value.fieldErrors[InviteModel.Field.Password])
    }

    @Test
    fun the_two_passwords_must_match() = runTest {
        val service = FakeInviteService()
        val model = model(service)
        model.start()
        runCurrent()
        service.calls.clear()
        model.onUsername("elodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une autre phrase de passe")
        assertEquals("Les deux mots de passe ne correspondent pas.", model.confirmError())
        model.submit()
        runCurrent()
        assertTrue(service.calls.isEmpty())
    }

    @Test
    fun a_valid_form_sends_the_folded_name_and_lands_where_the_account_says() = runTest {
        val service = FakeInviteService()
        service.answer = OnboardingFixtures.created(OnboardingStep.ACCOUNT)
        var landed: Pair<OnboardingStep, String>? = null
        val model = model(service, created = { step, name -> landed = step to name })
        model.start()
        runCurrent()
        service.calls.clear()
        model.onUsername("Élodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une phrase de passe simple")
        model.submit()
        runCurrent()
        assertEquals(listOf("redeem:elodie"), service.calls)
        assertEquals(InviteModel.Phase.Created, model.state.value.phase)
        assertEquals(OnboardingStep.ACCOUNT to "elodie", landed)
    }

    @Test
    fun a_refusal_goes_under_the_field_the_server_names() = runTest {
        val service = FakeInviteService()
        service.answer = OnboardingFixtures.fieldAnswer(
            InviteField.USERNAME,
            "Ce nom d'utilisateur n'est pas disponible.",
        )
        val model = model(service)
        model.start()
        runCurrent()
        model.onUsername("elodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une phrase de passe simple")
        model.submit()
        runCurrent()
        assertEquals(
            "Ce nom d'utilisateur n'est pas disponible.",
            model.state.value.fieldErrors[InviteModel.Field.Username],
        )
        assertEquals(InviteModel.Phase.Ready, model.state.value.phase)
        assertFalse(model.state.value.busy)
    }

    @Test
    fun a_link_that_vanishes_at_redeem_switches_to_invalid() = runTest {
        val service = FakeInviteService()
        service.answer = OnboardingFixtures.invalidAnswer()
        val model = model(service)
        model.start()
        runCurrent()
        model.onUsername("elodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une phrase de passe simple")
        model.submit()
        runCurrent()
        assertEquals(InviteModel.Phase.Invalid, model.state.value.phase)
    }

    @Test
    fun an_interrupted_connection_keeps_the_link_and_offers_retry() = runTest {
        val service = FakeInviteService()
        service.redeemError = NzException.Offline()
        val model = model(service)
        model.start()
        runCurrent()
        model.onUsername("elodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une phrase de passe simple")
        model.submit()
        runCurrent()
        assertTrue(model.state.value.pendingRetry)
        assertTrue(model.state.value.formError.contains("ton lien reste valable"))
        assertEquals(InviteModel.Phase.Ready, model.state.value.phase)
    }

    @Test
    fun a_rate_limited_redeem_says_the_wait() = runTest {
        val service = FakeInviteService()
        service.redeemError = NzException.NotReady(retryAfterS = 30u)
        val model = model(service)
        model.start()
        runCurrent()
        model.onUsername("elodie")
        model.onPassword("une phrase de passe simple")
        model.onConfirm("une phrase de passe simple")
        model.submit()
        runCurrent()
        assertTrue(model.state.value.formError.contains("Trop de tentatives"))
        assertFalse(model.state.value.pendingRetry)
    }

    @Test
    fun a_signed_in_person_is_told_and_can_use_the_invitation() = runTest {
        val service = FakeInviteService()
        service.account = OnboardingFixtures.me("nadia")
        val model = model(service)
        model.start()
        runCurrent()
        assertEquals("nadia", model.state.value.accountName)
        service.calls.clear()
        model.submit()
        runCurrent()
        assertTrue(service.calls.isEmpty())
        model.useInvitation()
        runCurrent()
        assertTrue(service.calls.contains("logout"))
        assertNull(model.state.value.accountName)
    }

    @Test
    fun a_failed_logout_keeps_the_session_open_and_says_it() = runTest {
        val service = FakeInviteService()
        service.account = OnboardingFixtures.me("nadia")
        service.logoutError = IllegalStateException("Hors ligne")
        val model = model(service)
        model.start()
        runCurrent()
        model.useInvitation()
        runCurrent()
        assertTrue(model.state.value.leaveError.contains("reste ouverte"))
        assertEquals("nadia", model.state.value.accountName)
    }

    @Test
    fun the_password_meter_follows_the_cores_rules() = runTest {
        val model = model()
        model.start()
        runCurrent()
        model.onPassword("court")
        assertEquals(1, model.score())
        assertEquals(7, model.missing())
        model.onPassword("abcdefghijklmnopqrstuvwx")
        assertEquals(4, model.score())
        assertEquals(0, model.missing())
        assertNotNull(OnboardingCopy.strengthLabel(model.strength()))
    }

    @Test
    fun a_link_with_no_time_says_nothing() {
        assertEquals("", InviteModel.expiryLabel(null))
        assertEquals("", InviteModel.expiryLabel(""))
        assertEquals("", InviteModel.expiryLabel("pas une date"))
    }
}
