<script>
  // Collaborators of a playlist. The list paints from the page's payload (no
  // request on open); every mutation carries `members_version`, and a 409 makes
  // ONE members GET then says so — never a merge, never a replay. No account
  // directory: an invite is one exact name.
  import { onMount, tick } from "svelte";
  import { sheet as sheetIn, veil } from "../lib/motion.js";
  import { toBody } from "../lib/portal.js";
  import { api } from "../lib/api.js";
  import { conflictOf, memberError, resyncOnce, roleLabel } from "../lib/actions.js";
  import { online } from "../lib/net.js";
  import { REMOTE } from "../lib/remote/mode.js";
  import Avatar from "./Avatar.svelte";
  import Icon from "./Icon.svelte";
  import VirtualList from "./VirtualList.svelte";

  export let id = null;
  export let title = "";
  export let members = [];
  export let membersVersion = 0;
  export let role = null;
  export let canManage = false;
  export let admin = false;
  export let privateTracks = 0;
  export let me = "";
  export let onclose = null;
  export let onchange = null;
  export let ongone = null;
  export let onleave = null;

  let sheet = null;
  let list = [];
  let version = 0;
  let seen = null;
  let username = "";
  let newRole = "editor";
  let busy = ""; // "" | "invite" | "leave" | "member:<id>"
  let error = "";
  let revoked = false;
  let returnFocus = null;

  $: if (members !== seen) {
    seen = members;
    list = Array.isArray(members) ? members.slice() : [];
    version = membersVersion;
  }
  $: ownerRow = list.find((m) => m.role === "owner") || list[0] || null;
  $: others = list.filter((m) => m !== ownerRow);
  $: managed = canManage && !revoked && !REMOTE;
  $: canLeave = !REMOTE && (role === "editor" || role === "viewer");
  $: remote = !!REMOTE;

  onMount(() => {
    returnFocus = typeof document !== "undefined" ? document.activeElement : null;
    tick().then(() => sheet?.focus?.({ preventScroll: true }));
  });

  function close() {
    const back = returnFocus;
    returnFocus = null;
    onclose?.();
    tick().then(() => {
      if (back && back.isConnected) back.focus?.({ preventScroll: true });
    });
  }

  function onKey(e) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
      return;
    }
    if (e.key !== "Tab" || !sheet) return;
    const focusable = [
      ...sheet.querySelectorAll("button, input, select, [href], [tabindex]:not([tabindex='-1'])"),
    ].filter((el) => !el.disabled && el.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (e.shiftKey && (active === first || active === sheet || !sheet.contains(active))) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }

  // Adopt the authoritative answer, and every mounted copy of it.
  function adopt(r) {
    if (!r) return;
    if (Array.isArray(r.members)) list = r.members;
    if (r.members_version != null) version = r.members_version;
    // The parent hands the same array back: do not rebuild for nothing.
    seen = list;
    onchange?.({ members: list, members_version: version });
  }

  function handle(err, who = "") {
    error = memberError(err, who);
    if (conflictOf(err)) {
      resyncOnce(`${id}:members`, async () => adopt(await api.playlistMembers(id))).catch(() => {});
      return;
    }
    if (err?.status === 403) {
      // One small GET refreshes the page's capabilities too.
      revoked = true;
      resyncOnce(`${id}:members`, async () => {
        const r = await api.playlistMembers(id);
        adopt(r);
        onchange?.({
          members: list,
          members_version: version,
          role: r.role,
          can_edit: r.can_edit,
          can_manage: r.can_manage,
          admin: r.admin,
        });
      }).catch(() => {});
      return;
    }
    if (err?.status === 404 && !(err.body && err.body.error === "unknown user")) ongone?.();
  }

  async function invite() {
    const name = username.trim();
    if (!name) {
      error = "Indiquez un nom d'utilisateur.";
      return;
    }
    if (busy || !managed || !$online) return;
    busy = "invite";
    error = "";
    try {
      adopt(await api.invitePlaylistMember(id, name, newRole, version));
      username = "";
      newRole = "editor";
      tick().then(() => sheet?.querySelector(".invite input")?.focus?.({ preventScroll: true }));
    } catch (e) {
      handle(e, name);
    } finally {
      busy = "";
    }
  }

  async function setRole(member, next) {
    if (!managed || busy || !next || next === member.role) return;
    const previous = member.role;
    busy = "member:" + member.id;
    error = "";
    member.role = next;
    list = list;
    try {
      adopt(await api.setPlaylistMemberRole(id, member.id, next, version));
    } catch (e) {
      const row = list.find((m) => String(m.id) === String(member.id));
      if (row) row.role = previous;
      list = list;
      handle(e);
    } finally {
      busy = "";
    }
  }

  async function removeMember(member) {
    if (!managed || busy) return;
    const previous = list;
    busy = "member:" + member.id;
    error = "";
    list = list.filter((m) => m !== member);
    try {
      adopt(await api.removePlaylistMember(id, member.id, version));
    } catch (e) {
      list = previous;
      handle(e);
    } finally {
      busy = "";
    }
  }

  async function leave() {
    if (!canLeave || busy) return;
    busy = "leave";
    error = "";
    try {
      await api.leavePlaylist(id);
      onleave?.();
    } catch (e) {
      if (e?.status === 409) error = "Le propriétaire ne peut pas quitter sa playlist.";
      else if (e?.status === 404) ongone?.();
      else error = memberError(e);
    } finally {
      busy = "";
    }
  }

  // A fixed overlay inside a transformed ancestor is fixed to it: it leaves
  // the page's subtree (lib/portal.js#toBody, M9).
