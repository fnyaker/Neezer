<script>
  import { onMount, tick } from "svelte";
  import { replace } from "svelte-spa-router";
  import { user } from "../lib/stores.js";
  import { api } from "../lib/api.js";
  import { online } from "../lib/net.js";
  import Icon from "../components/Icon.svelte";
  import LoadFailed from "../components/LoadFailed.svelte";
  import {
    MAX_PASSWORD,
    MAX_USERNAME,
    MIN_PASSWORD,
    MIN_USERNAME,
    clearDraft,
    expiryLabel,
    foldName,
    foldUsername,
    landingFor,
    newRequestId,
    passwordLabel,
    passwordMessage,
    passwordScore,
    readDraft,
    rememberDraft,
    tokenShapeOk,
    usernameMessage,
  } from "../lib/invite.js";

  export let token = "";
  export let savedAccount = null;

  let phase = "checking";
  let label = "";
  let expires = "";
  let errorTitle = "";
  let errorText = "";
  let account = $user || (savedAccount && savedAccount.name ? savedAccount : null);
  let leaving = false;
  let busy = false;
  let username = "";
  let password = "";
  let confirm = "";
  let showPassword = false;
  let submitted = false;
  let touched = { username: false, password: false, confirm: false };
  let fieldErrors = { username: "", password: "", confirm: "" };
  let formError = "";
  let pendingRetry = false;
  let requestId = "";
  let nameEl;
  let passEl;
  let confirmEl;
  let alertEl;

  $: if ($user) account = $user;
  $: folded = foldUsername(username);
  $: nameMessage = usernameMessage(username);
  $: nameError = fieldErrors.username || (submitted || touched.username ? nameMessage : "");
  $: passMessage = passwordMessage(password, folded);
  $: passError = fieldErrors.password || (submitted || touched.password ? passMessage : "");
  $: confirmMessage =
    password && confirm && password !== confirm
      ? "Les deux mots de passe ne correspondent pas."
      : "";
  $: confirmError = fieldErrors.confirm || (submitted || touched.confirm ? confirmMessage : "");
  $: score = passwordScore(password, folded);
  $: strength = passwordLabel(password, folded);
  $: missing = Math.max(0, MIN_PASSWORD - [...password].length);

  onMount(() => {
    const back = () => {
      if (phase === "error") inspect();
    };
    window.addEventListener("online", back);
    inspect();
    return () => window.removeEventListener("online", back);
  });

  async function inspect() {
    if (!tokenShapeOk(token)) {
      phase = "invalid";
      return;
    }
    phase = "checking";
    errorTitle = "";
    errorText = "";
    try {
      const r = await api.inviteInspect(token);
      if (!r || r.valid !== true) {
        phase = "invalid";
        return;
      }
      label = typeof r.label === "string" ? r.label : "";
      expires = expiryLabel(r.expires);
      const draft = readDraft(token);
      if (draft) {
        requestId = draft.request_id;
        if (draft.name && !username) username = draft.name;
      }
      phase = "valid";
      await tick();
      if (!account) focusField("username");
    } catch (e) {
      const status = e && e.status;
      if (status === 404) {
        phase = "invalid";
        return;
      }
      errorTitle = status === 429 ? "Trop de tentatives" : "Impossible de vérifier l'invitation";
      if (status === 429) {
        errorText =
          (e.body && e.body.error) || "Trop de tentatives. Réessaie dans quelques minutes.";
      } else if (!status) {
        errorText = $online
          ? "Le serveur ne répond pas. Vérifie ta connexion, puis réessaie."
          : "Tu es hors ligne. La vérification reprendra au retour du réseau.";
      } else {
        errorText =
          "Le serveur ne répond pas. Ton lien n'est pas en cause. Réessaie dans un instant.";
      }
      phase = "error";
    }
  }

  function focusField(field) {
    const el = field === "username" ? nameEl : field === "password" ? passEl : confirmEl;
    el?.focus();
  }

  function clearField(field) {
    fieldErrors = { ...fieldErrors, [field]: "" };
    formError = "";
    pendingRetry = false;
  }

  function onName(event) {
    const el = event.currentTarget;
    const position = el.selectionStart ?? el.value.length;
    const before = foldName(el.value.slice(0, position));
    const next = foldName(el.value);
    username = next;
    clearField("username");
    if (next !== el.value) {
      el.value = next;
      const at = Math.min(next.length, before.length);
      try {
        el.setSelectionRange(at, at);
      } catch {
      }
    }
  }

  function onPassword(event) {
    password = event.currentTarget.value;
    clearField("password");
    clearField("confirm");
  }

  function onConfirm(event) {
    confirm = event.currentTarget.value;
    clearField("confirm");
  }

  function onBlur(field) {
    if (field === "username") username = foldUsername(username);
    touched = { ...touched, [field]: true };
  }

  async function submit(event) {
    event?.preventDefault();
    if (busy || phase !== "valid" || account) return;
    submitted = true;
    formError = "";
    pendingRetry = false;
    const problems = {
      username: usernameMessage(username),
      password: passwordMessage(password, folded),
      confirm: password !== confirm ? "Les deux mots de passe ne correspondent pas." : "",
    };
    const first = ["username", "password", "confirm"].find((key) => problems[key]);
    fieldErrors = { username: "", password: "", confirm: "" };
    if (first) {
      fieldErrors = { ...fieldErrors, [first]: problems[first] };
      await tick();
      focusField(first);
      return;
    }
    const draft = readDraft(token);
    requestId = (draft && draft.request_id) || requestId || newRequestId();
    rememberDraft({ token, name: username, request_id: requestId });
    busy = true;
    try {
      const r = await api.inviteRedeem({
        token,
        username: folded,
        password,
        password_confirm: confirm,
        request_id: requestId,
      });
      clearDraft();
      phase = "done";
      user.set(r.user);
      replace(landingFor(r.user));
    } catch (e) {
      await handleFailure(e);
    } finally {
      busy = false;
    }
  }

  async function handleFailure(e) {
    const status = e && e.status;
    const body = (e && e.body) || {};
    const message = body.error || "";
    if (status === 400 && body.field) {
      const map = {
        username: "username",
        password: "password",
        password_confirm: "confirm",
      };
      const field = map[body.field];
      if (field) {
        fieldErrors = { ...fieldErrors, [field]: message || "Cette valeur n'est pas acceptée." };
        await tick();
        focusField(field);
      } else {
        formError = message || "Le formulaire n'a pas été accepté.";
        await tick();
        alertEl?.focus();
      }
      return;
    }
    if (status === 404) {
      clearDraft();
      phase = "invalid";
      return;
    }
    pendingRetry = false;
    if (status === 429) {
      formError = message || "Trop de tentatives. Réessaie dans quelques minutes.";
    } else if (status === 413) {
      formError = message || "Requête trop volumineuse.";
    } else if (!status || (e && e.offline)) {
      pendingRetry = true;
      formError =
        "La connexion s'est interrompue. Réessaie : " +
        "ton lien reste valable tant que le compte n'est pas créé.";
    } else {
      pendingRetry = true;
      formError = "Le serveur n'a pas pu créer ton compte. Réessaie dans un instant.";
    }
    await tick();
    alertEl?.focus();
  }

  async function useInvitation() {
    if (leaving) return;
    leaving = true;
    try {
      await api.logout();
    } catch {
    }
    user.set(null);
    account = null;
    leaving = false;
    await tick();
    focusField("username");
  }

  function openApp() {
    try {
      history.replaceState(null, "", location.pathname + location.search + "#/");
    } catch {
      location.hash = "#/";
    }
    location.reload();
  }
