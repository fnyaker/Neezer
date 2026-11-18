package fr.neezer.mobile.feature.admin

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import fr.neezer.mobile.bridge.NzErrors
import fr.neezer.mobile.bridge.ScreenState
import fr.neezer.mobile.bridge.screenFlow
import fr.neezer.mobile.designsystem.NzToasts
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nz_ffi.AdminAnalysisJob
import uniffi.nz_ffi.AdminArchiveStatus
import uniffi.nz_ffi.AdminAutomatch
import uniffi.nz_ffi.AdminCleanup
import uniffi.nz_ffi.AdminCredits
import uniffi.nz_ffi.AdminEnrich
import uniffi.nz_ffi.AdminInviteRow
import uniffi.nz_ffi.AdminMigrate
import uniffi.nz_ffi.AdminPlan
import uniffi.nz_ffi.AdminRules
import uniffi.nz_ffi.AdminServerCard
import uniffi.nz_ffi.AdminSimilar
import uniffi.nz_ffi.AdminStorage
import uniffi.nz_ffi.AdminTranscribe
import uniffi.nz_ffi.AdminUserDetail
import uniffi.nz_ffi.AdminUserRow
import uniffi.nz_ffi.AdminUsersPage

/**
 * L'état des pages d'administration (lot MB17).
 *
 * Chaque règle que la SPA a apprise, dans l'ordre où la personne la rencontre,
 * en Kotlin ordinaire au-dessus d'[AdminService] — `AdminModelTest` la prouve
 * sur la JVM avec un faux, sans JNA, sans `.so`, sans appareil.
 *
 * - **Un membre n'obtient rien et ne demande rien.** `/api/me`'s `user.admin`
 *   est lu de la session (zéro requête) ; sans lui, [open] rend la main
 *   immédiatement et l'écran dit « réservé aux administrateurs ». Douze `403`
 *   ne sont pas une erreur à afficher, c'est une entrée à ne pas dessiner.
 * - **Chaque carte se charge par `screenFlow`** : la copie de l'appareil peint
 *   tout de suite, le réseau la remplace, un lien mort garde la copie et dit
 *   « Hors ligne · version mémorisée », et sans copie la cause et « Réessayer ».
 * - **Un clic répond dans l'image.** Un rôle, un compte désactivé ou une
 *   invitation révoquée changent [state] AVANT la requête ; un refus remet l'état
 *   d'avant et la phrase du serveur dans [State.actionError].
 * - **Un travail long dit où il en est** (règle 2) : le lancement rend la main,
 *   la carte interroge le serveur **uniquement tant qu'il dit `running`**, et
 *   chaque échec porte sa CAUSE (`AdminJobFailures`). Aucune boucle ne tourne
 *   pour un travail fini.
 * - **Une écriture est UNE requête** : une règle, un quota, un champ de
 *   transcription à la fois — c'est le contrôle qui est une décision, pas l'écran.
 *
 * Les nombres auxquels ce lot est tenu : *Utilisateurs* froid = les 2 requêtes
 * de la SPA, *Serveur* froid = ses 12, et une ouverture chaude = ZÉRO.
 */