</script>

<svelte:window on:keydown={onKey} />

{#snippet personRow(m)}
  <div class="person" data-member={m.id}>
    <Avatar name={m.name} size={40} />
    <span class="who">
      <span class="nm">{m.name}</span>
      {#if m.name === me}<span class="you">Vous</span>{/if}
    </span>
    {#if managed}
      <span class="pick">
        <select
          class="rolesel"
          value={m.role}
          aria-label={`Rôle de ${m.name}`}
          disabled={busy === "member:" + m.id || !$online}
          on:change={(e) => setRole(m, e.currentTarget.value)}
        >
          <option value="editor">Peut modifier</option>
          <option value="viewer">Lecture seule</option>
        </select>
        <Icon name="chevronDown" size={14} />
      </span>
      <button
        type="button"
        class="remove"
        on:click={() => removeMember(m)}
        disabled={busy === "member:" + m.id || !$online}
        aria-label={`Retirer ${m.name}`}
        title="Retirer"
      >
        <Icon name="trash" size={17} />
      </button>
    {:else}
      <span class="fixed-role mono">{roleLabel(m.role)}</span>
    {/if}
  </div>
{/snippet}

{#if id}
  <!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
  <div class="overlay" use:toBody transition:veil={{ for: "sheet" }} on:click|self={close}>
    <div
      class="sheet"
      role="dialog"
      aria-modal="true"
      aria-labelledby="members-title"
      tabindex="-1"
      bind:this={sheet}
      transition:sheetIn
    >
      <header>
        <div class="ttl">
          <h2 id="members-title">Collaborateurs</h2>
          <p class="sub">{title}</p>
          {#if role === "viewer" && !canManage}
            <span class="badge mono">Lecture seule</span>
          {:else if admin && role === "admin"}
            <span class="badge mono">Administrateur</span>
          {/if}
        </div>
        <button type="button" class="close" on:click={close} aria-label="Fermer">
          <Icon name="close" size={20} />
        </button>
      </header>

      <div class="body">
        {#if remote}
          <p class="note">Gérez les collaborateurs depuis votre propre appareil.</p>
        {:else}
          {#if ownerRow}
            <div class="person owner" data-member={ownerRow.id}>
              <Avatar name={ownerRow.name} size={40} />
              <span class="who">
                <span class="nm">{ownerRow.name}</span>
                {#if ownerRow.name === me}<span class="you">Vous</span>{/if}
              </span>
              <span class="fixed-role mono">Propriétaire</span>
            </div>
          {/if}

          {#if others.length}
            <div class="people">
              {#if others.length > 24}
                <VirtualList items={others} estimateHeight={56} key={(m) => String(m.id)} let:item>
                  {@render personRow(item)}
                </VirtualList>
              {:else}
                {#each others as m (m.id)}
                  {@render personRow(m)}
                {/each}
              {/if}
            </div>
          {:else if !ownerRow}
            <p class="note">La liste des collaborateurs est vide.</p>
          {/if}

          {#if managed}
            <form class="invite" on:submit|preventDefault={invite}>
              <label class="label-mono" for="member-name">Inviter un compte</label>
              <input
                id="member-name"
                class="nameinput"
                type="text"
                bind:value={username}
                placeholder="Nom d'utilisateur"
                autocomplete="off"
                autocapitalize="off"
                spellcheck="false"
                maxlength="64"
              />
              <div class="seg" role="group" aria-label="Rôle du collaborateur">
                <button
                  type="button"
                  class="chip"
                  class:chip-active={newRole === "editor"}
                  aria-pressed={newRole === "editor"}
                  on:click={() => (newRole = "editor")}>Peut modifier</button>
                <button
                  type="button"
                  class="chip"
                  class:chip-active={newRole === "viewer"}
                  aria-pressed={newRole === "viewer"}
                  on:click={() => (newRole = "viewer")}>Lecture seule</button>
              </div>
              <p class="hint">La personne doit déjà avoir un compte sur ce serveur.</p>
              <button
                type="submit"
                class="btn btn-primary invitebtn"
                disabled={busy === "invite" || !$online || !username.trim()}
              >
                {busy === "invite" ? "Invitation…" : "Inviter"}
              </button>
            </form>
          {/if}

          {#if privateTracks > 0}
            <p class="note">
              <Icon name="eye" size={15} />
              <span>
                {privateTracks} titre{privateTracks > 1 ? "s" : ""} privé{privateTracks > 1 ? "s" : ""}
                ne ser{privateTracks > 1 ? "ont" : "a"} pas visible{privateTracks > 1 ? "s" : ""} par les autres.
              </span>
            </p>
          {/if}

          {#if !canManage && !remote}
            <p class="foot-note">Seul le propriétaire peut renommer ou supprimer cette playlist.</p>
          {/if}

          {#if canLeave}
            <button
              type="button"
              class="leave"
              on:click={leave}
              disabled={busy === "leave" || !$online}
            >
              <Icon name="logOut" size={16} />
              {busy === "leave" ? "Départ…" : "Quitter la playlist"}
            </button>
          {/if}

          {#if error}<p class="err" role="alert">{error}</p>{/if}
          {#if !$online && !error}<p class="foot-note">Hors ligne : les modifications attendront le réseau.</p>{/if}
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 410;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    background: color-mix(in srgb, var(--onac) 60%, transparent);
    overscroll-behavior: contain;
    touch-action: none;
  }
  .sheet {
    width: min(440px, 100%);
    max-width: 100%;
    max-height: min(720px, calc(100dvh - 40px));
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    background: var(--s1);
    border: 1px solid var(--ln);
    border-radius: 24px;
    color: var(--tx);
    box-shadow: 0 24px 64px color-mix(in srgb, var(--onac) 45%, transparent);
    touch-action: auto;
    overflow: hidden;
  }
  .sheet:focus {
    outline: none;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 22px 22px 12px;
    flex: none;
  }
  .ttl {
    flex: 1;
    min-width: 0;
  }
  .ttl h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 24px;
    font-weight: 700;
    letter-spacing: -0.02em;
    line-height: 1.1;
  }
  .sub {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    display: inline-block;
    margin-top: 6px;
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 3px 8px;
  }
  .close {
    flex: none;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 999px;
    background: var(--s2);
    color: var(--tx);
  }
  .close:hover {
    background: color-mix(in srgb, var(--tx) 12%, var(--s2));
  }
  .body {
    overflow-y: auto;
    min-height: 0;
    padding: 4px 22px calc(22px + env(safe-area-inset-bottom, 0px));
  }
  .person {
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 56px;
    padding: 6px 0;
  }
  .person.owner {
    border-bottom: 1px solid var(--ln);
    padding-bottom: 12px;
    margin-bottom: 4px;
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .nm {
    min-width: 0;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .you {
    flex: none;
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--dim);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 2px 6px;
  }
  .fixed-role {
    flex: none;
    font-size: 11px;
    color: var(--dim);
  }
  .people {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .pick {
    position: relative;
    display: inline-flex;
    align-items: center;
    flex: none;
  }
  .pick :global(svg) {
    position: absolute;
    right: 10px;
    color: var(--dim);
    pointer-events: none;
  }
  .rolesel {
    height: 40px;
    max-width: 148px;
    background: var(--s1);
    color: var(--tx);
    border: 1px solid var(--ln);
    border-radius: 999px;
    padding: 0 30px 0 12px;
    font: inherit;
    font-size: 13px;
    outline: none;
    cursor: pointer;
    appearance: none;
    -webkit-appearance: none;
  }
  .rolesel:focus-visible {
    border-color: var(--ac-text);
  }
  .remove {
    flex: none;
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: 999px;
    color: var(--dim);
  }
  .remove:hover:not(:disabled) {
    color: var(--tx);
    background: var(--s2);
  }
  .remove:disabled {
    opacity: 0.45;
  }
  .invite {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 18px;
    padding: 14px;
    border-radius: 16px;
    background: var(--s2);
  }
  .nameinput {
    width: 100%;
    height: 44px;
    box-sizing: border-box;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--tx);
    font-family: var(--font-body);
    font-size: 15px;
  }
  .nameinput::placeholder {
    color: var(--dim);
    opacity: 1;
  }
  .seg {
    display: flex;
    gap: 6px;
  }
  .seg .chip {
    flex: 1 1 0;
    min-height: 40px;
  }
  .hint {
    margin: 0;
    font-size: 12px;
    color: var(--dim);
    line-height: 1.45;
  }
  .invitebtn {
    align-self: flex-end;
    min-width: 120px;
  }
  .note {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin: 16px 0 0;
    color: var(--dim);
    font-size: 13px;
    line-height: 1.5;
  }
  .note :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .foot-note {
    margin: 16px 0 0;
    color: var(--dim);
    font-size: 12px;
    line-height: 1.5;
  }
  .leave {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-height: 44px;
    margin-top: 18px;
    padding: 0 14px;
    border-radius: 999px;
    border: 1px solid var(--ln);
    color: var(--tx);
    font-size: 14px;
    font-weight: 600;
  }
  .leave:hover:not(:disabled) {
    background: var(--s2);
  }
  .leave:disabled {
    opacity: 0.5;
  }
  .err {
    margin: 14px 0 0;
    padding: 10px 12px;
    border-radius: 12px;
    border: 1px solid color-mix(in srgb, #ff6b6b 45%, var(--ln));
    background: color-mix(in srgb, #ff6b6b 10%, var(--s1));
    color: var(--tx);
    font-size: 13px;
    line-height: 1.5;
  }

  @media (pointer: coarse) {
    .close {
      width: 44px;
      height: 44px;
    }
    .rolesel {
      height: 44px;
    }
    .seg .chip {
      min-height: 44px;
    }
  }

  @media (max-width: 640px) {
    .overlay {
      align-items: flex-end;
      padding: 0;
    }
    .sheet {
      width: 100%;
      max-height: 92dvh;
      border-radius: 24px 24px 0 0;
      border-bottom: none;
    }
    header {
      padding: 20px 18px 10px;
    }
    .body {
      padding: 4px 18px calc(18px + env(safe-area-inset-bottom, 0px));
    }
    .close {
      width: 44px;
      height: 44px;
    }
    .person {
      min-height: 60px;
    }
    .rolesel {
      max-width: 128px;
    }
  }
</style>