</script>

<svelte:head>
  <title>Rejoindre Neezer</title>
  <meta name="robots" content="noindex" />
</svelte:head>

<div class="page">
  <div class="halo" aria-hidden="true"></div>
  <main class="card" aria-labelledby="invite-title">
    <div class="brand">
      <span class="mark"><Icon name="disc" size={17} /></span>
      <span>Neezer</span>
    </div>

    {#if phase === "checking"}
      <div class="wait" aria-busy="true">
        <h1 id="invite-title" class="sr">Rejoindre Neezer</h1>
        <span class="sr">Vérification de l'invitation…</span>
        <span class="sk sk-title"></span>
        <span class="sk sk-line"></span>
        <span class="sk sk-line short"></span>
        <span class="sk sk-control"></span>
        <span class="sk sk-control"></span>
        <span class="sk sk-control"></span>
        <span class="sk sk-button"></span>
      </div>
    {:else if phase === "invalid"}
      <div class="state">
        <span class="state-icon"><Icon name="alert" size={24} /></span>
        <h1 id="invite-title">Cette invitation n'est plus valide.</h1>
        <p class="sub">Demande un nouveau lien à la personne qui t'a invité.</p>
      </div>
      <button class="primary wide" type="button" on:click={openApp}>
        {account ? "Retour à l'application" : "Se connecter"}
      </button>
    {:else if phase === "error"}
      <h1 id="invite-title">{errorTitle}</h1>
      <div class="failedslot">
        <LoadFailed notFound={errorText} on:retry={inspect} />
      </div>
      <button class="primary wide" type="button" on:click={inspect}>
        <Icon name="refresh" size={17} /> Réessayer
      </button>
    {:else if phase === "done"}
      <h1 id="invite-title">Ton compte est créé.</h1>
      <p class="sub">Ouverture de Neezer…</p>
    {:else if account}
      <h1 id="invite-title">Tu es déjà connecté</h1>
      <p class="sub">
        Tu es connecté en tant que <strong>{account.name}</strong>. Créer un compte ici te
        déconnectera de celui-ci.
      </p>
      <div class="actions">
        <button class="primary wide" type="button" on:click={openApp}>
          Retour à l'application
        </button>
        <button class="ghost wide" type="button" on:click={useInvitation} disabled={leaving}>
          {leaving ? "Déconnexion…" : "Utiliser cette invitation"}
        </button>
      </div>
    {:else}
      <div class="intro">
        {#if label}<span class="label-mono">{label}</span>{/if}
        <h1 id="invite-title">Rejoindre Neezer</h1>
        <p class="sub">Crée ton compte. Ce lien ne fonctionne qu'une fois.</p>
        {#if expires}<p class="expiry">{expires}</p>{/if}
      </div>

      <form on:submit={submit} novalidate>
        <div class="control">
          <label for="invite-name">Nom d'utilisateur</label>
          <input
            id="invite-name"
            bind:this={nameEl}
            value={username}
            on:input={onName}
            on:blur={() => onBlur("username")}
            autocomplete="username"
            autocapitalize="none"
            autocorrect="off"
            spellcheck="false"
            maxlength={MAX_USERNAME}
            aria-describedby={nameError ? "invite-name-help invite-name-err" : "invite-name-help"}
            aria-invalid={nameError ? "true" : "false"}
          />
          <p class="hint" id="invite-name-help">
            {MIN_USERNAME} à {MAX_USERNAME} caractères : a–z, 0–9, point, tiret, tiret bas.
          </p>
          {#if nameError}
            <p class="err" id="invite-name-err" role="alert">
              <Icon name="alert" size={14} /><span>{nameError}</span>
            </p>
          {/if}
        </div>

        <div class="control">
          <label for="invite-password">Mot de passe</label>
          <div class="passwrap">
            <input
              id="invite-password"
              bind:this={passEl}
              value={password}
              on:input={onPassword}
              on:blur={() => onBlur("password")}
              type={showPassword ? "text" : "password"}
              autocomplete="new-password"
              autocapitalize="none"
              autocorrect="off"
              spellcheck="false"
              aria-describedby={passError ? "invite-pass-help invite-pass-err" : "invite-pass-help"}
              aria-invalid={passError ? "true" : "false"}
            />
            <button
              class="reveal"
              type="button"
              aria-pressed={showPassword}
              aria-label={showPassword ? "Masquer le mot de passe" : "Afficher le mot de passe"}
              on:click={() => (showPassword = !showPassword)}
            >
              {showPassword ? "Masquer" : "Afficher"}
            </button>
          </div>
          {#if password}
            <div class="meterrow">
              <div
                class="meter"
                role="progressbar"
                aria-label="Force du mot de passe"
                aria-valuemin="0"
                aria-valuemax="4"
                aria-valuenow={score}
                aria-valuetext={strength}
              >
                {#each [1, 2, 3, 4] as level}
                  <span class="seg" class:on={level <= score}></span>
                {/each}
              </div>
              <span class="strength">{strength}</span>
              {#if missing > 0}
                <span class="count">{[...password].length} / {MIN_PASSWORD}</span>
              {/if}
            </div>
          {/if}
          <p class="hint" id="invite-pass-help">
            {MIN_PASSWORD} caractères minimum, {MAX_PASSWORD} au plus. Une phrase de passe fait très
            bien l'affaire.
          </p>
          {#if passError}
            <p class="err" id="invite-pass-err" role="alert">
              <Icon name="alert" size={14} /><span>{passError}</span>
            </p>
          {/if}
        </div>

        <div class="control">
          <label for="invite-confirm">Confirmer le mot de passe</label>
          <input
            id="invite-confirm"
            bind:this={confirmEl}
            value={confirm}
            on:input={onConfirm}
            on:blur={() => onBlur("confirm")}
            type={showPassword ? "text" : "password"}
            autocomplete="new-password"
            autocapitalize="none"
            autocorrect="off"
            spellcheck="false"
            aria-describedby={confirmError ? "invite-confirm-err" : undefined}
            aria-invalid={confirmError ? "true" : "false"}
          />
          {#if confirmError}
            <p class="err" id="invite-confirm-err" role="alert">
              <Icon name="alert" size={14} /><span>{confirmError}</span>
            </p>
          {/if}
        </div>

        {#if formError}
          <div class="alert" role="alert" tabindex="-1" bind:this={alertEl}>
            <Icon name="alert" size={16} />
            <span>{formError}</span>
            {#if pendingRetry}
              <button class="retry" type="button" on:click={submit}>Réessayer</button>
            {/if}
          </div>
        {/if}

        <button class="primary wide submit" type="submit" disabled={busy}>
          {busy ? "Création…" : "Créer mon compte"}
        </button>
      </form>

      <p class="foot">
        Déjà un compte ? <button class="link" type="button" on:click={openApp}>Se connecter</button>
      </p>
    {/if}
  </main>
</div>

<style>
  .page {
    min-height: 100dvh;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: max(24px, var(--safe-top, 0px)) 20px max(24px, var(--safe-bottom, 0px));
    padding-top: clamp(24px, 10vh, 96px);
    background: var(--bg);
  }
  .halo {
    display: none;
  }
  .card {
    position: relative;
    z-index: 1;
    width: min(440px, 100%);
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding: 24px;
    border-radius: var(--r-card);
    background: var(--s1);
    border: 1px solid var(--ln);
    overflow: visible;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-family: var(--font-display);
    font-size: 17px;
    font-weight: 700;
    letter-spacing: -0.02em;
  }
  .mark {
    width: 30px;
    height: 30px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--ac);
    color: var(--onac);
  }
  .intro,
  .state {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    line-height: 1.1;
    letter-spacing: -0.03em;
  }
  .state {
    gap: 10px;
  }
  .state h1 {
    font-size: 26px;
  }
  .state-icon {
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--s2);
    color: var(--dim);
  }
  .label-mono {
    color: var(--dim);
  }
  .sub {
    margin: 0;
    color: var(--dim);
    font-size: 14.5px;
    line-height: 1.5;
  }
  .sub strong {
    color: var(--tx);
    font-weight: 600;
  }
  .expiry {
    margin: 0;
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11.5px;
    line-height: 1.5;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .control {
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  label {
    font-size: 13px;
    font-weight: 600;
  }
  input {
    width: 100%;
    height: 48px;
    padding: 0 14px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--bg);
    color: var(--tx);
    font-size: 15px;
  }
  input:focus {
    border-color: var(--ac-text);
  }
  input[aria-invalid="true"] {
    border-color: var(--ac-text);
  }
  .hint {
    margin: 0;
    color: var(--dim);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .err {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: var(--ac-text);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .err :global(svg) {
    margin-top: 2px;
    flex: none;
  }
  .reveal {
    position: absolute;
    top: 2px;
    right: 2px;
    min-width: 64px;
    height: 44px;
    padding: 0 10px;
    border-radius: 999px;
    color: var(--dim);
    font-size: 12.5px;
    font-weight: 600;
  }
  .reveal:hover {
    color: var(--tx);
    background: var(--s2);
  }
  .passwrap {
    position: relative;
  }
  .passwrap input {
    padding-right: 76px;
  }
  .meterrow {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .meter {
    flex: 1;
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
  }
  .seg {
    height: 4px;
    border-radius: 2px;
    background: var(--ln);
  }
  .seg.on {
    background: var(--ac-text);
  }
  .strength {
    font-size: 12px;
    font-weight: 600;
  }
  .count {
    color: var(--dim);
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .alert {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 12px;
    border-radius: 12px;
    border: 1px solid var(--ln);
    background: var(--s2);
    color: var(--tx);
    font-size: 13px;
    line-height: 1.45;
  }
  .alert :global(svg) {
    flex: none;
    margin-top: 1px;
    color: var(--ac-text);
  }
  .retry {
    margin-left: auto;
    min-height: 44px;
    padding: 0 10px;
    color: var(--ac-text);
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .primary,
  .ghost {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 48px;
    min-height: 44px;
    padding: 0 20px;
    border-radius: 999px;
    font-size: 15px;
    font-weight: 700;
  }
  .primary {
    background: var(--btn);
    color: var(--onbtn);
  }
  .ghost {
    border: 1px solid var(--ln);
    background: transparent;
    color: var(--tx);
  }
  .ghost:hover {
    background: var(--s2);
  }
  .primary:disabled,
  .ghost:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .wide {
    width: 100%;
  }
  .primary:not(:disabled):hover {
    filter: brightness(1.06);
  }
  .submit {
    margin-top: 2px;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .foot {
    margin: 0;
    color: var(--dim);
    font-size: 13.5px;
    text-align: center;
  }
  .link {
    display: inline-flex;
    align-items: center;
    min-height: 44px;
    padding: 0 6px;
    color: var(--tx);
    font-weight: 600;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .link:hover {
    color: var(--ac-text);
  }
  .wait {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .sk {
    display: block;
    border-radius: 8px;
    background: var(--s2);
  }
  .sk-title {
    width: 68%;
    height: 30px;
    margin: 4px 0 2px;
  }
  .sk-line {
    height: 12px;
  }
  .sk-line.short {
    width: 72%;
  }
  .sk-control {
    height: 48px;
    margin-top: 6px;
    border-radius: 12px;
  }
  .sk-button {
    height: 48px;
    margin-top: 8px;
    border-radius: 999px;
  }
  .failedslot :global(.failed) {
    margin: 0;
    padding: 0;
    align-items: flex-start;
    text-align: left;
    gap: 8px;
  }
  .failedslot :global(.failed p) {
    margin: 0;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
  @media (min-width: 641px) {
    .card {
      padding: 32px;
      gap: 22px;
    }
    h1 {
      font-size: 34px;
    }
  }
  @media (min-width: 900px) {
    .page::before {
      content: "";
      position: fixed;
      left: 50%;
      top: 50%;
      width: min(720px, 78vw);
      aspect-ratio: 1;
      transform: translate(-50%, -50%);
      border-radius: 50%;
      border: 1px solid var(--s2);
      pointer-events: none;
      z-index: 0;
      opacity: 0.8;
    }
    .page::after {
      content: "";
      position: fixed;
      left: 50%;
      top: 50%;
      width: min(500px, 58vw);
      aspect-ratio: 1;
      transform: translate(-50%, -50%);
      border-radius: 50%;
      border: 1px solid var(--ln);
      pointer-events: none;
      z-index: 0;
      opacity: 0.55;
    }
    .halo {
      display: block;
      position: fixed;
      left: 50%;
      top: 50%;
      width: min(300px, 40vw);
      aspect-ratio: 1;
      transform: translate(-50%, -50%);
      border-radius: 50%;
      border: 1px solid var(--ln);
      pointer-events: none;
      z-index: 0;
      opacity: 0.4;
    }
  }
  @media (max-width: 380px) {
    h1 {
      font-size: 27px;
    }
    .state h1 {
      font-size: 23px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .primary,
    .ghost {
      transition: none;
    }
  }
</style>
