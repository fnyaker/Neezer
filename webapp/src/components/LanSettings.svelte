<script>
  // Réglages → « Appareils et amis » (docs/agents/lan-nodes.md, *Server* and
  // *Screens and Réglages*; lot LN8).
  //
  // The server is the authority of the LAN exchange: its LAN key signs the
  // credentials each device presents to its peers, and its roster says who may
  // fetch what. This screen is the only place a person (or an administrator)
  // can act on that from a browser — the web app is not a node, so nothing here
  // ever talks to another device.
  //
  // Three blocks: my enrolled devices (rename, revoke), my trusted friends and
  // what each of them may see, and — for an administrator only — the feature
  // switch and the lifetime of a device credential (CLAUDE.md rule 6).
  //
  // Opening it costs 2 requests (`/api/lan/devices`, `/api/lan/friends`) plus
  // one for an admin (`/api/lan/settings`); the feature probe belongs to the
  // Réglages shell, which only draws the category once it has answered. The
  // friend picker reads `/me/playlists` from the SPA's offline cache, so it
  // costs a request only when the library has never been visited.
  //
  // Optimistic, always: a rename or a revoke paints in the frame the finger
  // left, and a refusal puts the old row back WITH its cause and « Réessayer »
  // (rule 3: never a spinner that never ends, never a silent failure).
  import { onMount } from "svelte";
  import { collapse } from "../lib/motion.js";
  import { user } from "../lib/stores.js";
  import Icon from "./Icon.svelte";
  import LoadFailed from "./LoadFailed.svelte";
  import * as lan from "../lib/lan.js";

  // A reactive VALUE, not a function: the template tracks it, so the server
  // card appears or goes with the account rather than on the next event.
  $: isAdmin = !!$user?.admin;

  let loading = true;
  let failed = null;
  let devices = [];
  let friends = null; // { shares_to, trusted_by }
  let feature = null; // the probe: {enabled, keys, credential_days}
  let settings = null; // admin only: {enabled, credential_days}
  let settingsError = "";
  let settingsAgain = null;
  let now = Date.now();

  let editing = null; // device id whose name is being typed
  let draft = "";
  let confirming = null; // device/friend id whose destructive button waits a second tap
  let picker = null; // friend user_id whose playlists are open
  let choices = null; // the person's playlists, from the offline cache
  let choicesError = "";
  let busy = {}; // id -> true while its request is in flight
  let rowError = {}; // id -> {cause, again}
  let adding = false;
  let newName = "";
  let addError = "";
  let daysDraft = "";
  let daysError = "";
  let rotating = false;
  let confirmingRenew = false;
  let rotated = "";

  function mark(id, on) {
    busy = { ...busy, [id]: on };
  }
  function forget(id) {
    mark(id, false);
    if (!rowError[id]) return;
    const next = { ...rowError };
    delete next[id];
    rowError = next;
  }
  function fail(id, err, again) {
    mark(id, false);
    rowError = { ...rowError, [id]: { cause: lan.cause(err), again } };
  }

  async function load() {
    loading = true;
    failed = null;
    const [devicesAnswer, friendsAnswer, probe] = await Promise.allSettled([
      lan.devices(),
      lan.friends(),
      lan.lanFeature(),
    ]);
    if (devicesAnswer.status === "rejected" || friendsAnswer.status === "rejected") {
      failed = (devicesAnswer.status === "rejected" ? devicesAnswer : friendsAnswer).reason;
      loading = false;
      return;
    }
    devices = devicesAnswer.value || [];
    friends = friendsAnswer.value || { shares_to: [], trusted_by: [] };
    feature = probe.status === "fulfilled" ? probe.value : null;
    now = Date.now();
    loading = false;
    if (isAdmin) await loadSettings();
  }

  // The administrator's two settings are their own card: a refusal there must
  // not blank the device and friend lists, which are everybody's.
  async function loadSettings() {
    settingsError = "";
    settingsAgain = null;
    try {
      settings = await lan.settings();
      daysDraft = String(settings?.credential_days ?? "");
    } catch (err) {
      settings = null;
      settingsError = lan.cause(err);
      settingsAgain = loadSettings;
    }
  }

  onMount(load);

  // -- devices -----------------------------------------------------------

  function startRename(device) {
    editing = device.id;
    draft = device.name;
    confirming = null;
    forget(device.id);
  }

  function cancelRename() {
    editing = null;
    draft = "";
  }

  // The rename paints in the frame the finger left and asks after; a refusal
  // puts the old name back WITH its cause, and « Réessayer » re-sends the same.
  async function applyName(device, name) {
    const before = devices;
    devices = lan.renameDeviceIn(devices, device.id, name);
    editing = null;
    mark(device.id, true);
    try {
      const saved = await lan.renameDevice(device.id, name);
      if (saved && saved.name) devices = lan.renameDeviceIn(devices, device.id, saved.name);
      forget(device.id);
    } catch (err) {
      devices = before;
      draft = name;
      fail(device.id, err, () => applyName(device, name));
    }
  }

  function saveName(device) {
    const name = draft.trim().slice(0, lan.NAME_MAX);
    if (!name) return;
    if (name === device.name) {
      cancelRename();
      return;
    }
    applyName(device, name);
  }

  async function revoke(device) {
    const before = devices;
    devices = lan.withoutDevice(devices, device.id);
    confirming = null;
    mark(device.id, true);
    try {
      await lan.revokeDevice(device.id);
      forget(device.id);
    } catch (err) {
      devices = before;
      fail(device.id, err, () => revoke(device));
    }
  }

  // -- friends -----------------------------------------------------------

  async function addFriend() {
    const name = newName.trim().slice(0, lan.NAME_MAX);
    if (!name || adding) return;
    adding = true;
    addError = "";
    try {
      const friend = await lan.addFriend(name);
      if (friend?.user_id) {
        friends = { ...friends, shares_to: [...(friends?.shares_to || []), friend] };
      }
      newName = "";
    } catch (err) {
      addError = lan.cause(err);
    }
    adding = false;
  }

  async function setAll(friend, on) {
    const before = friends;
    friends = lan.withScope(friends, friend.user_id, { all: on });
    picker = on ? null : picker;
    mark(friend.user_id, true);
    try {
      const saved = await lan.setFriendScope(friend.user_id, { all: on });
      if (saved?.user_id) {
        // The server's answer is the truth (it drops the playlists when
        // "tout partager" is on): take its scope, keep the list order.
        friends = lan.withScope(friends, friend.user_id, {
          all: !!saved.all,
          playlists: saved.playlists || [],
        });
      }
      forget(friend.user_id);
    } catch (err) {
      friends = before;
      fail(friend.user_id, err, () => setAll(friend, on));
    }
  }

  async function toggleShare(friend, playlistId) {
    const before = friends;
    const ids = lan.togglePlaylist(lan.sharedIds(friend), playlistId);
    const pairs = localPairs(ids);
    friends = lan.withScope(friends, friend.user_id, { playlists: pairs });
    mark(friend.user_id, true);
    try {
      const saved = await lan.setFriendScope(friend.user_id, { playlists: ids });
      if (saved?.user_id) {
        friends = lan.withScope(friends, friend.user_id, {
          all: !!saved.all,
          playlists: saved.playlists || [],
        });
      }
      forget(friend.user_id);
    } catch (err) {
      friends = before;
      fail(friend.user_id, err, () => toggleShare(friend, playlistId));
    }
  }

  async function removeFriend(friend) {
    const before = friends;
    friends = lan.withoutFriend(friends, friend.user_id);
    confirming = null;
    picker = null;
    mark(friend.user_id, true);
    try {
      await lan.removeFriend(friend.user_id);
      forget(friend.user_id);
    } catch (err) {
      friends = before;
      fail(friend.user_id, err, () => removeFriend(friend));
    }
  }

  function localPairs(ids) {
    const byId = new Map((choices || []).map((p) => [String(p.id), p.title]));
    return ids.map((id) => ({ id: String(id), title: byId.get(String(id)) ?? "" }));
  }

  async function openPicker(friend) {
    if (picker === friend.user_id) {
      picker = null;
      return;
    }
    picker = friend.user_id;
    choicesError = "";
    if (choices) return;
    try {
      // From the SPA's own cache: the library was read when the person opened
      // it, so a picker costs no request in the common case.
      choices = await lan.playlistChoices();
    } catch (err) {
      choices = null;
      choicesError = lan.cause(err);
    }
  }

  // -- administration ----------------------------------------------------

  async function toggleFeature() {
    if (!settings) return;
    const before = settings;
    settings = { ...settings, enabled: !settings.enabled };
    settingsError = "";
    try {
      settings = await lan.saveSettings({ enabled: before.enabled !== true });
      daysDraft = String(settings?.credential_days ?? daysDraft);
    } catch (err) {
      settings = before;
      settingsError = lan.cause(err);
      settingsAgain = toggleFeature;
    }
  }

  async function saveDays() {
    const value = lan.daysValue(daysDraft);
    if (value === null) {
      daysError = `Entre ${lan.CREDENTIAL_DAYS_MIN} et ${lan.CREDENTIAL_DAYS_MAX} jours.`;
      return;
    }
    daysError = "";
    if (!settings || value === settings.credential_days) return;
    const before = settings;
    settings = { ...settings, credential_days: value };
    try {
      settings = await lan.saveSettings({ credential_days: value });
      daysDraft = String(settings?.credential_days ?? value);
    } catch (err) {
      settings = before;
      daysDraft = String(before.credential_days ?? "");
      settingsError = lan.cause(err);
      settingsAgain = saveDays;
    }
  }

  // A new server key signs the device credentials from now on; the previous
  // one stays known 90 days, so the devices already enrolled keep working
  // until their next silent renewal.
  async function renewKey() {
    if (rotating) return;
    rotating = true;
    settingsError = "";
    settingsAgain = null;
    confirmingRenew = false;
    try {
      const answer = await lan.renewKey();
      rotated = answer?.kid ? `n° ${String(answer.kid).slice(0, 8)}` : "";
      feature = await lan.lanFeature({ refresh: true });
    } catch (err) {
      settingsError = lan.cause(err);
      settingsAgain = renewKey;
    }
    rotating = false;
  }

  $: shared = friends?.shares_to || [];
  $: trustedBy = friends?.trusted_by || [];
  $: disabled = feature ? feature.enabled === false : false;
