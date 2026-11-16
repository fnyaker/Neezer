<script>
  import { tick } from "svelte";
  import { user } from "../lib/stores.js";
  import { api } from "../lib/api.js";
  import Icon from "../components/Icon.svelte";
  // The login renders outside the router, so it has no page enter of its own:
  // its root fades once the shell resolves "nobody is signed in" (I15), and the
  // error line arrives with a fade instead of appearing (I7).
  import { fadeIn } from "../lib/motion.js";

  let username = "";
  let password = "";
  let error = "";
  let alertEl;
  let busy = false;
  let passwordEl;

  function failureMessage(err) {
    if (!err) return "La connexion a échoué. Réessaie.";
    if (err.status === 401) return "Nom d'utilisateur ou mot de passe incorrect.";
    if (!err.status || err.offline)
      return "Le serveur ne répond pas. Vérifie ta connexion, puis réessaie.";
    if (err.status === 429) return "Trop de tentatives. Réessaie dans quelques minutes.";
    return "Le serveur n'a pas répondu. Réessaie dans un instant.";
  }

  async function submit(event) {
    event.preventDefault();
    if (busy) return;
    error = "";
    busy = true;
    try {
      const r = await api.login(username, password);
      user.set(r.user);
    } catch (err) {
      error = failureMessage(err);
      if (err && err.status === 401) {
        password = "";
        passwordEl?.focus();
      } else {
        tick().then(() => alertEl?.focus());
      }
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>Connexion · Neezer</title>
</svelte:head>

<div class="page" in:fadeIn>
  <div class="halo" aria-hidden="true"></div>
  <main class="card" aria-labelledby="login-title">
    <div class="brand">
      <span class="mark"><Icon name="disc" size={17} /></span>
      <span>Neezer</span>
    </div>

    <div class="intro">
      <h1 id="login-title">Connexion</h1>
      <p class="sub">Entre pour retrouver ta musique.</p>
    </div>

    <form on:submit={submit} novalidate>
      <div class="control">
        <label for="login-name">Nom d'utilisateur</label>
        <input
          id="login-name"
          bind:value={username}
          autocomplete="username"
          autocapitalize="none"
          autocorrect="off"
          spellcheck="false"
        />
      </div>

      <div class="control">
        <label for="login-password">Mot de passe</label>
        <input
          id="login-password"
          bind:this={passwordEl}
          bind:value={password}
          type="password"
          autocomplete="current-password"
        />
      </div>

      {#if error}
        <p class="err" role="alert" tabindex="-1" bind:this={alertEl} in:fadeIn><Icon name="alert" size={14} /><span>{error}</span></p>
      {/if}

      <button class="primary wide" type="submit" disabled={busy}>
        {busy ? "Connexion…" : "Se connecter"}
      </button>
    </form>
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
  .card {
    position: relative;
    z-index: 1;
    width: min(400px, 100%);
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
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 32px;
    font-weight: 700;
    line-height: 1.1;
    letter-spacing: -0.03em;
  }
  .sub {
    margin: 6px 0 0;
    color: var(--dim);
    font-size: 14.5px;
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
  .primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    height: 48px;
    min-height: 44px;
    padding: 0 20px;
    border-radius: 999px;
    background: var(--btn);
    color: var(--onbtn);
    font-size: 15px;
    font-weight: 700;
  }
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .primary:not(:disabled):hover {
    filter: brightness(1.06);
  }
  .wide {
    width: 100%;
  }
  .halo {
    display: none;
  }
  @media (min-width: 641px) {
    .card {
      padding: 32px;
    }
  }
  @media (min-width: 900px) {
    .page::before {
      content: "";
      position: fixed;
      left: 50%;
      top: 50%;
      width: min(680px, 74vw);
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
      width: min(480px, 56vw);
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
      width: min(290px, 38vw);
      aspect-ratio: 1;
      transform: translate(-50%, -50%);
      border-radius: 50%;
      border: 1px solid var(--ln);
      pointer-events: none;
      z-index: 0;
      opacity: 0.4;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .primary {
      transition: none;
    }
  }
</style>