class AdminModel(
    private val service: AdminService,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val toast: (String) -> Unit = { NzToasts.show(it) },
    private val clock: () -> Long = { System.currentTimeMillis() },
    /** `AdminUsers.svelte`'s 300 ms: a thumb typing a name is one search. */
    private val searchDelayMs: Long = 300,
) {
    /** One card's four states (rule 3): the copy, the fresh answer, the cause. */
    data class Section<T>(
        val data: T? = null,
        val loading: Boolean = false,
        val stale: Boolean = false,
        val failure: String? = null,
    ) {
        val ready: Boolean get() = data != null && failure == null
    }

    /** What the account sheet asked to do, and what is open over the table. */
    sealed interface Sheet {
        /** *Inviter quelqu'un* / *Renvoyer l'invitation*. */
        data class Invite(val source: AdminInviteRow? = null) : Sheet

        /** The destructive confirmation, with the account's name to type. */
        data class Confirm(val action: ConfirmAction, val user: AdminUserRow) : Sheet
    }

    data class State(
        val section: String? = null,
        /** `null` = not read yet; `false` = a member (no request, no entry). */
        val admin: Boolean? = null,
        val adminName: String = "",
        /** The field's text — it moves on every keystroke, the request does not. */
        val query: String = "",
        /** The term the SERVER was actually asked for (debounced). */
        val term: String = "",
        val filter: AccountFilter = AccountFilter.All,
        val rows: List<AccountRow> = emptyList(),
        val total: Int = 0,
        val next: String? = null,
        val usersLoading: Boolean = false,
        val usersStale: Boolean = false,
        val usersFailure: String? = null,
        val moreLoading: Boolean = false,
        val moreFailure: String? = null,
        val invites: List<AdminInviteRow> = emptyList(),
        val invitesLoaded: Boolean = false,
        val invitesLoading: Boolean = false,
        val invitesFailure: String? = null,
        val refreshedAt: Long? = null,
        /** The server page's eight cards (`Settings.svelte`'s own reads). */
        val server: Section<AdminServerCard> = Section(),
        val storage: Section<AdminStorage> = Section(),
        val rules: Section<AdminRules> = Section(),
        val cleanup: Section<AdminCleanup> = Section(),
        val archive: Section<AdminArchiveStatus> = Section(),
        val migrate: Section<AdminMigrate> = Section(),
        val plan: Section<AdminPlan> = Section(),
        val analysis: Section<AdminAnalysisJob> = Section(),
        val enrich: Section<AdminEnrich> = Section(),
        val credits: Section<AdminCredits> = Section(),
        val similar: Section<AdminSimilar> = Section(),
        val automatch: Section<AdminAutomatch> = Section(),
        val transcribe: Section<AdminTranscribe> = Section(),
        /** The quota's draft, in the field, before it is saved. */
        val quotaDraft: String = "",
        val deezerChecking: Boolean = false,
        /** The account sheet: whose it is, and what it holds. */
        val openUser: String? = null,
        val userDetail: Section<AdminUserDetail> = Section(),
        val sheet: Sheet? = null,
        /** The freshly minted link, which exists only in this answer. */
        val issuedUrl: String? = null,
        val issuedToken: String? = null,
        val actionError: String? = null,
        /** One write is in flight: a second tap on the same control is ignored. */
        val busy: Boolean = false,
    ) {
        /** What the table draws: the filter over the rows already loaded. */
        val visible: List<AccountRow>
            get() = filterRows(rows, filter, term)

        val visibleUsers: List<AdminUserRow> get() = visible.users()

        val visibleInvites: List<AdminInviteRow> get() = visible.invites()

        val allLoaded: Boolean get() = rows.users().size >= total

        val hasFilters: Boolean get() = filter != AccountFilter.All || term.isNotEmpty()

        /** *Charger plus* exists while the server has more AND the cap is not hit. */
        val showMore: Boolean
            get() = next != null && filter != AccountFilter.Invited && rows.users().size < AdminCopy.MAX_ROWS

        val capped: Boolean get() = next != null && rows.users().size >= AdminCopy.MAX_ROWS

        /** The count line above the table (`AdminUsers.svelte#countLine`). */
        fun countLine(): String = when {
            usersLoading && rows.isEmpty() -> "Chargement des comptes…"
            term.isNotEmpty() -> {
                val matching = filterRows(invites.map { AccountRow.Invite(it) }, AccountFilter.Invited, term).size
                val accounts = AdminCopy.countLabel(total, "compte") + " correspondant" +
                    (if (total > 1) "s" else "")
                if (matching > 0) {
                    accounts + " · " + AdminCopy.countLabel(matching, "invitation") +
                        " correspondante" + (if (matching > 1) "s" else "")
                } else {
                    accounts
                }
            }
            else -> AdminCopy.countLabel(total, "compte") + " · " +
                AdminCopy.countLabel(invites.size, "invitation") + " en attente"
        }

        /** The pastille's counters; `null` means "not known yet". */
        fun chipCount(filter: AccountFilter): Int? = when (filter) {
            AccountFilter.All -> total + if (term.isNotEmpty()) {
                filterRows(invites.map { AccountRow.Invite(it) }, AccountFilter.Invited, term).size
            } else {
                invites.size
            }
            AccountFilter.Invited ->
                filterRows(invites.map { AccountRow.Invite(it) }, AccountFilter.Invited, term).size
            AccountFilter.Active -> if (allLoaded) rows.users().count { !it.disabled } else null
            AccountFilter.Admins -> if (allLoaded) rows.users().count { it.admin } else null
            AccountFilter.Members -> if (allLoaded) rows.users().count { !it.admin } else null
            AccountFilter.Disabled -> if (allLoaded) rows.users().count { it.disabled } else null
        }

        /** The range line under the table. */
        fun rangeLine(): String = when {
            filter == AccountFilter.Invited -> AdminCopy.countLabel(visibleInvites.size, "invitation")
            filter == AccountFilter.All -> "${rows.users().size} sur $total"
            allLoaded -> "${visible.size} affiché" + (if (visible.size > 1) "s" else "") + " sur $total"
            else -> "${visible.size} affiché" + (if (visible.size > 1) "s" else "") +
                " · ${rows.users().size} sur $total chargés"
        }

        /** The disk gauge's filled share, for *Stockage serveur*. */
        val diskUsedShare: Float
            get() {
                val store = storage.data ?: return 0f
                if (store.diskTotal == 0uL) return 0f
                return (((store.diskTotal - store.diskFree).toDouble() / store.diskTotal.toDouble()) * 100)
                    .coerceIn(0.0, 100.0).toFloat() / 100f
            }
    }

    var state by mutableStateOf(State())
        private set

    private var load: Job? = null
    private var search: Job? = null
    private val polls = mutableSetOf<String>()

    // --- opening ------------------------------------------------------------

    /**
     * Who is signed in, read once from the session (no request): the ONE gate.
     * Called before anything else, so a member's screen never opens a socket.
     */
    fun start() {
        if (state.admin != null) return
        val identity = runCatching { service.identity() }.getOrNull()
        state = state.copy(admin = identity?.admin ?: false, adminName = identity?.name.orEmpty())
    }

    /**
     * A category is opened: *Utilisateurs* or *Serveur*. A member gets nothing
     * and asks nothing (the route can be reached by a deep link too).
     */
    fun open(section: String?) {
        start()
        if (state.section == section) return
        load?.cancel()
        state = state.copy(section = section, actionError = null)
        if (section == null || state.admin != true) return
        when (section) {
            SECTION_USERS -> loadUsers()
            SECTION_SERVER -> loadServer()
        }
    }

    /** A short confirmation the SCREEN itself produced (a link copied). */
    fun notify(text: String) {
        toast(text)
    }

    /** The shell draws only these two pages; anything else is not an admin page. */
    fun isAdminPage(section: String?): Boolean = section == SECTION_USERS || section == SECTION_SERVER

    // --- *Utilisateurs* -----------------------------------------------------

    private fun loadUsers() {
        load = scope.launch {
            launch {
                // The account page first: it also brings the count line.
                screenFlow(
                    cached = { withContext(io) { service.cachedUsers() } },
                    fresh = { withContext(io) { service.users(state.term, null, AdminCopy.PAGE_SIZE.toUInt()) } },
                ).collect { screen -> applyUsers(screen) }
            }
            launch {
                screenFlow(
                    cached = { withContext(io) { service.cachedInvites() } },
                    fresh = { withContext(io) { service.invites("pending") } },
                ).collect { screen -> applyInvites(screen) }
            }
        }
    }

    private fun applyUsers(screen: ScreenState<AdminUsersPage>) {
        when (screen) {
            is ScreenState.Loading -> state = state.copy(usersLoading = true, usersFailure = null)
            is ScreenState.Ready -> {
                val page = screen.data
                val merged = mergeRows(emptyList(), page)
                state = state.copy(
                    rows = merged.map { AccountRow.User(it) } + state.invites.map { AccountRow.Invite(it) },
                    total = page.total.toInt(),
                    next = page.next,
                    usersLoading = false,
                    usersStale = screen.stale,
                    usersFailure = null,
                    refreshedAt = clock(),
                )
            }
            is ScreenState.Failed -> state = state.copy(
                usersLoading = false,
                usersFailure = screen.cause,
            )
        }
    }

    private fun applyInvites(screen: ScreenState<List<AdminInviteRow>>) {
        when (screen) {
            is ScreenState.Loading -> state = state.copy(invitesLoading = true)
            is ScreenState.Ready -> state = state.copy(
                invites = screen.data,
                invitesLoaded = true,
                invitesLoading = false,
                invitesFailure = null,
                rows = state.rows.users().map { AccountRow.User(it) } + screen.data.map { AccountRow.Invite(it) },
            )
            is ScreenState.Failed -> state = state.copy(
                invitesLoading = false,
                invitesFailure = screen.cause,
            )
        }
    }

    /**
     * *Réessayer* on the table: the SAME read again, one attempt per tap (never a
     * loop), and the page it already had stays on screen while it runs.
     */
    fun retryUsers() {
        reloadUsers(state.term)
    }

    /** *Réessayer* on the invitations banner: one attempt. */
    fun retryInvites() {
        reloadInvites()
    }

    /** *Réessayer* on a server card: that card alone, never the whole page. */
    fun retryCard(card: String) {
        scope.launch {
            val section = when (card) {
                "server" -> withContext(io) { runCatching { service.serverCard() } }.map { state = state.copy(server = Section(it)) }
                "storage" -> withContext(io) { runCatching { service.storage() } }.map { state = state.copy(storage = Section(it)) }
                "rules" -> withContext(io) { runCatching { service.rules() } }.map { state = state.copy(rules = Section(it)) }
                "cleanup" -> withContext(io) { runCatching { service.cleanup() } }.map { state = state.copy(cleanup = Section(it)) }
                "archive" -> withContext(io) { runCatching { service.archiveStatus() } }.map { state = state.copy(archive = Section(it)) }
                "migrate" -> withContext(io) { runCatching { service.migrate() } }.map { state = state.copy(migrate = Section(it)) }
                "plan" -> withContext(io) { runCatching { service.plan() } }.map { state = state.copy(plan = Section(it)) }
                "analysis" -> withContext(io) { runCatching { service.analysis() } }.map { state = state.copy(analysis = Section(it)) }
                "enrich" -> withContext(io) { runCatching { service.enrich() } }.map { state = state.copy(enrich = Section(it)) }
                "credits" -> withContext(io) { runCatching { service.credits() } }.map { state = state.copy(credits = Section(it)) }
                "similar" -> withContext(io) { runCatching { service.similar() } }.map { state = state.copy(similar = Section(it)) }
                "automatch" -> withContext(io) { runCatching { service.automatch() } }.map { state = state.copy(automatch = Section(it)) }
                "transcribe" -> withContext(io) { runCatching { service.transcribe(false) } }.map { state = state.copy(transcribe = Section(it)) }
                else -> null
            }
            section?.let { answer ->
                answer.onFailure { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) }
            }
        }
    }

    /**
     * The search field: the text moves at once, the request waits 300 ms and
     * needs two characters — a thumb typing "Léa" is ONE search, not three
     * (`AdminUsers.svelte#scheduleSearch`).
     */
    fun setQuery(text: String) {
        state = state.copy(query = text)
        search?.cancel()
        val term = text.trim()
        if (term.isEmpty()) {
            if (state.term.isNotEmpty()) reloadUsers("")
            return
        }
        if (term.length < 2 || term == state.term) return
        search = scope.launch {
            delay(searchDelayMs)
            reloadUsers(term)
        }
    }

    /** *Entrée* on a hardware keyboard: the same search, without the wait. */
    fun submitQuery() {
        search?.cancel()
        val term = state.query.trim()
        if (term.length < 2 || term == state.term) return
        reloadUsers(term)
    }

    fun clearQuery() {
        state = state.copy(query = "")
        search?.cancel()
        if (state.term.isNotEmpty()) reloadUsers("")
    }

    private fun reloadUsers(term: String) {
        if (state.admin != true) return
        state = state.copy(term = term, usersLoading = state.rows.isEmpty())
        load = scope.launch {
            screenFlow(
                cached = { null },
                fresh = { withContext(io) { service.users(term, null, AdminCopy.PAGE_SIZE.toUInt()) } },
            ).collect { screen -> applyUsers(screen) }
        }
    }

    /** *Charger plus* — one page more, from the cursor the last one answered. */
    fun loadMore() {
        val cursor = state.next ?: return
        if (state.moreLoading || state.rows.users().size >= AdminCopy.MAX_ROWS) return
        state = state.copy(moreLoading = true, moreFailure = null)
        scope.launch {
            val answer = withContext(io) {
                runCatching { service.users(state.term, cursor, AdminCopy.PAGE_SIZE.toUInt()) }
            }
            answer.fold(
                onSuccess = { page ->
                    val users = mergeRows(state.rows.users(), page)
                    state = state.copy(
                        rows = users.map { AccountRow.User(it) } + state.invites.map { AccountRow.Invite(it) },
                        total = page.total.toInt(),
                        next = page.next,
                        moreLoading = false,
                        refreshedAt = clock(),
                    )
                },
                onFailure = { error ->
                    state = state.copy(moreLoading = false, moreFailure = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    /** *Rafraîchir la liste* — the same page size the person already loaded. */
    fun refresh() {
        if (state.busy) return
        val limit = state.rows.users().size.coerceIn(AdminCopy.PAGE_SIZE, 200).toUInt()
        state = state.copy(usersLoading = state.rows.isEmpty(), usersFailure = null)
        scope.launch {
            val answer = withContext(io) {
                runCatching { service.users(state.term, null, limit) }
            }
            answer.fold(
                onSuccess = { page ->
                    val users = mergeRows(emptyList(), page)
                    state = state.copy(
                        rows = users.map { AccountRow.User(it) } + state.invites.map { AccountRow.Invite(it) },
                        total = page.total.toInt(),
                        next = page.next,
                        usersLoading = false,
                        refreshedAt = clock(),
                    )
                },
                onFailure = { error ->
                    state = state.copy(
                        usersLoading = false,
                        usersFailure = if (state.rows.isEmpty()) AdminCopy.guardMessage(error) else null,
                        actionError = if (state.rows.isEmpty()) null else AdminCopy.guardMessage(error),
                    )
                },
            )
            reloadInvites()
        }
    }

    private fun reloadInvites() {
        state = state.copy(invitesLoading = true, invitesFailure = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.invites("pending") } }
            answer.fold(
                onSuccess = { invites ->
                    state = state.copy(
                        invites = invites,
                        invitesLoaded = true,
                        invitesLoading = false,
                        rows = state.rows.users().map { AccountRow.User(it) } + invites.map { AccountRow.Invite(it) },
                    )
                },
                onFailure = { error ->
                    state = state.copy(invitesLoading = false, invitesFailure = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    fun setFilter(filter: AccountFilter) {
        if (state.filter == filter) return
        state = state.copy(filter = filter)
    }

    fun clearFilters() {
        state = state.copy(filter = AccountFilter.All, query = "")
        if (state.term.isNotEmpty()) reloadUsers("")
    }

    // --- one account --------------------------------------------------------

    /** A row is tapped: the sheet opens at once, its detail follows. */
    fun openUser(user: AdminUserRow) {
        state = state.copy(openUser = user.id, actionError = null, userDetail = Section(loading = true))
        scope.launch {
            val answer = withContext(io) { runCatching { service.user(user.id) } }
            answer.fold(
                onSuccess = { detail -> state = state.copy(userDetail = Section(detail)) },
                onFailure = { error ->
                    state = state.copy(userDetail = Section(failure = AdminCopy.guardMessage(error)))
                },
            )
        }
    }

    fun closeSheet() {
        state = state.copy(openUser = null, userDetail = Section(), sheet = null, actionError = null)
    }

    /**
     * *Nommer administrateur* / *Retirer les droits*: the row moves in the same
     * frame, then one `PATCH`; a refusal puts the row back with the server's own
     * sentence (`last_admin`, `self_disable`).
     */
    fun setRole(user: AdminUserRow, on: Boolean) = optimistic(user, { it.copy(admin = on) }) {
        service.setRole(user.id, on).also {
            toast(if (on) "Droits d'administrateur accordés" else "Droits d'administrateur retirés")
        }
    }

    /** *Désactiver* / *Réactiver* — the person is disconnected everywhere. */
    fun setDisabled(user: AdminUserRow, on: Boolean) = optimistic(user, { it.copy(disabled = on) }) {
        service.setDisabled(user.id, on).also {
            toast(if (on) "Compte désactivé" else "Compte réactivé")
        }
    }

    private fun optimistic(
        user: AdminUserRow,
        preview: (AdminUserRow) -> AdminUserRow,
        write: suspend () -> AdminUserRow,
    ) {
        if (state.busy) return
        val before = state.rows
        replaceRow(user.id, preview(user))
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { write() } }
            answer.fold(
                onSuccess = { fresh -> replaceRow(user.id, fresh) },
                onFailure = { error ->
                    state = state.copy(rows = before, actionError = AdminCopy.guardMessage(error))
                },
            )
            state = state.copy(busy = false)
        }
    }

    /** The confirmation the destructive actions need, before anything leaves. */
    fun askDelete(user: AdminUserRow) {
        state = state.copy(sheet = Sheet.Confirm(ConfirmAction.Delete, user), actionError = null)
        openUser(user)
    }

    fun askRevoke(invite: AdminInviteRow) {
        state = state.copy(
            sheet = Sheet.Confirm(ConfirmAction.Revoke, inviteAsUser(invite)),
            actionError = null,
        )
    }

    /**
     * `DELETE /api/admin/users/<id>` — the account's NAME confirms it (the
     * server refuses anything else). The archived audio stays; the files this
     * person uploaded move to the admin.
     */
    fun confirmDelete(user: AdminUserRow, typed: String) {
        if (state.busy || typed.trim() != user.name) return
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.deleteUser(user.id, user.name) } }
            answer.fold(
                onSuccess = { result ->
                    state = state.copy(
                        rows = state.rows.filterNot { it is AccountRow.User && it.row.id == user.id },
                        total = (state.total - 1).coerceAtLeast(0),
                        sheet = null,
                        openUser = null,
                        busy = false,
                    )
                    val moved = buildList {
                        if (result.uploadsReassigned > 0u) {
                            add(AdminCopy.countLabel(result.uploadsReassigned.toInt(), "fichier") + " réattribué" +
                                if (result.uploadsReassigned > 1u) "s" else "")
                        }
                        if (result.podcastsReassigned > 0u) {
                            add(AdminCopy.countLabel(result.podcastsReassigned.toInt(), "émission") + " réattribuée" +
                                if (result.podcastsReassigned > 1u) "s" else "")
                        }
                    }
                    toast(if (moved.isEmpty()) "Compte supprimé" else "Compte supprimé · " + moved.joinToString(" · "))
                },
                onFailure = { error ->
                    state = state.copy(busy = false, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    /** *Révoquer l'invitation*: the row leaves the table at once. */
    fun confirmRevoke(invite: AdminInviteRow) {
        if (state.busy) return
        val before = state.rows
        dropInvite(invite.id)
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.revokeInvite(invite.id) } }
            answer.fold(
                onSuccess = {
                    state = state.copy(busy = false, sheet = null)
                    toast("Invitation révoquée")
                },
                onFailure = { error ->
                    // `invite_used`/`invite_not_found`: the row is not revocable
                    // any more, so it must not come back.
                    val code = AdminCopy.guardMessage(error)
                    state = state.copy(
                        busy = false,
                        sheet = null,
                        rows = if (code.contains("déjà servi") || code.contains("n'existe plus")) {
                            state.rows
                        } else {
                            before
                        },
                        actionError = code,
                    )
                },
            )
        }
    }

    // --- the invitation sheet ------------------------------------------------

    fun openInvite(source: AdminInviteRow? = null) {
        state = state.copy(sheet = Sheet.Invite(source), actionError = null, issuedUrl = null)
    }

    /**
     * *Créer le lien* / *Créer un nouveau lien*: `POST /admin/invites` (or its
     * `renew`). The answer carries the token ONCE — it is handed to the sheet,
     * and the row joins the table.
     */
    fun createInvite(note: String, expiresIn: UInt, source: AdminInviteRow? = null) {
        if (state.busy) return
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) {
                runCatching {
                    if (source != null) service.renewInvite(source.id)
                    else service.createInvite(note.trim().takeIf { it.isNotEmpty() }, expiresIn)
                }
            }
            answer.fold(
                onSuccess = { issued ->
                    val invites = state.invites.filterNot { it.id == issued.invite.id || it.id == source?.id }
                    state = state.copy(
                        busy = false,
                        issuedUrl = issued.url,
                        issuedToken = issued.token,
                        invites = listOf(issued.invite) + invites,
                        invitesLoaded = true,
                        rows = state.rows.users().map { AccountRow.User(it) } +
                            (listOf(issued.invite) + invites).map { AccountRow.Invite(it) },
                    )
                },
                onFailure = { error ->
                    state = state.copy(busy = false, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    fun dismissIssued() {
        state = state.copy(issuedUrl = null, issuedToken = null, sheet = null)
    }

    // --- *Serveur* -----------------------------------------------------------

    /**
     * The server page's twelve reads, the SPA's own, each card on its own
     * `screenFlow` so a card that fails does not take the page down with it.
     */
    private fun loadServer() {
        load = scope.launch {
            launch { card({ service.cachedServerCard() }, { service.serverCard() }) { state = state.copy(server = it) } }
            launch { card({ service.cachedStorage() }, { service.storage() }) { state = state.copy(storage = it) } }
            launch { card({ service.cachedRules() }, { service.rules() }) { state = state.copy(rules = it) } }
            launch { card({ service.cachedCleanup() }, { service.cleanup() }) { state = state.copy(cleanup = it) } }
            launch { card({ service.cachedArchiveStatus() }, { service.archiveStatus() }) { state = state.copy(archive = it) } }
            launch { card({ service.cachedMigrate() }, { service.migrate() }) { state = state.copy(migrate = it) } }
            launch { card({ service.cachedPlan() }, { service.plan() }) { state = state.copy(plan = it) } }
            launch { card({ service.cachedAnalysis() }, { service.analysis() }) { state = state.copy(analysis = it) } }
            launch { card({ service.cachedEnrich() }, { service.enrich() }) { state = state.copy(enrich = it) } }
            launch { card({ service.cachedCredits() }, { service.credits() }) { state = state.copy(credits = it) } }
            launch { card({ service.cachedSimilar() }, { service.similar() }) { state = state.copy(similar = it) } }
            launch { card({ service.cachedAutomatch() }, { service.automatch() }) { state = state.copy(automatch = it) } }
            launch { card({ service.cachedTranscribe() }, { service.transcribe(false) }) { state = state.copy(transcribe = it) } }
            // The quota's field starts on the server's own value.
            state.server.data?.let { server -> state = state.copy(quotaDraft = formatQuota(server.uploadQuotaGb)) }
            // A job that was still running when the page was left resumes its
            // poll: the server did not stop because the screen closed.
            pollIfRunning()
        }
    }

    private suspend fun <T> card(
        cached: suspend () -> T?,
        fresh: suspend () -> T,
        apply: (Section<T>) -> Unit,
    ) {
        screenFlow(
            cached = { withContext(io) { cached() } },
            fresh = { withContext(io) { fresh() } },
        ).collect { screen ->
            apply(
                when (screen) {
                    is ScreenState.Loading -> Section(loading = true)
                    is ScreenState.Ready -> Section(screen.data, stale = screen.stale)
                    is ScreenState.Failed -> Section(failure = screen.cause)
                },
            )
        }
    }

    /** *Tester* — re-tests the credential instead of the served verdict. */
    fun testDeezer() {
        if (state.deezerChecking) return
        state = state.copy(deezerChecking = true)
        scope.launch {
            val answer = withContext(io) { runCatching { service.testDeezer(true) } }
            state = state.copy(
                deezerChecking = false,
                server = answer.fold(
                    onSuccess = { Section(it) },
                    onFailure = { Section(state.server.data, failure = AdminCopy.guardMessage(it)) },
                ),
            )
        }
    }

    fun setQuotaDraft(text: String) {
        state = state.copy(quotaDraft = text)
    }

    /** *Enregistrer* the upload quota — 0 disables the limit. */
    fun saveQuota() {
        val value = state.quotaDraft.replace(',', '.').toDoubleOrNull()
        if (value == null || value < 0.0) {
            state = state.copy(actionError = "Valeur de quota invalide")
            return
        }
        if (state.busy) return
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.saveQuota(value) } }
            answer.fold(
                onSuccess = { card ->
                    state = state.copy(server = Section(card), busy = false, quotaDraft = formatQuota(card.uploadQuotaGb))
                    toast(
                        if (value == 0.0) "Quota d'upload désactivé (illimité)"
                        else "Quota d'upload réglé à ${formatQuota(card.uploadQuotaGb)} Go",
                    )
                },
                onFailure = { error ->
                    state = state.copy(busy = false, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    /** *Vider le cache* — the derived caches only, never the archive. */
    fun flushCache() {
        if (state.busy) return
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.flushCache() } }
            answer.fold(
                onSuccess = { freed ->
                    state = state.copy(busy = false)
                    toast("Cache serveur vidé (${AdminCopy.bytes(freed)} libérés)")
                    refreshStorage()
                },
                onFailure = { error ->
                    state = state.copy(busy = false, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    private fun refreshStorage() {
        scope.launch {
            withContext(io) { runCatching { service.storage() } }
                .onSuccess { state = state.copy(storage = Section(it)) }
        }
    }

    // --- the rules ----------------------------------------------------------

    /**
     * ONE rule per decision, moved in the frame, written once. The card then
     * draws the SERVER's whole rule set (rule 6): a refused value puts back what
     * the server still holds, with the cause.
     */
    fun saveRule(field: String, value: String) {
        val before = state.rules
        val preview = before.data?.let { rules -> previewRule(rules, field, value) }
        if (preview != null) state = state.copy(rules = Section(preview))
        scope.launch {
            val answer = withContext(io) { runCatching { service.saveRule(field, value) } }
            answer.fold(
                onSuccess = { rules -> state = state.copy(rules = Section(rules)) },
                onFailure = { error ->
                    state = state.copy(rules = before, actionError = "Réglage non enregistré : " + AdminCopy.guardMessage(error))
                },
            )
            // A cleanup-policy change moves what a preview would delete.
            if (field.startsWith("clean")) refreshCleanup()
        }
    }

    private fun refreshCleanup() {
        scope.launch {
            withContext(io) { runCatching { service.cleanup() } }
                .onSuccess { state = state.copy(cleanup = Section(it)) }
        }
    }

    /** *Prévisualiser* — what a cleanup would take, without taking it. */
    fun previewCleanup() {
        state = state.copy(cleanup = state.cleanup.copy(loading = true))
        refreshCleanup()
    }

    /** *Libérer maintenant* — the ONE button that deletes archived audio. */
    fun runCleanup() {
        if (state.busy) return
        state = state.copy(busy = true, actionError = null)
        scope.launch {
            val answer = withContext(io) { runCatching { service.runCleanup() } }
            answer.fold(
                onSuccess = { run ->
                    state = state.copy(busy = false)
                    toast(
                        if (run.deleted > 0u) {
                            "${run.deleted} fichier(s) supprimé(s), ${AdminCopy.bytes(run.freed)} libérés"
                        } else {
                            "Rien à libérer : le disque est au-dessus du seuil"
                        },
                    )
                    refreshCleanup()
                    refreshStorage()
                },
                onFailure = { error ->
                    state = state.copy(busy = false, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    // --- the archive ---------------------------------------------------------

    /** *Tout archiver maintenant* and its three narrower scopes. */
    fun startArchive(archiveScope: String) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startArchive(archiveScope) } }
            answer.fold(
                onSuccess = { status ->
                    state = state.copy(archive = Section(status))
                    toast("Archivage lancé — il continue même si vous fermez l'app")
                    poll(JOB_ARCHIVE)
                },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    /** *Mettre à niveau les fichiers* — safe to re-run. */
    fun startMigrate() {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startMigrate(false) } }
            answer.fold(
                onSuccess = { status ->
                    state = state.copy(migrate = Section(status))
                    toast("Mise à niveau lancée — elle continue même si vous fermez l'app")
                    poll(JOB_MIGRATE)
                },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    // --- the analysis --------------------------------------------------------

    fun startAnalysis(force: Boolean, workers: UInt) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startAnalysis(force, workers) } }
            answer.fold(
                onSuccess = { job ->
                    state = state.copy(analysis = Section(job))
                    poll(JOB_ANALYSIS)
                },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    /** *Arrêter* — the job stops after the track in hand and resumes next time. */
    fun stopAnalysis() {
        scope.launch {
            val answer = withContext(io) { runCatching { service.stopAnalysis() } }
            answer.fold(
                onSuccess = { job -> state = state.copy(analysis = Section(job)) },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    /** The nightly window: `HH:MM` on the SERVER's clock. */
    fun saveSchedule(enabled: Boolean, at: String, hours: UInt) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.saveSchedule(enabled, at, hours) } }
            answer.fold(
                onSuccess = { plan -> state = state.copy(plan = Section(plan)) },
                onFailure = { error ->
                    state = state.copy(actionError = AdminCopy.guardMessage(error))
                    refreshPlan()
                },
            )
        }
    }

    /** Who measures: the `server` or a `client` machine. */
    fun saveAnalysisPlace(place: String) {
        val before = state.plan
        state.plan.data?.let { plan -> state = state.copy(plan = Section(plan.copy(place = place))) }
        scope.launch {
            val answer = withContext(io) { runCatching { service.saveAnalysisPlace(place) } }
            answer.fold(
                onSuccess = { plan -> state = state.copy(plan = Section(plan)) },
                onFailure = { error ->
                    state = state.copy(plan = before, actionError = AdminCopy.guardMessage(error))
                },
            )
        }
    }

    private fun refreshPlan() {
        scope.launch {
            withContext(io) { runCatching { service.plan() } }
                .onSuccess { state = state.copy(plan = Section(it)) }
        }
    }

    // --- the backfills -------------------------------------------------------

    fun startEnrich(force: Boolean) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startEnrich(force) } }
            answer.fold(
                onSuccess = { status -> state = state.copy(enrich = Section(status)); poll(JOB_ENRICH) },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    fun startCredits(force: Boolean) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startCredits(force) } }
            answer.fold(
                onSuccess = { status -> state = state.copy(credits = Section(status)); poll(JOB_CREDITS) },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    fun startSimilar(force: Boolean) {
        scope.launch {
            val answer = withContext(io) { runCatching { service.startSimilar(force) } }
            answer.fold(
                onSuccess = { status -> state = state.copy(similar = Section(status)); poll(JOB_SIMILAR) },
                onFailure = { error -> state = state.copy(actionError = AdminCopy.guardMessage(error)) },
            )
        }
    }

    // --- the auto-match ------------------------------------------------------

    fun setAutomatch(on: Boolean) {
        val before = state.automatch
        state.automatch.data?.let { state = state.copy(automatch = Section(it.copy(enabled = on))) }
        scope.launch {
            val answer = withContext(io) { runCatching { service.setAutomatch(on) } }
            answer.fold(
                onSuccess = { status -> state = state.copy(automatch = Section(status)) },
                onFailure = { error ->
                    state = state.copy(automatch = before, actionError = "Réglage impossible : " + AdminCopy.guardMessage(error))
                },
            )
        }
    }

    fun runAutomatch() {
        scope.launch {
            val answer = withContext(io) { runCatching { service.runAutomatch() } }
            answer.fold(
                onSuccess = { status -> state = state.copy(automatch = Section(status)); poll(JOB_AUTOMATCH) },
                onFailure = { error ->
                    state = state.copy(actionError = AdminCopy.guardMessage(error))
                    // A `409` means one pass is already in flight: re-read rather
                    // than leaving the card saying nothing (`AutomatchPanel`).
                    if ((error as? uniffi.nz_ffi.NzException.Conflict) != null) refreshAutomatch()
                },
            )
        }
    }

    fun cancelAutomatch() {
        scope.launch {
            val answer = withContext(io) { runCatching { service.cancelAutomatch() } }
            answer.fold(
                onSuccess = { status -> state = state.copy(automatch = Section(status)) },
                onFailure = { refreshAutomatch() },
            )
        }
    }

    private fun refreshAutomatch() {
        scope.launch {
            withContext(io) { runCatching { service.automatch() } }
                .onSuccess { state = state.copy(automatch = Section(it)) }
        }
    }

    // --- transcription -------------------------------------------------------

    /** ONE transcription setting per decision; a refusal puts the old value back. */
    fun saveTranscribe(field: String, value: String) {
        val before = state.transcribe
        scope.launch {
            val answer = withContext(io) { runCatching { service.saveTranscribe(field, value) } }
            answer.fold(
                onSuccess = { settings -> state = state.copy(transcribe = Section(settings)) },
                onFailure = { error ->
                    val cause = AdminCopy.guardMessage(error)
                    state = state.copy(
                        transcribe = before,
                        actionError = if (field == "enabled") "Impossible d'activer : $cause" else cause,
                    )
                },
            )
        }
    }

    /** *Tester le moteur* — the one slow read of the card. */
    fun probeTranscribe() {
        state = state.copy(transcribe = state.transcribe.copy(loading = true))
        scope.launch {
            val answer = withContext(io) { runCatching { service.transcribe(true) } }
            answer.fold(
                onSuccess = { settings -> state = state.copy(transcribe = Section(settings)) },
                onFailure = { error ->
                    state = state.copy(
                        transcribe = Section(state.transcribe.data, failure = "Le moteur n'a pas répondu. " + AdminCopy.guardMessage(error)),
                    )
                },
            )
        }
    }

    // --- the polls (rule 2) --------------------------------------------------

    /**
     * The job a page must keep watching, when it was already running on open:
     * a job is the SERVER's, and closing the screen never stopped it.
     */
    private fun pollIfRunning() {
        if (state.archive.data?.running == true) poll(JOB_ARCHIVE)
        if (state.migrate.data?.running == true) poll(JOB_MIGRATE)
        if (state.analysis.data?.running == true) poll(JOB_ANALYSIS)
        if (state.enrich.data?.running == true) poll(JOB_ENRICH)
        if (state.credits.data?.running == true) poll(JOB_CREDITS)
        if (state.similar.data?.running == true) poll(JOB_SIMILAR)
        if (state.automatch.data?.running == true) poll(JOB_AUTOMATCH)
    }

    /**
     * ONE loop per job, at the SPA's own cadence, and it stops the moment the
     * server says the job is over — never a tick for nothing.
     */
    private fun poll(kind: String) {
        if (!polls.add(kind)) return
        scope.launch {
            try {
                while (true) {
                    delay(cadenceOf(kind))
                    val answer = withContext(io) { runCatching { sampleJob(kind) } }
                    val running = answer.fold(
                        onSuccess = { job ->
                            applyJob(kind, job)
                        },
                        onFailure = { false },
                    )
                    if (!running) break
                }
            } finally {
                polls.remove(kind)
            }
            finished(kind)
        }
    }

    /** One read of a job's route, off the screen's thread. */
    private suspend fun sampleJob(kind: String): Any = when (kind) {
        JOB_ARCHIVE -> service.archiveStatus()
        JOB_MIGRATE -> service.migrate()
        JOB_ANALYSIS -> service.analysis()
        JOB_ENRICH -> service.enrich()
        JOB_CREDITS -> service.credits()
        JOB_SIMILAR -> service.similar()
        JOB_AUTOMATCH -> service.automatch()
        else -> error("travail inconnu : $kind")
    }

    /** What the card draws, and whether the job is still running. */
    private fun applyJob(kind: String, job: Any): Boolean = when (kind) {
        JOB_ARCHIVE -> (job as AdminArchiveStatus).let { state = state.copy(archive = Section(it)); it.running }
        JOB_MIGRATE -> (job as AdminMigrate).let { state = state.copy(migrate = Section(it)); it.running }
        JOB_ANALYSIS -> (job as AdminAnalysisJob).let { state = state.copy(analysis = Section(it)); it.running }
        JOB_ENRICH -> (job as AdminEnrich).let { state = state.copy(enrich = Section(it)); it.running }
        JOB_CREDITS -> (job as AdminCredits).let { state = state.copy(credits = Section(it)); it.running }
        JOB_SIMILAR -> (job as AdminSimilar).let { state = state.copy(similar = Section(it)); it.running }
        JOB_AUTOMATCH -> (job as AdminAutomatch).let { state = state.copy(automatch = Section(it)); it.running }
        else -> false
    }

    /** What a job says when it ends: what it did, or the CAUSE of each failure. */
    private fun finished(kind: String) {
        when (kind) {
            JOB_ARCHIVE -> state.archive.data?.let { status ->
                if (status.total > 0u) {
                    val bits = mutableListOf("${status.archived} archivé(s)")
                    if (status.unavailable > 0u) bits.add("${status.unavailable} indisponible(s)")
                    if (status.failed > 0u) bits.add("${status.failed} échec(s)")
                    toast("Archivage terminé : " + bits.joinToString(", "))
                }
                refreshStorage()
            }
            JOB_MIGRATE -> state.migrate.data?.let { status ->
                toast(
                    if (status.failed > 0u) {
                        "Mise à niveau terminée : ${status.done} fichier(s), ${status.failed} échec(s)"
                    } else {
                        "Mise à niveau terminée : ${status.done} fichier(s) vérifié(s)"
                    },
                )
            }
            JOB_ANALYSIS -> {
                state.analysis.data?.error?.let { toast(it) }
                refreshPlan()
            }
            JOB_ENRICH -> state.enrich.data?.let { status ->
                toast(
                    if (status.failures.failed > 0u) "${status.failures.failed} titre(s) en échec" else "Faits à jour",
                )
            }
            JOB_CREDITS -> state.credits.data?.let { status ->
                toast(
                    if (status.failed > 0u) "${status.failed} titre(s) en échec" else "Crédits à jour",
                )
            }
            JOB_SIMILAR -> toast("Voisins sonores à jour")
            JOB_AUTOMATCH -> state.automatch.data?.let { status ->
                val replaced = status.lastCounts.firstOrNull { it.name == "match" }?.count ?: 0u
                toast(
                    when {
                        status.lastStopped != null -> "Remplacement automatique interrompu : Deezer est injoignable"
                        replaced > 0u -> "$replaced titre(s) remplacé(s) automatiquement"
                        else -> "Aucun titre n'a atteint 99 % : rien n'a été remplacé"
                    },
                )
            }
        }
    }

    private fun cadenceOf(kind: String): Long = when (kind) {
        JOB_SIMILAR -> 900
        JOB_MIGRATE -> 2_000
        else -> 1_500
    }

    // --- the copy's small edits ----------------------------------------------

    private fun replaceRow(id: String, fresh: AdminUserRow) {
        val rows = state.rows.map { row ->
            if (row is AccountRow.User && row.row.id == id) AccountRow.User(fresh) else row
        }
        val detail = state.userDetail.data
        state = state.copy(
            rows = rows,
            userDetail = if (detail != null && detail.user.id == id) {
                Section(detail.copy(user = fresh))
            } else {
                state.userDetail
            },
        )
    }

    private fun dropInvite(id: String) {
        val invites = state.invites.filterNot { it.id == id }
        state = state.copy(
            invites = invites,
            rows = state.rows.users().map { AccountRow.User(it) } + invites.map { AccountRow.Invite(it) },
        )
    }

    /** `mergeUsers` — a page joins the rows already loaded, deduplicated. */
    private fun mergeRows(current: List<AdminUserRow>, page: AdminUsersPage): List<AdminUserRow> {
        val seen = current.map { it.id }.toMutableSet()
        return current + page.users.filter { seen.add(it.id) }
    }

    /** The SPA's `stateKey` for an invitation, or a user's, as one string. */
    private fun inviteAsUser(invite: AdminInviteRow): AdminUserRow = AdminUserRow(
        id = invite.id,
        name = AdminCopy.inviteName(invite),
        admin = false,
        disabled = false,
        lastLogin = null,
        mail = null,
        onboarding = null,
    )

    companion object {
        /** The two categories this lot owns (`SettingsCopy.ADMIN_CATEGORIES`). */
        const val SECTION_USERS: String = "users"
        const val SECTION_SERVER: String = "server"

        const val JOB_ARCHIVE: String = "archive"
        const val JOB_MIGRATE: String = "migrate"
        const val JOB_ANALYSIS: String = "analysis"
        const val JOB_ENRICH: String = "enrich"
        const val JOB_CREDITS: String = "credits"
        const val JOB_SIMILAR: String = "similar"
        const val JOB_AUTOMATCH: String = "automatch"

        /** `Stats.svelte`'s number field: two decimals at most, no trailing zeros. */
        fun formatQuota(gb: Double): String =
            if (gb == gb.toLong().toDouble()) {
                gb.toLong().toString()
            } else {
                String.format(java.util.Locale.ROOT, "%.2f", gb).trimEnd('0').trimEnd('.')
            }
    }
}

/** What a destructive confirmation is about (`AdminUsers.svelte`'s three). */
enum class ConfirmAction { Delete, Revoke }

/** The rules card's optimistic preview, one field at a time. */
internal fun previewRule(rules: AdminRules, field: String, value: String): AdminRules {
    val on = value == "true"
    val number = value.toDoubleOrNull()
    val set = rules.rules
    val next = when (field) {
        "on_fav_track", "on_fav_album", "on_fav_artist", "on_fav_playlist",
        "on_playlist_add", "on_play_context", "on_podcast",
        -> rules.copy(events = rules.events.map { if (it.id == field) it.copy(on = on) else it })
        "artist_scope" -> rules.copy(rules = set.copy(artistScope = value))
        "artist_limit" -> rules.copy(rules = set.copy(artistLimit = number?.toInt()?.toUInt() ?: set.artistLimit))
        "chapters" -> rules.copy(rules = set.copy(chapters = on))
        "chapter_urls" -> rules.copy(rules = set.copy(chapterUrls = on))
        "clean_on" -> rules.copy(rules = set.copy(cleanOn = on))
        "clean_min_free_gb" -> rules.copy(rules = set.copy(cleanMinFreeGb = number ?: set.cleanMinFreeGb))
        "clean_stale_days" -> rules.copy(rules = set.copy(cleanStaleDays = number?.toInt()?.toUInt() ?: set.cleanStaleDays))
        "clean_order" -> rules.copy(rules = set.copy(cleanOrder = value))
        "clean_keep_fav" -> rules.copy(rules = set.copy(cleanKeepFav = on))
        "clean_keep_playlist" -> rules.copy(rules = set.copy(cleanKeepPlaylist = on))
        "clean_keep_podcast" -> rules.copy(rules = set.copy(cleanKeepPodcast = on))
        else -> rules
    }
    return next
}

/** The French sentence for the cause of a failed load, never a code (rule 3). */
internal fun causeOf(e: Throwable): String = NzErrors.text(e)

/** A cancelled screen is not a failure: it says nothing. */
internal fun isCancellation(e: Throwable): Boolean = e is CancellationException
