<script context="module">
  let cache = null;
  const CACHE_TTL = 60000;
</script>

<script>
  import { onMount, onDestroy, tick } from "svelte";
  import { get } from "svelte/store";
  import { replace } from "svelte-spa-router";
  import { user, toasts } from "../lib/stores.js";
  import { online } from "../lib/net.js";
  import { api } from "../lib/api.js";
  import {
    PAGE_SIZE,
    MAX_ROWS,
    FILTERS,
    mergeUsers,
    inviteRow,
    filterRows,
    stateLabel,
    stateKey,
    roleLabel,
    relativeTime,
    absoluteTime,
    shortDate,
    lastSeenLabel,
    expiryLabel,
    guardMessage,
    countLabel,
    clockTime,
    copyText,
  } from "../lib/adminusers.js";
  import { dialog, veil } from "../lib/motion.js";
  import { toBody } from "../lib/portal.js";
  import Icon from "./Icon.svelte";
  import Avatar from "./Avatar.svelte";
  import InviteSheet from "./InviteSheet.svelte";

  let q = "";
  let term = "";
  let filter = "all";
  let rows = [];
  let invites = [];
  let next = null;
  let total = 0;
  let loading = true;
  let searching = false;
  let error = null;
  let refreshError = null;
  let moreError = null;
  let loadingMore = false;
  let refreshing = false;
  let invitesLoading = false;
  let invitesLoaded = false;
  let invitesError = null;
  let loadedAt = null;
  let seq = 0;
  let debounceTimer = null;
  let detail = null;
  let confirm = null;
  let inviteSheet = null;
  let linkFallback = null;
  let links = new Map();
  let actionError = "";
  let detailBusy = false;
  let returnFocus = null;
  let detailEl = null;
  let confirmEl = null;
  let cancelEl = null;
  let typedEl = null;
  let linkFallbackEl = null;
  let linkFieldEl = null;
  let searchEl = null;
  let chipRow = null;
  let wideQuery = null;
  let desktop =
    typeof matchMedia === "function" &&
    matchMedia("(min-width: 1280px)").matches;

  $: combined = [...rows, ...invites];
  $: visible = filterRows(combined, filter, term);
  $: visibleUsers = visible.filter((row) => row.kind === "user");
  $: visibleInvites = visible.filter((row) => row.kind === "invite");
  $: allLoaded = rows.length >= total;
  $: activeLoaded = rows.filter((row) => !row.disabled).length;
  $: disabledLoaded = rows.filter((row) => row.disabled).length;
  $: hasFilters = filter !== "all" || !!term;
  $: shortQuery = q.trim().length === 1;
  $: showGroup = filter === "all" && visibleUsers.length > 0 && visibleInvites.length > 0;
  $: showMore = !!next && filter !== "invited" && rows.length < MAX_ROWS;
  $: capped = !!next && rows.length >= MAX_ROWS;
  $: matchingInvites = filterRows(invites, "invited", term);
  $: countLine =
    loading && !rows.length
      ? "Chargement des comptes…"
      : refreshError && term
        ? "Recherche indisponible : réessayez."
        : term
        ? `${countLabel(total, "compte")} correspondant${total > 1 ? "s" : ""}${matchingInvites.length ? ` · ${countLabel(matchingInvites.length, "invitation")} correspondante${matchingInvites.length > 1 ? "s" : ""}` : ""}`
        : `${countLabel(total, "compte")} · ${countLabel(invites.length, "invitation")} en attente`;
  $: filterHint =
    filter !== "all" && !allLoaded
      ? `Le filtre porte sur ${countLabel(rows.length, "compte chargé", "comptes chargés")}.`
      : "";
  $: chipCounts = {
    all: total + (term ? matchingInvites.length : invites.length),
    invited: matchingInvites.length,
    active: allLoaded ? activeLoaded : null,
    admins: allLoaded ? rows.filter((row) => row.kind === "user" && row.admin).length : null,
    members: allLoaded ? rows.filter((row) => row.kind === "user" && !row.admin).length : null,
    disabled: allLoaded ? disabledLoaded : null,
  };
  $: rangeLabel =
    filter === "invited"
      ? countLabel(visibleInvites.length, "invitation")
      : filter === "all"
        ? `${rows.length} sur ${total}`
        : allLoaded
          ? `${visible.length} affiché${visible.length > 1 ? "s" : ""} sur ${total}`
          : `${visible.length} affiché${visible.length > 1 ? "s" : ""} · ${rows.length} sur ${total} chargés`;

  onMount(() => {
    if (typeof matchMedia === "function") {
      wideQuery = matchMedia("(min-width: 1280px)");
      wideQuery.addEventListener?.("change", onWide);
    }
    if (!get(user)?.admin) {
      loading = false;
      return;
    }
    if (cache) {
      hydrate(cache);
      if (!cache.at || Date.now() - cache.at >= CACHE_TTL) loadFirst();
      return;
    }
    loadFirst();
  });

  onDestroy(() => {
    clearTimeout(debounceTimer);
    wideQuery?.removeEventListener?.("change", onWide);
  });

  function onWide() {
    desktop = wideQuery.matches;
  }

  function hydrate(saved) {
    q = saved.q || "";
    term = saved.term || "";
    filter = saved.filter || "all";
    rows = saved.rows || [];
    invites = saved.invites || [];
    next = saved.next || null;
    total = saved.total || 0;
    invitesLoaded = !!saved.invitesLoaded;
    loadedAt = saved.at || null;
    loading = false;
    revealChip();
    if (!invitesLoaded) loadInvites();
  }

  function remember(at = loadedAt) {
    if (!at) return;
    cache = {
      q,
      term,
      filter,
      rows,
      invites,
      next,
      total,
      invitesLoaded,
      at: at || Date.now(),
    };
  }

  function touch() {
    loadedAt = Date.now();
    remember();
  }

  function revealChip() {
    tick().then(() => {
      if (!chipRow || chipRow.scrollWidth <= chipRow.clientWidth) return;
      chipRow
        .querySelector(".chip.on")
        ?.scrollIntoView({ block: "nearest", inline: "center" });
    });
  }

  async function loadFirst() {
    if (!get(user)?.admin) return;
    clearTimeout(debounceTimer);
    const run = ++seq;
    loading = rows.length === 0;
    searching = rows.length > 0;
    error = null;
    refreshError = null;
    moreError = null;
    try {
      const page = await api.adminUsers({ q: term, limit: PAGE_SIZE });
      if (run !== seq) return;
      const merged = mergeUsers(null, page, false);
      rows = merged.rows;
      next = merged.next;
      total = merged.total;
      loadedAt = Date.now();
      remember();
      if (!invitesLoaded) loadInvites();
    } catch (err) {
      if (run !== seq) return;
      if (err?.status === 403) {
        cache = null;
        rows = [];
        invites = [];
        total = 0;
        next = null;
        invitesLoaded = false;
        error = err;
      } else if (rows.length) refreshError = err;
      else error = err;
    } finally {
      if (run === seq) {
        loading = false;
        searching = false;
      }
    }
  }

  async function loadMore() {
    if (loadingMore || !next || rows.length >= MAX_ROWS) return;
    const run = seq;
    loadingMore = true;
    moreError = null;
    try {
      const page = await api.adminUsers({
        q: term,
        after: next,
        limit: PAGE_SIZE,
      });
      if (run !== seq) return;
      const merged = mergeUsers({ rows }, page, true);
      rows = merged.rows;
      next = merged.next;
      total = merged.total;
      touch();
    } catch (err) {
      if (run === seq) moreError = err;
    } finally {
      loadingMore = false;
    }
  }

  async function loadInvites(force = false) {
    if (!force && invitesLoaded) return;
    if (invitesLoading) return;
    invitesLoading = true;
    invitesError = null;
    try {
      const data = await api.adminInvites({ state: "pending", limit: 200 });
      invites = (data.invites || []).map((invite) => inviteRow(invite));
      invitesLoaded = true;
      if (!loadedAt) loadedAt = Date.now();
      remember();
    } catch (err) {
      invitesError = err;
      if (err?.status === 403) invites = [];
    } finally {
      invitesLoading = false;
    }
  }

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    refreshError = null;
    moreError = null;
    const run = ++seq;
    const limit = Math.min(Math.max(rows.length, PAGE_SIZE), 200);
    try {
      const page = await api.adminUsers({ q: term, limit });
      if (run !== seq) return;
      const merged = mergeUsers(null, page, false);
      rows = merged.rows;
      next = merged.next;
      total = merged.total;
      loadedAt = Date.now();
      remember();
      loadInvites(true);
    } catch (err) {
      if (run === seq) {
        if (err?.status === 403) {
          cache = null;
          rows = [];
          invites = [];
          total = 0;
          next = null;
          error = err;
        } else {
          refreshError = err;
        }
      }
    } finally {
      if (run === seq) refreshing = false;
    }
  }

  function onQueryInput(event) {
    q = event.currentTarget.value;
    scheduleSearch();
  }

  function scheduleSearch() {
    clearTimeout(debounceTimer);
    const value = q.trim();
    if (!value) {
      if (term) {
        term = "";
        loadFirst();
      }
      return;
    }
    if (value.length < 2 || value === term) return;
    debounceTimer = setTimeout(() => {
      term = value;
      loadFirst();
    }, 300);
  }

  function submitSearch() {
    clearTimeout(debounceTimer);
    const value = q.trim();
    if (!value) {
      if (term) {
        term = "";
        loadFirst();
      }
      return;
    }
    if (value.length < 2 || value === term) return;
    term = value;
    loadFirst();
  }

  function clearSearch() {
    q = "";
    if (term) {
      term = "";
      loadFirst();
    }
    tick().then(() => searchEl?.focus());
  }

  function setFilter(id) {
    if (filter === id) return;
    filter = id;
    remember();
    revealChip();
  }

  function clearFilters() {
    filter = "all";
    q = "";
    if (term) {
      term = "";
      loadFirst();
    } else {
      remember();
    }
    revealChip();
    tick().then(() => searchEl?.focus());
  }

  function isMe(row) {
    return row.kind === "user" && get(user)?.name === row.name;
  }

  function stateIcon(row) {
    const key = stateKey(row);
    if (key === "invite") return "link";
    if (key === "expired") return "timer";
    if (key === "used") return "check";
    if (key === "revoked" || key === "disabled") return "minusCircle";
    return "check";
  }

  function rowAria(row) {
    return row.kind === "invite"
      ? `Ouvrir l'invitation ${row.name}`
      : `Ouvrir le compte de ${row.name}`;
  }

  function rowSeen(row) {
    return row.kind === "invite"
      ? `Invitation créée ${relativeTime(row.created)} · ${lastSeenLabel(row)}`
      : `Dernière connexion : ${lastSeenLabel(row)}`;
  }

  function rowSeenTitle(row) {
    return absoluteTime(row.kind === "invite" ? row.expires : row.last_login);
  }

  function openDetail(row) {
    returnFocus = document.activeElement;
    actionError = "";
    detail = row;
    tick().then(() => detailEl?.focus({ preventScroll: true }));
  }

  function closeDetail() {
    detail = null;
    actionError = "";
    restoreFocus();
  }

  function restoreFocus() {
    const element = returnFocus;
    returnFocus = null;
    tick().then(() => {
      if (element?.isConnected) element.focus({ preventScroll: true });
      else searchEl?.focus({ preventScroll: true });
    });
  }

  function openInvite(source = null) {
    if (!returnFocus) returnFocus = document.activeElement;
    detail = null;
    actionError = "";
    inviteSheet = { source };
  }

  function closeInvite() {
    inviteSheet = null;
    restoreFocus();
  }

  function onInviteCreated(event) {
    const info = event.detail || {};
    if (!info.invite) return;
    if (info.replaces) {
      invites = invites.filter((invite) => invite.id !== info.replaces);
      links.delete(info.replaces);
    }
    const row = inviteRow(info.invite);
    if (!row.created_by) row.created_by = get(user)?.name || null;
    invites = [row, ...invites];
    if (info.url) links.set(row.id, info.url);
    touch();
  }

  function onInviteStale(event) {
    const id = event.detail;
    invites = invites.filter((invite) => invite.id !== id);
    links.delete(id);
    touch();
  }

  function onInviteRevoked(event) {
    const id = event.detail;
    invites = invites.filter((invite) => invite.id !== id);
    links.delete(id);
    touch();
    toasts.push("Invitation révoquée");
  }

  function linkFor(row) {
    return stateKey(row) === "invite" ? links.get(row.id) || "" : "";
  }

  async function copyLink(row) {
    const url = linkFor(row);
    if (!url) return;
    const copied = await copyText(url);
    if (copied) {
      toasts.push("Lien copié");
      return;
    }
    toasts.push(
      "Copie impossible — copiez le lien manuellement.",
      "error",
      6000,
      { label: "Afficher le lien", run: () => showLink(url) },
    );
  }

  function showLink(url) {
    detail = null;
    linkFallback = { url };
    tick().then(() => {
      linkFieldEl?.focus();
      linkFieldEl?.select();
      if (linkFieldEl) linkFieldEl.scrollLeft = 0;
    });
  }

  function closeLink() {
    linkFallback = null;
    restoreFocus();
  }

  function askConfirm(config) {
    detail = null;
    actionError = "";
    confirm = {
      typed: "",
      error: "",
      busy: false,
      loading: false,
      uploads: null,
      playlists: null,
      ...config,
    };
    tick().then(() => {
      if (config.kind === "delete") confirmEl?.focus({ preventScroll: true });
      else cancelEl?.focus({ preventScroll: true });
    });
    if (config.kind === "delete") loadDeleteInfo();
  }

  async function loadDeleteInfo() {
    if (!confirm || confirm.kind !== "delete") return;
    confirm = { ...confirm, loading: true, error: "" };
    try {
      const data = await api.adminUser(confirm.row.id);
      if (!confirm || confirm.kind !== "delete") return;
      confirm = {
        ...confirm,
        loading: false,
        uploads: data.uploads,
        playlists: data.playlists,
      };
      tick().then(() => typedEl?.focus({ preventScroll: true }));
    } catch (err) {
      if (!confirm) return;
      if (err?.status === 404) {
        removeUser(confirm.row.id);
        closeConfirm();
        toasts.push("Ce compte n'existe plus.", "error");
        return;
      }
      confirm = { ...confirm, loading: false, error: guardMessage(err) };
    }
  }

  function askRole(row) {
    askConfirm({
      kind: "role",
      row,
      title: row.admin
        ? `Retirer les droits d'administrateur à ${row.name} ?`
        : `Nommer ${row.name} administrateur ?`,
      body: row.admin
        ? "Le compte et sa bibliothèque restent ; l'accès aux réglages du serveur et aux comptes est retiré. La personne sera déconnectée pour appliquer ce changement."
        : "Un administrateur voit et modifie tout le serveur : comptes, réglages, sources et clés.",
      label: row.admin ? "Retirer les droits" : "Nommer administrateur",
      danger: row.admin,
    });
  }

  function askDisable(row) {
    if (row.disabled) {
      reactivate(row);
      return;
    }
    askConfirm({
      kind: "state",
      row,
      title: `Désactiver ${row.name} ?`,
      body: "Cette personne sera déconnectée de tous ses appareils. Ses playlists, sa bibliothèque et son historique sont conservés. Vous pourrez la réactiver à tout moment.",
      label: "Désactiver le compte",
      danger: true,
    });
  }

  async function reactivate(row) {
    if (detailBusy) return;
    detailBusy = true;
    actionError = "";
    const previous = { ...row };
    patchUser(row.id, { disabled: false });
    try {
      const data = await api.adminUserPatch(row.id, { disabled: false });
      replaceUser(data.user);
      toasts.push("Compte réactivé");
    } catch (err) {
      replaceUser(previous);
      actionError = guardMessage(err);
      if (err?.status === 404) {
        removeUser(row.id);
        closeDetail();
      }
    } finally {
      detailBusy = false;
    }
  }

  function askDelete(row) {
    askConfirm({
      kind: "delete",
      row,
      title: `Supprimer ${row.name} ?`,
      body: `Cette action est définitive. Ses playlists et son historique seront supprimés. L'audio déjà archivé reste sur le serveur. Tapez ${row.name} pour confirmer.`,
      label: "Supprimer définitivement",
      danger: true,
    });
  }

  function askRevoke(row) {
    askConfirm({
      kind: "revoke",
      row,
      title: "Révoquer l'invitation ?",
      body: "Le lien cessera de fonctionner immédiatement. La personne n'aura plus de moyen de créer son compte avec ce lien.",
      label: "Révoquer l'invitation",
      danger: true,
    });
  }

  function closeConfirm(reopen = false) {
    const row = confirm?.row;
    confirm = null;
    if (reopen && row) {
      detail = row;
      tick().then(() => detailEl?.focus({ preventScroll: true }));
    } else {
      restoreFocus();
    }
  }

  async function runConfirm() {
    if (!confirm || confirm.busy) return;
    const kind = confirm.kind;
    const row = confirm.row;
    if (kind === "delete" && confirm.typed !== row.name) return;
    confirm = { ...confirm, busy: true, error: "" };

    if (kind === "role" || kind === "state") {
      const previous = { ...row };
      const payload =
        kind === "role" ? { admin: !row.admin } : { disabled: !row.disabled };
      patchUser(row.id, payload);
      try {
        const data = await api.adminUserPatch(row.id, payload);
        replaceUser(data.user);
        toasts.push(
          kind === "role"
            ? data.user.admin
              ? "Droits d'administrateur accordés"
              : "Droits d'administrateur retirés"
            : data.user.disabled
              ? "Compte désactivé"
              : "Compte réactivé",
        );
        finishConfirm();
      } catch (err) {
        replaceUser(previous);
        if (err?.status === 404) removeUser(row.id);
        confirm = { ...confirm, busy: false, error: guardMessage(err) };
      }
      return;
    }

    if (kind === "delete") {
      try {
        const data = await api.adminUserDelete(row.id, confirm.typed);
        removeUser(row.id);
        finishConfirm();
        const moved = [];
        if (data.uploads_reassigned)
          moved.push(
            `${countLabel(data.uploads_reassigned, "fichier")} réattribué${data.uploads_reassigned > 1 ? "s" : ""}`
          );
        if (data.podcasts_reassigned)
          moved.push(
            `${countLabel(data.podcasts_reassigned, "émission")} réattribuée${data.podcasts_reassigned > 1 ? "s" : ""}`
          );
        toasts.push(moved.length ? `Compte supprimé · ${moved.join(" · ")}` : "Compte supprimé");
      } catch (err) {
        confirm = { ...confirm, busy: false, error: guardMessage(err) };
      }
      return;
    }

    if (kind === "revoke") {
      const previous = invites.slice();
      invites = invites.filter((invite) => invite.id !== row.id);
      links.delete(row.id);
      try {
        await api.adminInviteRevoke(row.id);
        touch();
        finishConfirm();
        toasts.push("Invitation révoquée");
      } catch (err) {
        invites = previous;
        if (
          err?.body?.code === "invite_used" ||
          err?.body?.code === "invite_not_found"
        ) {
          touch();
          finishConfirm();
          toasts.push(guardMessage(err), "error");
        } else {
          confirm = { ...confirm, busy: false, error: guardMessage(err) };
        }
      }
    }
  }

  function finishConfirm() {
    confirm = null;
    detail = null;
    touch();
    restoreFocus();
  }

  function patchUser(id, patch) {
    rows = rows.map((row) => (row.id === id ? { ...row, ...patch } : row));
  }

  function replaceUser(userData) {
    const replacement = accountRow(userData);
    rows = rows.map((row) => (row.id === replacement.id ? replacement : row));
    if (detail?.id === replacement.id) detail = replacement;
    touch();
  }

  function removeUser(id) {
    rows = rows.filter((row) => row.id !== id);
    total = Math.max(0, total - 1);
    if (detail?.id === id) detail = null;
    touch();
  }

  function accountRow(userData) {
    return {
      ...userData,
      kind: "user",
      admin: !!userData.admin,
      disabled: !!userData.disabled,
      last_login: userData.last_login || null,
    };
  }

  function onWindowKey(event) {
    if (inviteSheet) return;
    const layer = confirm
      ? confirmEl
      : detail
        ? detailEl
        : linkFallback
          ? linkFallbackEl
          : null;
    if (!layer) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (confirm) closeConfirm(true);
      else if (detail) closeDetail();
      else closeLink();
      return;
    }
    if (event.key === "Tab") trapFocus(event, layer);
  }

  function trapFocus(event, layer) {
    const focusable = [
      ...layer.querySelectorAll(
        "button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex='-1'])",
      ),
    ].filter((element) => element.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (
      event.shiftKey &&
      (active === first || active === layer || !layer.contains(active))
    ) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window on:keydown={onWindowKey} />

{#if $user?.admin}
  <div class="admin">
    {#if error && error.status === 403 && !rows.length}
      <section class="state-block" role="alert">
        <span class="state-ico"><Icon name="shield" size={26} /></span>
        <h2>Accès administrateur perdu</h2>
        <p>Votre compte n'a plus les droits d'administration. Reconnectez-vous pour vérifier.</p>
        <button class="btn primary" on:click={() => replace("/settings")}>
          Retour aux réglages
        </button>
      </section>
    {:else}
      <div class="admin-head">
        <div class="head-copy">
          <p class="lede">Les comptes de ce serveur et les invitations en attente.</p>
          <p class="count label-mono" role="status" aria-live="polite">
            {countLine}
            {#if loadedAt}<span class="fresh">· Actualisé à {clockTime(new Date(loadedAt).toISOString())}</span>{/if}
          </p>
        </div>
        <div class="head-actions">
          <button class="btn primary invite" on:click={() => openInvite()}>
            <Icon name="personAdd" size={17} />
            <span class="invite-long">Inviter quelqu'un</span>
            <span class="invite-short">Inviter</span>
          </button>
          <button
            class="icon-btn"
            aria-label="Rafraîchir la liste"
            title="Rafraîchir la liste"
            on:click={refresh}
            disabled={refreshing}
          >
            <Icon name="refresh" size={18} />
          </button>
        </div>
      </div>

      {#if !$online}
        <div class="banner" role="status">
          <Icon name="cloudOff" size={16} />
          <span>Vous êtes hors ligne. La dernière liste connue reste affichée.</span>
        </div>
      {/if}

      {#if refreshError}
        <div class="banner error" role="alert">
          <Icon name="alert" size={16} />
          <span>La liste n'a pas pu être actualisée. {guardMessage(refreshError)}</span>
          <button class="inline-action" on:click={refresh}>Réessayer</button>
        </div>
      {/if}

      <div class="search field" class:filled={!!q}>
        <Icon name="search" size={16} />
        <input
          bind:this={searchEl}
          type="search"
          value={q}
          placeholder="Rechercher un nom…"
          aria-label="Rechercher un compte"
          autocomplete="off"
          spellcheck="false"
          on:input={onQueryInput}
          on:keydown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submitSearch();
            }
          }}
        />
        {#if q}
          <button class="clear" on:click={clearSearch} aria-label="Effacer la recherche">
            <Icon name="close" size={15} />
          </button>
        {/if}
      </div>
      {#if shortQuery}
        <p class="search-hint">Saisissez au moins 2 caractères pour filtrer.</p>
      {/if}

      <div class="filters" bind:this={chipRow} role="group" aria-label="Filtrer par état">
        {#each FILTERS as item (item.id)}
          <button
            class="chip"
            class:on={filter === item.id}
            aria-pressed={filter === item.id}
            on:click={() => setFilter(item.id)}
          >
            {item.label}
            {#if chipCounts[item.id] !== null}
              <span class="chip-n">{chipCounts[item.id]}</span>
            {/if}
          </button>
        {/each}
        {#if hasFilters && visible.length}
          <button class="clear-filters" on:click={clearFilters}>Effacer les filtres</button>
        {/if}
      </div>
      {#if filterHint}<p class="search-hint">{filterHint}</p>{/if}

      {#if loading && !rows.length && !invites.length}
        {#if desktop}
          <div class="board skeleton-board" aria-label="Chargement des comptes" aria-busy="true">
            {#each Array(6) as _}
              <div class="sk-row">
                <span class="sk avatar"></span>
                <span class="sk line name"></span>
                <span class="sk line small"></span>
                <span class="sk line small"></span>
                <span class="sk line tiny"></span>
              </div>
            {/each}
          </div>
        {:else}
          <div class="cards skeleton-cards" aria-label="Chargement des comptes" aria-busy="true">
            {#each Array(4) as _}
              <div class="sk-card">
                <span class="sk avatar"></span>
                <span class="sk-lines">
                  <span class="sk line name"></span>
                  <span class="sk line small"></span>
                  <span class="sk line tiny"></span>
                </span>
              </div>
            {/each}
          </div>
        {/if}
      {:else if error && !rows.length}
        <section class="state-block" role="alert">
          <span class="state-ico"><Icon name="cloudOff" size={26} /></span>
          <h2>Impossible de charger les comptes.</h2>
          <p>{guardMessage(error)}</p>
          <button class="btn primary" on:click={loadFirst}>
            <Icon name="refresh" size={16} />
            Réessayer
          </button>
        </section>
      {:else}
        <div class="list-wrap" aria-busy={searching}>
          {#if !visible.length}
            <section class="state-block empty">
              <span class="state-ico">
                <Icon name={filter === "invited" ? "personAdd" : "users"} size={26} />
              </span>
              {#if filter === "invited" && term}
                <h2>Aucune invitation ne correspond.</h2>
                <p>Essayez un autre nom ou retirez la recherche.</p>
                <button class="btn" on:click={clearFilters}>Effacer les filtres</button>
              {:else if filter === "invited"}
                <h2>Aucune invitation en attente.</h2>
                <p>Un lien d'invitation crée un compte membre, une seule fois.</p>
                <button class="btn primary" on:click={() => openInvite()}>
                  <Icon name="personAdd" size={16} />
                  Inviter quelqu'un
                </button>
              {:else if hasFilters}
                <h2>Aucun compte ne correspond.</h2>
                <p>Essayez un autre nom ou retirez les filtres.</p>
                <button class="btn" on:click={clearFilters}>Effacer les filtres</button>
                {#if showMore}
                  <button class="btn ghost" on:click={loadMore} disabled={loadingMore}>
                    {loadingMore ? "Chargement…" : "Charger la suite"}
                  </button>
                {/if}
              {:else}
                <h2>Aucun compte pour l'instant.</h2>
                <p>Invitez la première personne : le lien crée un compte membre, une seule fois.</p>
                <button class="btn primary" on:click={() => openInvite()}>
                  <Icon name="personAdd" size={16} />
                  Inviter quelqu'un
                </button>
              {/if}
            </section>
          {:else if desktop}
            <div class="board">
              <table>
                <caption>Comptes et invitations du serveur</caption>
                <colgroup>
                  <col />
                  <col class="c-role" />
                  <col class="c-state" />
                  <col class="c-when" />
                  <col class="c-act" />
                </colgroup>
                <thead>
                  <tr>
                    <th scope="col">Nom</th>
                    <th scope="col">Rôle</th>
                    <th scope="col">État</th>
                    <th scope="col">Connexion / expiration</th>
                    <th scope="col" aria-label="Actions"></th>
                  </tr>
                </thead>
                <tbody>
                  {#each visibleUsers as row (row.kind + ":" + row.id)}
                    <tr>
                      <td>
                        <button class="who" on:click={() => openDetail(row)}>
                          <Avatar name={row.name} size={36} />
                          <span class="who-text">
                            <span class="who-line">
                              <span class="nm" title={row.name}>{row.name}</span>
                              {#if isMe(row)}<span class="you">Vous</span>{/if}
                            </span>
                          </span>
                        </button>
                      </td>
                      <td>
                        <span class="role" class:admin={row.admin}>
                          {#if row.admin}<Icon name="shield" size={14} />{/if}
                          {roleLabel(row)}
                        </span>
                      </td>
                      <td>
                        <span class="state {stateKey(row)}">
                          <Icon name={stateIcon(row)} size={15} />
                          {stateLabel(row)}
                        </span>
                      </td>
                      <td class="when">
                        <span title={rowSeenTitle(row)}>{lastSeenLabel(row)}</span>
                      </td>
                      <td class="act">
                        <button
                          class="more"
                          aria-label={rowAria(row)}
                          title={rowAria(row)}
                          on:click={() => openDetail(row)}
                        >
                          <Icon name="more" size={18} />
                        </button>
                      </td>
                    </tr>
                  {/each}
                  {#if visibleInvites.length}
                    {#if showGroup}
                      <tr class="group-row">
                        <td colspan="5"><span class="label-mono">Invitations en attente</span></td>
                      </tr>
                    {/if}
                    {#each visibleInvites as row (row.kind + ":" + row.id)}
                      <tr>
                        <td>
                          <button class="who" on:click={() => openDetail(row)}>
                            <Avatar name={row.name} size={36} />
                            <span class="who-text">
                              <span class="who-line">
                                <span class="nm" title={row.name}>{row.name}</span>
                              </span>
                              <span class="who-sub">Invitation en attente</span>
                            </span>
                          </button>
                        </td>
                        <td>
                          <span class="role invite-role" title="Rôle à la création">Membre</span>
                        </td>
                        <td>
                          <span class="state {stateKey(row)}">
                            <Icon name={stateIcon(row)} size={15} />
                            {stateLabel(row)}
                          </span>
                        </td>
                        <td class="when">
                          <span title={rowSeenTitle(row)}>{expiryLabel(row.expires)}</span>
                        </td>
                        <td class="act">
                          <button
                            class="more"
                            aria-label={rowAria(row)}
                            title={rowAria(row)}
                            on:click={() => openDetail(row)}
                          >
                            <Icon name="more" size={18} />
                          </button>
                        </td>
                      </tr>
                    {/each}
                  {/if}
                </tbody>
              </table>
            </div>
          {:else}
            <div class="cards">
              {#each visibleUsers as row (row.kind + ":" + row.id)}
                <button class="row-card" on:click={() => openDetail(row)} aria-label={rowAria(row)}>
                  <Avatar name={row.name} size={40} />
                  <span class="card-main">
                    <span class="card-top">
                      <span class="nm" title={row.name}>{row.name}</span>
                      {#if isMe(row)}<span class="you">Vous</span>{/if}
                      <span class="chev"><Icon name="chevronRight" size={17} /></span>
                    </span>
                    <span class="card-mid">
                      <span class="role" class:admin={row.admin}>
                        {#if row.admin}<Icon name="shield" size={13} />{/if}
                        {roleLabel(row)}
                      </span>
                      <span class="state {stateKey(row)}">
                        <Icon name={stateIcon(row)} size={14} />
                        {stateLabel(row)}
                      </span>
                    </span>
                    <span class="card-sub" title={rowSeenTitle(row)}>{rowSeen(row)}</span>
                  </span>
                </button>
              {/each}
              {#if visibleInvites.length}
                {#if showGroup}
                  <div class="group-card"><span class="label-mono">Invitations en attente</span></div>
                {/if}
                {#each visibleInvites as row (row.kind + ":" + row.id)}
                  <button class="row-card" on:click={() => openDetail(row)} aria-label={rowAria(row)}>
                    <Avatar name={row.name} size={40} />
                    <span class="card-main">
                      <span class="card-top">
                        <span class="nm" title={row.name}>{row.name}</span>
                        <span class="chev"><Icon name="chevronRight" size={17} /></span>
                      </span>
                      <span class="card-mid">
                        <span class="role invite-role">Membre</span>
                        <span class="state {stateKey(row)}">
                          <Icon name={stateIcon(row)} size={14} />
                          {stateLabel(row)}
                        </span>
                      </span>
                      <span class="card-sub" title={rowSeenTitle(row)}>{rowSeen(row)}</span>
                    </span>
                  </button>
                {/each}
              {/if}
            </div>
          {/if}
        </div>

        {#if invitesLoading}
          <p class="load-line label-mono">Chargement des invitations…</p>
        {/if}
        {#if invitesError}
          <div class="banner error" role="alert">
            <Icon name="alert" size={16} />
            <span>Les invitations n'ont pas pu être chargées. {guardMessage(invitesError)}</span>
            <button class="inline-action" on:click={() => loadInvites(true)}>Réessayer</button>
          </div>
        {/if}

        {#if moreError}
          <div class="banner error" role="alert">
            <Icon name="alert" size={16} />
            <span>Impossible de charger la suite. {guardMessage(moreError)}</span>
            <button class="inline-action" on:click={loadMore}>Réessayer</button>
          </div>
        {:else if showMore}
          <div class="more-row">
            <span class="range label-mono">{rangeLabel}</span>
            <button class="btn ghost load-more" on:click={loadMore} disabled={loadingMore}>
              {loadingMore ? "Chargement…" : "Charger plus"}
            </button>
          </div>
        {:else if visible.length && (filter !== "all" || rows.length)}
          <p class="range label-mono end-range">{rangeLabel}</p>
        {/if}
        {#if capped}
          <p class="search-hint">
            {MAX_ROWS} comptes chargés. Affinez la recherche pour trouver les autres.
          </p>
        {/if}
      {/if}
    {/if}
  </div>
{/if}

{#if detail}
  <div class="overlay" role="presentation" use:toBody transition:veil={{ for: "dialog" }}>
    <div
      class="dialog"
      role="dialog"
      transition:dialog
      aria-modal="true"
      aria-labelledby="admin-detail-title"
      tabindex="-1"
      bind:this={detailEl}
    >
      <header class="dialog-head">
        <Avatar name={detail.name} size={48} />
        <div class="dialog-title">
          <h2 id="admin-detail-title">{detail.name}</h2>
          <p class="dialog-sub">
            {roleLabel(detail)} · {stateLabel(detail)}
            {#if isMe(detail)}<span class="you">Vous</span>{/if}
          </p>
        </div>
        <button class="icon-btn close" on:click={closeDetail} aria-label="Fermer">
          <Icon name="close" size={18} />
        </button>
      </header>

      {#if detail.kind === "invite"}
        <dl class="facts">
          <div>
            <dt>Créée par</dt>
            <dd>{detail.created_by || "—"}</dd>
          </div>
          <div>
            <dt>Créée le</dt>
            <dd>{absoluteTime(detail.created)}</dd>
          </div>
          <div>
            <dt>Expire le</dt>
            <dd>{absoluteTime(detail.expires)} <span class="muted">({expiryLabel(detail.expires)})</span></dd>
          </div>
        </dl>
        <p class="note">Les liens d'invitation ne sont plus affichés après leur création.</p>
        {#if !$online}
          <p class="guard">Vous êtes hors ligne : cette action demande le serveur.</p>
        {/if}
        <div class="actions">
          {#if linkFor(detail)}
            <button class="action" on:click={() => copyLink(detail)}>
              <span class="a-ico"><Icon name="copy" size={18} /></span>
              <span class="a-txt">
                <span class="a-title">Copier le lien</span>
                <span class="a-desc">Le lien n'est affiché qu'à sa création. Il ne sert qu'à un compte.</span>
              </span>
              <span class="chev"><Icon name="chevronRight" size={18} /></span>
            </button>
          {:else}
            <div class="action disabled" aria-disabled="true">
              <span class="a-ico"><Icon name="link" size={18} /></span>
              <span class="a-txt">
                <span class="a-title">Lien non disponible</span>
                <span class="a-desc">Le lien n'est plus affiché. Renvoyez l'invitation pour en créer un nouveau.</span>
              </span>
            </div>
          {/if}
          <button class="action" on:click={() => openInvite(detail)} disabled={!$online}>
            <span class="a-ico"><Icon name="refresh" size={18} /></span>
            <span class="a-txt">
              <span class="a-title">Renvoyer l'invitation</span>
              <span class="a-desc">Révoque l'ancien lien et en crée un nouveau.</span>
            </span>
            <span class="chev"><Icon name="chevronRight" size={18} /></span>
          </button>
        </div>
        <div class="danger">
          <div class="label-mono">Zone dangereuse</div>
          <button class="action" on:click={() => askRevoke(detail)} disabled={!$online}>
            <span class="a-ico"><Icon name="minusCircle" size={18} /></span>
            <span class="a-txt">
              <span class="a-title">Révoquer l'invitation</span>
              <span class="a-desc">Le lien cesse de fonctionner immédiatement.</span>
            </span>
            <span class="chev"><Icon name="chevronRight" size={18} /></span>
          </button>
        </div>
      {:else}
        <dl class="facts">
          <div>
            <dt>Dernière connexion</dt>
            <dd>{absoluteTime(detail.last_login)} <span class="muted">({relativeTime(detail.last_login)})</span></dd>
          </div>
          <div>
            <dt>Adresse e-mail</dt>
            <dd>{detail.mail || "—"}</dd>
          </div>
          <div>
            <dt>État</dt>
            <dd>{stateLabel(detail)}</dd>
          </div>
        </dl>
        {#if isMe(detail)}
          <p class="guard">Vous ne pouvez pas modifier votre propre compte ici. Demandez à un autre administrateur.</p>
        {:else if !$online}
          <p class="guard">Vous êtes hors ligne : ces actions demandent le serveur.</p>
        {/if}
        <div class="actions">
          <button
            class="action"
            on:click={() => askRole(detail)}
            disabled={isMe(detail) || detailBusy || !$online}
          >
            <span class="a-ico"><Icon name="shield" size={18} /></span>
            <span class="a-txt">
              <span class="a-title">
                {detail.admin ? "Retirer les droits d'administrateur" : "Nommer administrateur"}
              </span>
              <span class="a-desc">
                {detail.admin
                  ? "Garde le compte, perd les réglages du serveur et les comptes."
                  : "Accès aux réglages du serveur et à tous les comptes."}
              </span>
            </span>
            <span class="chev"><Icon name="chevronRight" size={18} /></span>
          </button>
          <button
            class="action"
            on:click={() => askDisable(detail)}
            disabled={isMe(detail) || detailBusy || !$online}
          >
            <span class="a-ico"><Icon name={detail.disabled ? "refresh" : "minusCircle"} size={18} /></span>
            <span class="a-txt">
              <span class="a-title">{detail.disabled ? "Réactiver le compte" : "Désactiver le compte"}</span>
              <span class="a-desc">
                {detail.disabled
                  ? "La personne pourra se reconnecter."
                  : "Déconnecte la personne partout. Ses données restent."}
              </span>
            </span>
            <span class="chev"><Icon name="chevronRight" size={18} /></span>
          </button>
        </div>
        {#if actionError}
          <p class="inline-error" role="alert"><Icon name="alert" size={15} />{actionError}</p>
        {/if}
        <div class="danger">
          <div class="label-mono">Zone dangereuse</div>
          <button
            class="action"
            on:click={() => askDelete(detail)}
            disabled={isMe(detail) || detailBusy || !$online}
          >
            <span class="a-ico"><Icon name="trash" size={18} /></span>
            <span class="a-txt">
              <span class="a-title">Supprimer le compte</span>
              <span class="a-desc">Définitif. Ses playlists et son historique partent ; l'audio archivé reste.</span>
            </span>
            <span class="chev"><Icon name="chevronRight" size={18} /></span>
          </button>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if confirm}
  <div class="overlay" role="presentation" use:toBody transition:veil={{ for: "dialog" }}>
    <div
      class="dialog confirm-dialog"
      role="dialog"
      transition:dialog
      aria-modal="true"
      aria-labelledby="admin-confirm-title"
      aria-describedby="admin-confirm-body"
      tabindex="-1"
      bind:this={confirmEl}
    >
      <h2 id="admin-confirm-title">{confirm.title}</h2>
      <p id="admin-confirm-body" class="confirm-body">{confirm.body}</p>

      {#if confirm.kind === "delete"}
        {#if confirm.loading}
          <p class="load-line label-mono">Chargement des fichiers et des playlists…</p>
        {:else if confirm.uploads !== null}
          <dl class="facts compact">
            <div>
              <dt>Fichiers importés</dt>
              <dd>
                {countLabel(confirm.uploads, "fichier")}
                {#if confirm.uploads}
                  <span class="muted">· attribués à votre compte, aucun fichier supprimé</span>
                {/if}
              </dd>
            </div>
            <div>
              <dt>Playlists</dt>
              <dd>{countLabel(confirm.playlists ?? 0, "playlist")}</dd>
            </div>
          </dl>
        {/if}
        <label class="typed-label" for="admin-confirm-typed">
          Tapez <strong>{confirm.row.name}</strong> pour confirmer
        </label>
        <input
          id="admin-confirm-typed"
          class="typed"
          bind:this={typedEl}
          bind:value={confirm.typed}
          autocomplete="off"
          spellcheck="false"
          aria-describedby="admin-confirm-body"
        />
      {/if}

      {#if confirm.error}
        <p class="inline-error" role="alert">
          <Icon name="alert" size={15} />
          <span>{confirm.error}</span>
          {#if confirm.kind === "delete" && confirm.uploads === null}
            <button class="inline-action" on:click={loadDeleteInfo}>Réessayer</button>
          {/if}
        </p>
      {/if}

      <div class="dialog-actions">
        <button class="btn ghost" bind:this={cancelEl} on:click={() => closeConfirm(true)}>
          Annuler
        </button>
        <button
          class="btn danger"
          on:click={runConfirm}
          disabled={confirm.busy ||
            confirm.loading ||
            (confirm.kind === "delete" &&
              (confirm.typed !== confirm.row.name || confirm.uploads === null))}
        >
          {#if confirm.busy}
            <span>En cours…</span>
          {:else}
            <Icon name={confirm.kind === "delete" || confirm.kind === "revoke" ? "trash" : "check"} size={16} />
            <span>{confirm.label}</span>
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if linkFallback}
  <div class="overlay" role="presentation" use:toBody transition:veil={{ for: "dialog" }}>
    <div
      class="dialog link-dialog"
      role="dialog"
      transition:dialog
      aria-modal="true"
      aria-labelledby="admin-link-title"
      tabindex="-1"
      bind:this={linkFallbackEl}
    >
      <h2 id="admin-link-title">Copiez le lien manuellement</h2>
      <p class="confirm-body">La copie automatique a échoué. Sélectionnez le lien ci-dessous et copiez-le.</p>
      <input
        class="typed mono"
        readonly
        bind:this={linkFieldEl}
        value={linkFallback.url}
        on:focus={(event) => {
          event.currentTarget.select();
          event.currentTarget.scrollLeft = 0;
        }}
      />
      <div class="dialog-actions">
        <button class="btn ghost" on:click={closeLink}>Fermer</button>
        <button class="btn primary" on:click={async () => { if (await copyText(linkFallback.url)) toasts.push("Lien copié"); }}>
          <Icon name="copy" size={16} />
          Copier le lien
        </button>
      </div>
    </div>
  </div>
{/if}

{#if inviteSheet}
  <InviteSheet
    source={inviteSheet.source}
    on:close={closeInvite}
    on:created={onInviteCreated}
    on:revoked={onInviteRevoked}
    on:stale={onInviteStale}
  />
{/if}

<style>
  .admin {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .admin-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 14px;
  }
  .head-copy {
    min-width: 0;
  }
  .lede {
    margin: 0 0 5px;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.4;
  }
  .count {
    margin: 0;
    color: var(--dim);
    line-height: 1.5;
  }
  .fresh {
    text-transform: none;
    letter-spacing: 0;
    color: var(--dim);
    opacity: 0.8;
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: none;
  }
  .invite-short {
    display: none;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-height: 44px;
    padding: 0 16px;
    border-radius: 999px;
    border: 1px solid transparent;
    background: var(--s2);
    color: var(--tx);
    font-size: 14px;
    font-weight: 600;
    line-height: 1;
    white-space: nowrap;
  }
  .btn.primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .btn.ghost {
    background: transparent;
    border-color: var(--ln);
  }
  .btn.danger {
    background: transparent;
    border-color: var(--ln);
    color: var(--tx);
  }
  .btn:hover:not(:disabled) {
    border-color: var(--ac-text);
  }
  .btn:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    flex: none;
    border-radius: 50%;
    background: var(--s2);
    color: var(--tx);
  }
  .icon-btn:hover:not(:disabled) {
    color: var(--ac-text);
  }
  .icon-btn:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 0 0 12px;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
    color: var(--dim);
    font-size: 13px;
    line-height: 1.4;
  }
  .banner.error {
    color: var(--tx);
  }
  .banner.error :global(svg) {
    color: var(--ac-text);
  }
  .inline-action {
    margin-left: auto;
    min-height: 44px;
    padding: 0 8px;
    color: var(--ac-text);
    font-size: 13px;
    font-weight: 600;
    text-decoration: underline;
    white-space: nowrap;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 44px;
    padding: 0 4px 0 14px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: var(--s1);
    color: var(--dim);
  }
  .search:focus-within {
    border-color: var(--ac-text);
    color: var(--tx);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: none;
    color: var(--tx);
    font: inherit;
    font-size: 14px;
    outline: none;
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .clear {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    flex: none;
    border-radius: 50%;
    color: var(--dim);
  }
  .clear:hover {
    background: var(--s2);
    color: var(--tx);
  }
  .search-hint {
    margin: 7px 2px 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.4;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 12px 0 2px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .filters::-webkit-scrollbar {
    display: none;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 40px;
    padding: 0 14px;
    flex: none;
    border: 1px solid var(--ln);
    border-radius: 999px;
    background: transparent;
    color: var(--dim);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .chip:hover {
    color: var(--tx);
    border-color: var(--s2);
  }
  .chip.on {
    background: var(--ac);
    border-color: var(--ac);
    color: var(--onac);
    font-weight: 700;
  }
  .chip-n {
    font-family: var(--font-mono);
    font-size: 11px;
    opacity: 0.72;
  }
  .clear-filters {
    min-height: 40px;
    padding: 0 8px;
    flex: none;
    color: var(--ac-text);
    font-size: 13px;
    font-weight: 600;
    text-decoration: underline;
    white-space: nowrap;
  }
  .list-wrap {
    margin-top: 10px;
  }
  .board {
    border: 1px solid var(--ln);
    border-radius: var(--r-card);
    background: var(--s1);
    overflow: visible;
  }
  table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 0;
    table-layout: fixed;
  }
  caption {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
  .c-role {
    width: 136px;
  }
  .c-state {
    width: 122px;
  }
  .c-when {
    width: 154px;
  }
  .c-act {
    width: 56px;
  }
  thead th {
    position: sticky;
    top: var(--topbar-h, 68px);
    z-index: 2;
    height: 42px;
    padding: 0 12px;
    background: var(--s1);
    border-bottom: 1px solid var(--ln);
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    text-align: left;
    white-space: nowrap;
  }
  thead th:first-child {
    border-radius: var(--r-card) 0 0 0;
  }
  thead th:last-child {
    border-radius: 0 var(--r-card) 0 0;
  }
  tbody td {
    height: 56px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--ln);
    vertical-align: middle;
  }
  tbody tr:last-child td {
    border-bottom: none;
  }
  tbody tr:last-child td:first-child {
    border-radius: 0 0 0 var(--r-card);
  }
  tbody tr:last-child td:last-child {
    border-radius: 0 0 var(--r-card) 0;
  }
  tbody tr:hover {
    background: var(--s2);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    min-height: 44px;
    text-align: left;
    color: var(--tx);
    min-width: 0;
  }
  .who:hover .nm {
    color: var(--ac-text);
  }
  .who-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .who-line {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }
  .nm {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-display);
    font-size: 14.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .who-sub {
    color: var(--dim);
    font-size: 12px;
  }
  .you {
    display: inline-flex;
    align-items: center;
    min-height: 18px;
    padding: 0 6px;
    border: 1px solid var(--ln);
    border-radius: 999px;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .role {
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    gap: 5px;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.3;
    white-space: nowrap;
  }
  .role.admin {
    color: var(--tx);
    font-weight: 600;
  }
  .invite-role {
    color: var(--dim);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.3;
    white-space: nowrap;
  }
  .state-block {
    padding: 24px 0;
    text-align: center;
  }
  .state-block.empty {
    display: block;
    padding: 34px 16px;
    text-align: center;
  }
  .state.active {
    color: var(--tx);
  }
  .state.disabled,
  .state.expired,
  .state.revoked {
    color: var(--dim);
  }
  .when {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .act {
    text-align: right;
  }
  .more {
    display: inline-grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    color: var(--dim);
  }
  .more:hover {
    background: var(--bg);
    color: var(--tx);
  }
  .group-row td {
    height: 38px;
    padding: 0 12px;
    background: var(--s1);
    border-bottom: 1px solid var(--ln);
  }
  .cards {
    border-top: 1px solid var(--ln);
  }
  .row-card {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    width: 100%;
    min-height: 88px;
    padding: 14px 12px;
    border-bottom: 1px solid var(--ln);
    text-align: left;
    color: var(--tx);
  }
  .row-card:hover,
  .row-card:focus-visible {
    background: var(--s1);
  }
  .card-main {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .card-top .nm {
    font-size: 16px;
  }
  .chev {
    margin-left: auto;
    color: var(--dim);
    flex: none;
  }
  .card-mid {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .card-sub {
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.4;
  }
  .group-card {
    padding: 12px;
    border-bottom: 1px solid var(--ln);
  }
  .state-ico {
    display: grid;
    place-items: center;
    width: 58px;
    height: 58px;
    margin: 0 auto 12px;
    border: 1px solid var(--ln);
    border-radius: 50%;
    background: var(--s1);
    color: var(--dim);
  }
  .state-block h2 {
    margin: 0 0 6px;
    font-family: var(--font-display);
    font-size: 21px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .state-block p {
    max-width: 440px;
    margin: 0 auto 14px;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.5;
  }
  .state-block .btn {
    margin: 3px 3px 0;
  }
  .load-line {
    margin: 12px 2px 0;
  }
  .more-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-top: 12px;
  }
  .range {
    margin: 0;
  }
  .end-range {
    margin: 10px 2px 0;
  }
  .load-more {
    min-width: 132px;
  }
  .skeleton-board {
    padding: 4px 0;
  }
  .sk-row {
    display: grid;
    grid-template-columns: 48px minmax(120px, 1fr) 92px 108px 92px;
    align-items: center;
    gap: 12px;
    height: 56px;
    padding: 0 12px;
    border-bottom: 1px solid var(--ln);
  }
  .skeleton-cards {
    border-top: 1px solid var(--ln);
  }
  .sk-card {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 88px;
    padding: 14px 12px;
    border-bottom: 1px solid var(--ln);
  }
  .sk-lines {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1;
  }
  .sk {
    display: block;
    height: 12px;
    border-radius: 6px;
    background: var(--s2);
  }
  .sk.avatar {
    width: 38px;
    height: 38px;
    border-radius: 50%;
  }
  .sk.line.name {
    width: 42%;
    height: 14px;
  }
  .sk.line.small {
    width: 26%;
  }
  .sk.line.tiny {
    width: 34%;
  }
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 430;
    display: grid;
    place-items: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .dialog {
    width: min(480px, 100%);
    max-height: min(720px, calc(100dvh - 40px));
    box-sizing: border-box;
    overflow-y: auto;
    padding: 24px;
    border: 1px solid var(--ln);
    border-radius: 24px;
    background: var(--s1);
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
  }
  .dialog:focus {
    outline: none;
  }
  .dialog-head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 20px;
  }
  .dialog-title {
    min-width: 0;
    flex: 1;
  }
  .dialog h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 23px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.12;
    overflow-wrap: anywhere;
  }
  .dialog-sub {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    margin: 5px 0 0;
    color: var(--dim);
    font-size: 13px;
  }
  .close {
    margin: -6px -6px 0 0;
  }
  .facts {
    display: grid;
    gap: 12px;
    margin: 0 0 16px;
  }
  .facts.compact {
    gap: 7px;
    margin: 14px 0;
    padding: 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
  }
  .facts div {
    min-width: 0;
  }
  .facts dt {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    letter-spacing: 0.07em;
    text-transform: uppercase;
  }
  .facts dd {
    margin: 3px 0 0;
    font-size: 14px;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--dim);
  }
  .note {
    margin: 0 0 16px;
    padding: 10px 12px;
    border-radius: 12px;
    background: var(--s2);
    color: var(--dim);
    font-size: 13px;
    line-height: 1.45;
  }
  .guard {
    margin: 0 0 14px;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.45;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .action {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    min-height: 64px;
    padding: 10px 12px;
    border: 1px solid transparent;
    border-radius: 14px;
    background: var(--s2);
    text-align: left;
    color: var(--tx);
  }
  .action:hover:not(:disabled):not(.disabled) {
    border-color: var(--ac-text);
  }
  .action:disabled,
  .action.disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .a-ico {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 50%;
    background: var(--bg);
    color: var(--ac-text);
  }
  .a-txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .a-title {
    font-size: 14.5px;
    font-weight: 600;
    line-height: 1.3;
  }
  .a-desc {
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.4;
  }
  .danger {
    margin-top: 18px;
    padding-top: 16px;
    border-top: 1px solid var(--ln);
  }
  .danger > .label-mono {
    display: block;
    margin-bottom: 8px;
  }
  .inline-error {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 12px 0 0;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    color: var(--tx);
    font-size: 13px;
    line-height: 1.45;
  }
  .inline-error :global(svg) {
    color: var(--ac-text);
    flex: none;
    margin-top: 1px;
  }
  .confirm-body {
    margin: 10px 0 14px;
    color: var(--dim);
    font-size: 14px;
    line-height: 1.55;
  }
  .typed-label {
    display: block;
    margin: 12px 0 6px;
    color: var(--dim);
    font-size: 12.5px;
  }
  .typed-label strong {
    color: var(--tx);
    font-family: var(--font-mono);
    font-weight: 600;
  }
  .typed {
    width: 100%;
    height: 46px;
    box-sizing: border-box;
    padding: 0 12px;
    border: 1px solid var(--ln);
    border-radius: 10px;
    background: var(--bg);
    color: var(--tx);
    font: inherit;
    font-size: 14px;
  }
  .typed:focus-visible {
    outline: none;
    border-color: var(--ac-text);
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 22px;
    flex-wrap: wrap;
  }
  .link-dialog .typed {
    font-size: 12.5px;
  }
  @media (pointer: coarse) {
    .chip,
    .clear-filters {
      min-height: 44px;
    }
  }
  @media (max-width: 899px) {
    .admin-head {
      flex-direction: column;
      align-items: stretch;
      gap: 10px;
    }
    .head-actions {
      justify-content: flex-end;
    }
    .lede {
      display: none;
    }
    .invite-long {
      display: none;
    }
    .invite-short {
      display: inline;
    }
    .invite {
      padding: 0 14px;
    }
  }
  @media (max-width: 640px) {
    .overlay {
      align-items: end;
      padding: 0;
    }
    .dialog {
      width: 100%;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      padding: 20px 20px calc(20px + env(safe-area-inset-bottom));
    }
    .more-row {
      align-items: stretch;
      flex-direction: column-reverse;
    }
    .load-more {
      width: 100%;
    }
  }
</style>