</script>

{#if loading}
  <section class="card">
    <p class="muted">Chargement des appareils et des amis…</p>
  </section>
{:else if failed}
  <section class="card">
    <LoadFailed error={failed} notFound="Impossible de lire vos appareils et vos amis" on:retry={load} />
  </section>
{:else}
  {#if disabled}
    <section class="card note">
      <span class="note-ico"><Icon name="cloudOff" size={20} /></span>
      <div>
        <h2>Échange entre appareils désactivé</h2>
        <p class="muted sub">
          Ce serveur n'accepte plus l'échange direct entre appareils proches. Ce qui est déjà
          téléchargé reste lisible partout&nbsp;; sans serveur, un appareil ne renouvelle plus sa clé
          et cesse d'échanger à son expiration.
          {#if isAdmin}Réactivez-le dans « Serveur », en bas de cette page.{:else}Demandez à
            l'administrateur du serveur de le réactiver.{/if}
        </p>
      </div>
    </section>
  {/if}

  <section class="card">
    <div class="head">
      <h2>Mes appareils</h2>
      {#if devices.length}<span class="count muted">{devices.length}</span>{/if}
    </div>
    <p class="muted sub">
      Chaque appareil s'inscrit tout seul au premier contact avec le serveur, puis échange ses sons
      avec vos autres appareils sur le même réseau. Révoquer un appareil le coupe chez tous vos
      appareils&nbsp;: il devra se réinscrire.
    </p>

    {#if !devices.length}
      <p class="empty muted">
        Aucun appareil inscrit. Ouvrez Neezer sur votre ordinateur ou votre téléphone&nbsp;: il
        s'inscrira au premier contact avec le serveur.
      </p>
    {:else}
      <ul class="members">
        {#each devices as d (d.id)}
          <li class="member" class:gone={lan.deviceState(d) === "revoked"} class:pending={busy[d.id]}>
            <span class="glyph"><Icon name={lan.platformIcon(d.platform)} size={18} /></span>
            <div class="who">
              {#if editing === d.id}
                <form
                  class="rename"
                  on:submit|preventDefault={() => saveName(d)}
                >
                  <input
                    class="field rename-input"
                    bind:value={draft}
                    maxlength={lan.NAME_MAX}
                    aria-label="Nom de l'appareil"
                    autocomplete="off"
                    spellcheck="false"
                  />
                  <button class="btn btn-sm btn-primary" type="submit" disabled={!draft.trim()}>
                    Enregistrer
                  </button>
                  <button class="btn btn-sm btn-ghost" type="button" on:click={cancelRename}>
                    Annuler
                  </button>
                </form>
              {:else}
                <span class="name">
                  {d.name}
                  {#if lan.deviceState(d) === "current"}<span class="tag">cet appareil</span>{/if}
                  {#if lan.deviceState(d) === "revoked"}<span class="tag tag-off">révoqué</span>{/if}
                </span>
                <span class="meta muted">{lan.deviceDetail(d, now)}</span>
              {/if}
            </div>
            <div class="acts">
              {#if lan.deviceState(d) === "revoked"}
                <span class="muted small">Ne peut plus échanger</span>
              {:else if confirming === d.id}
                <button class="btn btn-sm btn-ghost danger" on:click={() => revoke(d)} disabled={busy[d.id]}>
                  Confirmer la révocation
                </button>
                <button class="btn btn-sm btn-ghost" on:click={() => (confirming = null)}>Annuler</button>
              {:else}
                <button
                  class="btn btn-sm btn-ghost"
                  on:click={() => startRename(d)}
                  disabled={busy[d.id]}
                  aria-label={"Renommer " + d.name}
                >
                  <Icon name="edit" size={15} /> Renommer
                </button>
                <button
                  class="btn btn-sm btn-ghost"
                  on:click={() => (confirming = d.id)}
                  disabled={busy[d.id]}
                  aria-label={"Révoquer " + d.name}
                >
                  <Icon name="trash" size={15} /> Révoquer
                </button>
              {/if}
            </div>
            {#if rowError[d.id]}
              <p class="row-error" role="alert">
                <Icon name="alert" size={14} />
                <span>{rowError[d.id].cause}</span>
                <button class="link" on:click={() => rowError[d.id].again()}>Réessayer</button>
              </p>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="card">
    <h2>Amis de confiance</h2>
    <p class="muted sub">
      Un ami de confiance peut prendre vos sons sur ses propres appareils, et seulement ce que vous
      lui ouvrez&nbsp;: tout ce que vous avez en local, ou les listes de lecture que vous cochez.
      Vous ne voyez que ses appareils à lui, jamais son compte.
    </p>

    <form class="add" on:submit|preventDefault={addFriend}>
      <input
        class="field add-input"
        bind:value={newName}
        maxlength={lan.NAME_MAX}
        placeholder="Nom du compte"
        aria-label="Nom du compte à ajouter"
        autocomplete="off"
        spellcheck="false"
      />
      <button class="btn btn-primary" type="submit" disabled={!newName.trim() || adding}>
        <Icon name="personAdd" size={16} />
        {adding ? "Ajout…" : "Ajouter"}
      </button>
    </form>
    {#if addError}
      <p class="row-error" role="alert">
        <Icon name="alert" size={14} /><span>{addError}</span>
      </p>
    {/if}

    {#if !shared.length}
      <p class="empty muted">
        Aucun ami de confiance pour l'instant. Ajoutez le compte d'une personne du serveur pour
        l'autoriser à prendre vos sons sur ses appareils.
      </p>
    {:else}
      <ul class="members">
        {#each shared as f (f.user_id)}
          <li class="member friend" class:pending={busy[f.user_id]}>
            <span class="glyph"><Icon name="user" size={18} /></span>
            <div class="who">
              <span class="name">{f.name}</span>
              <span class="meta muted">{lan.sharedSummary(f)}</span>
            </div>
            <div class="acts">
              <button
                class="toggle"
                role="switch"
                aria-checked={!!f.all}
                on:click={() => setAll(f, !f.all)}
                disabled={busy[f.user_id]}
              >
                <span class="tg-title">Tout partager</span>
                <span class="sw" class:on={!!f.all}><span class="knob"></span></span>
              </button>
              {#if !f.all}
                <button
                  class="btn btn-sm btn-ghost"
                  on:click={() => openPicker(f)}
                  aria-expanded={picker === f.user_id}
                >
                  <Icon name="bookmark" size={15} />
                  {(f.playlists || []).length ? "Listes partagées" : "Choisir des listes"}
                </button>
              {/if}
              {#if confirming === f.user_id}
                <button class="btn btn-sm btn-ghost danger" on:click={() => removeFriend(f)} disabled={busy[f.user_id]}>
                  Confirmer
                </button>
                <button class="btn btn-sm btn-ghost" on:click={() => (confirming = null)}>Annuler</button>
              {:else}
                <button
                  class="btn btn-sm btn-ghost"
                  on:click={() => (confirming = f.user_id)}
                  disabled={busy[f.user_id]}
                  aria-label={"Retirer " + f.name}
                >
                  <Icon name="trash" size={15} /> Retirer
                </button>
              {/if}
            </div>

            {#if picker === f.user_id && !f.all}
              <div class="picker" transition:collapse>
                {#if choicesError}
                  <p class="row-error" role="alert">
                    <Icon name="alert" size={14} /><span>Listes indisponibles : {choicesError}</span>
                    <button class="link" on:click={() => ((choices = null), openPicker(f))}>Réessayer</button>
                  </p>
                {:else if !choices}
                  <p class="muted small">Lecture de vos listes…</p>
                {:else if !choices.length}
                  <p class="muted small">Vous n'avez aucune liste de lecture à partager.</p>
                {:else}
                  <p class="picker-title label-mono">Ce qu'il peut prendre</p>
                  <ul class="picker-list">
                    {#each choices as p (p.id)}
                      <li>
                        <label class="pick">
                          <input
                            type="checkbox"
                            checked={lan.sharedIds(f).includes(String(p.id))}
                            on:change={() => toggleShare(f, p.id)}
                            disabled={busy[f.user_id]}
                          />
                          <span class="pick-title">{p.title}</span>
                        </label>
                      </li>
                    {/each}
                  </ul>
                {/if}
              </div>
            {/if}

            {#if rowError[f.user_id]}
              <p class="row-error" role="alert">
                <Icon name="alert" size={14} /><span>{rowError[f.user_id].cause}</span>
                <button class="link" on:click={() => rowError[f.user_id].again()}>Réessayer</button>
              </p>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}

    {#if trustedBy.length}
      <div class="trusted">
        <p class="label-mono">Qui partage avec vous</p>
        <ul class="trusted-list">
          {#each trustedBy as t (t.user_id)}
            <li>
              <Icon name="share" size={15} />
              <span><strong>{t.name}</strong> vous ouvre ses sons</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </section>

  {#if isAdmin}
    <section class="card">
      <h2>Serveur</h2>
      <p class="muted sub">
        Ces deux réglages valent pour tout le serveur&nbsp;: ils décident si les appareils peuvent
        s'échanger des sons, et combien de temps la clé qu'un appareil présente à ses voisins reste
        valable.
      </p>

      {#if settingsError}
        <p class="row-error" role="alert">
          <Icon name="alert" size={14} /><span>{settingsError}</span>
          {#if settingsAgain}<button class="link" on:click={settingsAgain}>Réessayer</button>{/if}
        </p>
      {/if}

      {#if !settings}
        {#if !settingsError}
          <p class="muted">Lecture des réglages du serveur…</p>
        {/if}
      {:else}
        <button class="toggle" role="switch" aria-checked={!!settings.enabled} on:click={toggleFeature}>
          <span class="tg-txt">
            <span class="tg-title">Échanger entre les appareils de ce serveur</span>
            <span class="tg-hint muted">
              Coupé, plus aucun appareil ne s'inscrit&nbsp;: ceux qui le sont déjà cessent d'échanger
              à l'expiration de leur clé. Rien de ce qui est téléchargé ne disparaît.
            </span>
          </span>
          <span class="sw" class:on={!!settings.enabled}><span class="knob"></span></span>
        </button>

        <div class="days">
          <label for="lan-days">Durée de validité d'une clé d'appareil</label>
          <span class="days-row">
            <input
              id="lan-days"
              class="field days-input"
              type="number"
              min={lan.CREDENTIAL_DAYS_MIN}
              max={lan.CREDENTIAL_DAYS_MAX}
              step="1"
              bind:value={daysDraft}
              on:change={saveDays}
              aria-invalid={daysError ? "true" : undefined}
            />
            <span class="muted">jours</span>
            <span class="muted small">({lan.CREDENTIAL_DAYS_MIN} à {lan.CREDENTIAL_DAYS_MAX})</span>
          </span>
          {#if daysError}
            <p class="row-error" role="alert"><Icon name="alert" size={14} /><span>{daysError}</span></p>
          {:else}
            <p class="muted small">
              Un appareil renouvelle sa clé tout seul chaque jour où il joint le serveur. Plus la
              durée est courte, plus un appareil perdu cesse vite d'échanger&nbsp;; {lan.daysLabel(
                settings.credential_days,
              )} par défaut.
            </p>
          {/if}
        </div>

        <p class="key-line muted small">
          <Icon name="shield" size={14} />
          Clé du serveur {lan.keyLabel(feature)} · elle signe les clés de vos appareils.
        </p>

        {#if rotated}
          <p class="muted small" role="status">Nouvelle clé du serveur {rotated}, déjà en service.</p>
        {/if}
        {#if confirmingRenew}
          <div class="renew">
            <p class="muted small">
              Une nouvelle clé signe les appareils à partir de maintenant. Ceux qui sont déjà inscrits
              gardent la leur jusqu'à leur prochain renouvellement, et l'ancienne clé reste reconnue
              90 jours&nbsp;: rien ne se coupe.
            </p>
            <div class="acts renew-acts">
              <button class="btn btn-sm btn-ghost danger" on:click={renewKey} disabled={rotating}>
                {rotating ? "Renouvellement…" : "Confirmer le renouvellement"}
              </button>
              <button class="btn btn-sm btn-ghost" on:click={() => (confirmingRenew = false)}>Annuler</button>
            </div>
          </div>
        {:else}
          <button class="btn btn-sm btn-ghost" on:click={() => (confirmingRenew = true)}>
            <Icon name="refresh" size={15} /> Renouveler la clé
          </button>
        {/if}
      {/if}
    </section>
  {/if}
{/if}

<style>
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  .count {
    font-size: 0.9rem;
  }
  .note {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .note-ico {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    flex: none;
    border-radius: 50%;
    background: var(--s2);
    color: var(--ac-text);
  }
  .empty {
    margin: 10px 0 0;
    font-size: 0.92rem;
    line-height: 1.5;
  }

  /* One member row: glyph, name and meta, then its actions. The actions take
     their own line under 760 px so a 340 px column (the player's side panel)
     never squeezes a name to two letters. */
  .members {
    list-style: none;
    margin: 10px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .member {
    display: grid;
    grid-template-columns: 34px minmax(0, 1fr);
    align-items: center;
    gap: 4px 12px;
    padding: 12px 4px;
  }
  .member + .member {
    border-top: 1px solid var(--ln);
  }
  .member.pending {
    opacity: 0.72;
  }
  .member.gone .name {
    color: var(--dim);
  }
  .glyph {
    display: grid;
    place-items: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    background: var(--s2);
    color: var(--tx);
  }
  .who {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .meta {
    font-size: 0.82rem;
    overflow-wrap: anywhere;
  }
  .tag {
    font-size: 0.68rem;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 7px;
    border-radius: 999px;
    background: var(--ac);
    color: var(--onac);
  }
  .tag-off {
    background: var(--s2);
    color: var(--dim);
  }
  .acts {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .acts :global(.btn) {
    flex: none;
  }
  .danger {
    color: var(--ac-text);
    border-color: color-mix(in srgb, var(--ac-text) 50%, var(--ln));
  }
  .danger:hover:not(:disabled) {
    background: var(--s2);
  }
  .rename {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .rename-input {
    flex: 1 1 140px;
    min-width: 0;
    height: 36px;
  }
  .row-error {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    margin: 2px 0 0;
    font-size: 0.85rem;
    color: var(--tx);
  }
  .row-error :global(.icon) {
    color: var(--ac-text);
    flex: none;
  }
  .link {
    padding: 0;
    font-size: 0.85rem;
    font-weight: 600;
    color: var(--ac-text);
    text-decoration: underline;
    background: none;
    border: 0;
  }
  .small {
    font-size: 0.8rem;
  }

  /* The add form is a field and its button, on one line while there is room. */
  .add {
    display: flex;
    gap: 8px;
    margin-top: 12px;
    flex-wrap: wrap;
  }
  .add-input {
    flex: 1 1 180px;
    min-width: 0;
  }

  /* A switch, drawn the one way the design system draws one: a --btn track
     with an --onbtn knob, an --s2 track ringed by --dim when off. The label is
     a column (title over its explanation) and the row is a GRID, not
     `justify-content: space-between`: the text column must be told it may
     shrink (`minmax(0, 1fr)`), or the explanation runs under the knob — a
     `flex` button kept both children on one line at 1280 px (measured: the
     title and its hint shared a line, and the hint's first line ended outside
     the card's content box). */
  .toggle {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 16px;
    width: 100%;
    min-height: 44px;
    padding: 6px 0;
    text-align: left;
    background: none;
    border: 0;
    color: var(--tx);
  }
  .tg-txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .tg-title {
    font-weight: 600;
  }
  .tg-hint {
    font-size: 0.78rem;
    line-height: 1.45;
  }
  /* In a member row the switch is one of the row's actions, so it keeps its
     own width and its short label. */
  .member .toggle {
    display: inline-flex;
    align-items: center;
    width: auto;
    min-height: 34px;
    padding: 0 4px;
    gap: 10px;
  }
  .member .tg-title {
    font-size: 0.86rem;
    white-space: nowrap;
  }
  .sw {
    flex: none;
    width: 44px;
    height: 26px;
    border-radius: 999px;
    background: var(--s2);
    border: 1px solid var(--dim);
    position: relative;
    transition:
      background var(--dur-instant) var(--ease-standard),
      border-color var(--dur-instant) var(--ease-standard);
  }
  .sw.on {
    background: var(--btn);
    border-color: var(--btn);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--tx);
    transition:
      transform var(--dur-instant) var(--ease-standard),
      background var(--dur-instant) var(--ease-standard);
  }
  .sw.on .knob {
    background: var(--onbtn);
    transform: translateX(16px);
  }
  @media (prefers-reduced-motion: reduce) {
    .knob {
      transition: none;
    }
  }

  .picker {
    grid-column: 1 / -1;
    margin: 6px 0 0;
    padding: 10px 12px;
    border: 1px solid var(--ln);
    border-radius: 12px;
    background: var(--s1);
  }
  .picker-title {
    margin: 0 0 6px;
    color: var(--dim);
  }
  .picker-list {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 36px;
    font-size: 0.9rem;
  }
  .pick input {
    width: 18px;
    height: 18px;
    flex: none;
    accent-color: var(--btn);
  }
  .pick-title {
    overflow-wrap: anywhere;
  }

  .trusted {
    margin-top: 16px;
    padding-top: 12px;
    border-top: 1px solid var(--ln);
  }
  .trusted-list {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 0.88rem;
  }
  .trusted-list li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .trusted-list :global(.icon) {
    color: var(--dim);
    flex: none;
  }

  .days {
    margin-top: 6px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .days > label {
    font-weight: 600;
    font-size: 0.9rem;
  }
  .days-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .days-input {
    width: 92px;
    height: 36px;
    text-align: right;
  }
  .days-input[aria-invalid="true"] {
    border-color: var(--ac-text);
  }
  .key-line {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 14px 0 0;
  }

  /* The key's renewal: a sentence and its two buttons, never a bare destructive
     icon (this changes what every device signs with). */
  .renew {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 8px;
  }
  .renew p {
    margin: 0;
    max-width: 62ch;
    line-height: 1.5;
  }
  .renew-acts {
    grid-column: auto;
    justify-content: flex-start;
  }

  /* Above 760 px a member's actions sit on the name's own line. */
  @media (min-width: 761px) {
    .member {
      grid-template-columns: 34px minmax(0, 1fr) auto;
    }
    .acts {
      grid-column: auto;
      justify-content: flex-end;
    }
    .row-error {
      grid-column: 2 / -1;
    }
    .picker {
      grid-column: 2 / -1;
    }
  }
</style>
