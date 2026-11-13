# Neezer — web player (`/app`)

The Svelte 5 + Vite 7 single-page app served by Neezer at **`/app/`**. It talks to the JSON
backend under `/api` and owns the screens, the queue, the offline cache and the visual design;
the audio itself is documented in the playback notes.

The UI is in French. Routing is hash-based (`svelte-spa-router`), so a deep link survives a reload
without any server configuration.

## Develop

Run a Neezer server locally (it serves `/api`), then:

```sh
cd webapp
npm install
npm run dev      # http://localhost:5173, proxies /api -> http://localhost:5000
```

Log in with a Neezer user (the same credentials as the Subsonic API).

## Build

```sh
cd webapp
npm install
npm test         # SPA suite (audio, rhythm, animations, party, remote, clicks…)
npm run build    # outputs to ../neezer/webui/dist
```

The Flask server serves the build at **`/app/`**; the Docker image builds it automatically, so Node
is not needed to deploy. `npm run wasm` rebuilds the Rust crates — commit the `.wasm` files except
`measure.wasm`, which CI and the Dockerfile build.

## Where the rules live

This README is only the entry point. Before changing a screen, read:

- **[docs/agents/webapp.md](../docs/agents/webapp.md)** — Sillon tokens and themes, typography,
  the page layout, navigation, offline cache and service worker, code splitting, queue, search.
- **[docs/agents/playback.md](../docs/agents/playback.md)** — `Player.svelte`, crossfade, silence
  trim, output path, big-list cost.
- **[docs/agents/commands.md](../docs/agents/commands.md)** — every browser bench (clicks, ui,
  requests, queue, fullscreen, panel, render) and the API performance tool.

Do not copy those rules here: when one changes, the area note changes in the same commit.
