// What the browser tests share (test/queue/run.mjs, test/player/run.mjs): a
// real server on a big synthetic library, Playwright, and audio to play.
//
// Not a test itself (npm test only picks up *.test.mjs).
import { spawn, execSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";

export async function loadPlaywright() {
  for (const spec of ["playwright", "playwright-core"]) {
    try {
      return await import(spec);
    } catch {
      /* try the next */
    }
  }
  const root = execSync("npm root -g", { stdio: ["ignore", "pipe", "ignore"] }).toString().trim();
  for (const name of ["playwright", "playwright-core"]) {
    const p = join(root, name, "index.mjs");
    if (existsSync(p)) return import(pathToFileURL(p).href);
  }
  throw new Error("Playwright is not installed (npm i -g playwright).");
}

/**
 * tools/perf_api.py --serve: 8 000 tracks, a 4 000-track playlist, Deezer off,
 * against the BUILT SPA (npm run build first). Login: bench / Bench1.
 * Resolves { base, ids, stop }.
 */
export async function startServer(extra = []) {
  const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
  const py =
    process.env.PYTHON || (existsSync(join(repo, ".venv/bin/python")) ? join(repo, ".venv/bin/python") : "python3");
  // DIST=/path/to/dist serves a build made elsewhere (`vite build --outDir …`),
  // so a bench never has to replace neezer/webui/dist another agent is using.
  const dist = process.env.DIST ? ["--dist", resolve(process.env.DIST)] : [];
  const srv = spawn(py, [join(repo, "tools/perf_api.py"), "--runs", "1", "--serve", ...dist, ...extra], {
    stdio: ["pipe", "pipe", "ignore"],
  });
  const ready = await new Promise((res, rej) => {
    let buf = "";
    srv.stdout.on("data", (d) => {
      buf += d;
      const m = buf.match(/READY (\d+) (.*)\n/);
      if (m) res({ port: +m[1], ids: JSON.parse(m[2]) });
    });
    srv.on("exit", () => rej(new Error("server exited")));
  });
  // `stop` is fire-and-forget (a `process.on("exit")` handler cannot await) and
  // a promise at the same time, for the callers that want to see the server go.
  return { base: `http://127.0.0.1:${ready.port}`, ids: ready.ids, stop: () => stopServer(srv) };
}

/** How long a bench server gets between SIGTERM and SIGKILL. */
const STOP_GRACE_MS = 15000;

// Route matchers for the benches. Playwright hands a matcher the request's `URL`, and these
// decide on the HOST or the PATH — never on the whole URL string, where `?next=api.deezer.com`
// would match Deezer's name by accident (CodeQL js/regex/missing-regexp-anchor).

/** Deezer's API. */
export const deezerApi = (u) => u.hostname === "api.deezer.com";

/** Deezer's image CDN (`e-cdns-images.dzcdn.net` and its siblings). */
export const dzcdn = (u) => u.hostname === "dzcdn.net" || u.hostname.endsWith(".dzcdn.net");

/** Every Deezer host a bench keeps the app away from. */
export const deezerHost = (u) => deezerApi(u) || dzcdn(u);

/** A path on the app's own server: `pathOn("/api/stream/")`. */
export const pathOn = (prefix) => (u) => u.pathname.startsWith(prefix);

/**
 * Stop a bench server the way `tools/perf_api.py` expects: SIGTERM first — it
 * removes its 5 MB to 2.3 GB library on SIGTERM, and `/tmp` is RAM — with
 * SIGKILL only as the last resort, which leaves the directory for the next
 * launch's sweep. Safe to call twice, and from `process.on("exit")`.
 * Returns a promise that resolves true once the child is gone.
 */
export function stopServer(srv) {
  if (srv.exitCode !== null || srv.signalCode !== null) return Promise.resolve(true);
  srv.kill("SIGTERM");
  return new Promise((res) => {
    const timer = setTimeout(() => {
      if (srv.exitCode === null && srv.signalCode === null) srv.kill("SIGKILL");
      res(srv.exitCode !== null || srv.signalCode !== null);
    }, STOP_GRACE_MS);
    timer.unref();
    srv.once("exit", () => {
      clearTimeout(timer);
      res(true);
    });
  });
}

/** Silence as a WAV (8 kHz, 8-bit, mono) — the tracks in that library have no audio. */
export function silence(seconds = 60) {
  const rate = 8000;
  const n = rate * seconds;
  const h = Buffer.alloc(44);
  h.write("RIFF", 0);
  h.writeUInt32LE(36 + n, 4);
  h.write("WAVEfmt ", 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(rate, 24);
  h.writeUInt32LE(rate, 28);
  h.writeUInt16LE(1, 32);
  h.writeUInt16LE(8, 34);
  h.write("data", 36);
  h.writeUInt32LE(n, 40);
  return Buffer.concat([h, Buffer.alloc(n, 128)]);
}

export async function login(page, base) {
  await page.goto(base + "/app/");
  await page.evaluate(async () => {
    const r = await fetch("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ username: "bench", password: "Bench1" }),
    });
    if (!r.ok) return;
    const me = await fetch("/api/me").then((x) => x.json());
    if (me?.user?.id) sessionStorage.setItem("__uid", String(me.user.id));
  });
}

/** Page script (ctx.addInitScript, BEFORE the injection one): personal values
 *  live under the account's id, not a name (lib/account.js). login() leaves
 *  that id in sessionStorage, so an injected queue lands where the app reads
 *  it, whatever account the bench runs as. */
export function installSessionPrefix() {
  const uid = sessionStorage.getItem("__uid");
  window.__nsPkey = (k) => `neezer:${location.origin}/${uid ? "id:" + uid : "bench"}:${k}`;
}

export function checker() {
  const state = { failed: 0 };
  const check = (ok, what, extra = "") => {
    console.log(`${ok ? "ok  " : "FAIL"} ${what}${extra ? "  — " + extra : ""}`);
    if (!ok) state.failed++;
  };
  return { check, state };
}
