// Lot T6c — podcasts, nouveautés, import, playlist partagée
// (docs/agents/transitions.md, inventory I12).
//
// One scenario per row of I12 that has a clickable trigger:
//
//   nouveautes-shelves     I6  the two shelves appear after the fetch, and the
//                              "Toutes les sorties" disclosure (I4) opens;
//   podcasts-recherche     I1  the results grid appears once a committed search
//                              answers (never per keystroke: 300 ms debounce);
//   podcast-chapitres      I4  the chapter chip expands its list + turns its
//                              chevron;
//   podcast-panneau        I2/I3  the episode panel's Chapitres/Transcription
//                              tabs (140 ms opacity fade + pill colour);
//   import-progression     I1/I8  the job's screen appears, then its bar grows
//                              by transform: scaleX (never a width);
//   playlist-partagee      SharedPlaylist's root fades in on arrival and its
//                              player bar rises once a title plays.
//
// Rows of I12 without a clickable trigger are not scenarios here:
// `ch-shimmer` (a skeleton, decorative: F7) and GradientHeader's 600 ms colour
// walk (unchanged by this lot, webapp.md).
//
// The answers are mocked on the page (the bench's server has no podcast and
// no Deezer): the requests a scenario makes are therefore the SAME multiset in
// the three motion levels, which is what the runner checks. Every mock answers
// a FIXED number of times — an import job that kept polling would make the
// request multiset depend on the clock.

import { silence } from "../../harness.mjs";

const WAV = silence(30);

const SHOW = "9400001";
const EP = "ep-t6c-1";
const TOKEN = "t6c-token";

const episode = {
  id: EP,
  deezer_id: "9410000",
  podcast: true,
  title: "Épisode 1 · La maison des histoires",
  description: "Un épisode du banc, archivé pour la page d'émission et le panneau.",
  duration: 3600,
  published: 1763000000,
  status: "completed",
  archived: true,
  unavailable: false,
  chapter_count: 3,
  transcript: null,
};

const SHOW_PAYLOAD = {
  id: SHOW,
  deezer_id: SHOW,
  title: "Les Histoires de Neezer",
  cover: null,
  description: "Une heure de récits, découpée en chapitres.",
  local: false,
  episode_count: 1,
  archived_count: 1,
  auto_archive: false,
  auto_transcribe: false,
  transcribe_language: "fr",
  episodes: [episode],
  next: null,
};

const CHAPTERS = {
  archived: true,
  scanned: true,
  source: "deezer",
  duration: 3600,
  chapters: [
    { start: 0, end: 600, title: "Ouverture" },
    { start: 600, end: 1800, title: "La maison" },
    { start: 1800, end: 3600, title: "Le retour" },
  ],
};

// Nouveautés: two shelves (a favourite's release and one close to it), each
// with two titles, so `buildReleaseShelves` fills both and `releaseTotal` > 0.
const releaseCard = (id, closeTo) => ({
  deezer_id: id,
  title: `Sortie ${id}`,
  cover: null,
  kind: "album",
  released: "2026-11-06",
  nb_tracks: 2,
  close_to: closeTo,
  artist: { deezer_id: "9", name: `Groupe ${id}` },
});
const relTrack = (n, releaseId) => ({
  deezer_id: `80${n}`,
  title: `Titre ${n}`,
  duration: 180,
  artist: { deezer_id: "9", name: `Groupe ${releaseId}` },
  display_artist: `Groupe ${releaseId}`,
  release: { deezer_id: releaseId, title: `Sortie ${releaseId}`, cover: null, kind: "album" },
});

const RELEASES = {
  unseen: 2,
  pending: 0,
  releases: [releaseCard("501", null), releaseCard("502", "Groupe 9")],
  tracks: [relTrack(1, "501"), relTrack(2, "501"), relTrack(3, "502"), relTrack(4, "502")],
};

// The import job: `matching` on the first poll (the determinate bar walks to
// 12/40 by scaleX), `done` on the second — exactly two GETs, then no polling.
const IMPORT_ITEMS = [1, 2, 3].map((n) => ({
  line: { title: `Titre original ${n}`, artists: [`Artiste ${n}`], album: "Album", duration: 200 },
  track: {
    id: `imp-track-${n}`,
    deezer_id: `9${n}`,
    title: `Titre retrouvé ${n}`,
    duration: 200,
    artist: { deezer_id: "9", name: `Artiste ${n}` },
    artists: [{ deezer_id: "9", name: `Artiste ${n}`, role: "Main" }],
    display_artist: `Artiste ${n}`,
    album: { deezer_id: "10", title: "Album", cover: "" },
  },
  via: "deezer",
  sure: true,
}));
const IMPORT_JOB = {
  id: "job-t6c",
  source: "spotify",
  source_label: "Spotify",
  kind: "playlist",
  url: "https://open.spotify.com/playlist/t6c",
  name: "Ma sélection",
  cover: null,
  total: 40,
  truncated: false,
  error: null,
  playlist_id: null,
  matched: 12,
  counts: { deezer: 12 },
};

const SHARED = {
  playlist: {
    title: "Une playlist partagée",
    description: "Le lien public, sans compte.",
    cover: null,
    nb_tracks: 2,
    owner: "bench",
  },
  tracks: [
    { mid: "mid-1", title: "Titre partagé 1", duration: 200, streamable: true, artist: { name: "Artiste 1" } },
    { mid: "mid-2", title: "Titre partagé 2", duration: 210, streamable: true, artist: { name: "Artiste 2" } },
  ],
};

/** Install every mock once per page (a second setup must not double them). */
async function mock(page) {
  if (page.__t6cMocked) return;
  page.__t6cMocked = true;
  // When the last request started: `quiet()` below waits for the page to have
  // stopped asking for things (the sidebar's covers, the player's backdrop),
  // so the window the runner measures holds only this scenario's own requests.
  const quietState = { last: Date.now() };
  page.__t6cQuiet = quietState;
  page.on("request", () => {
    quietState.last = Date.now();
  });

  const json = (route, body, status = 200) =>
    route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });

  await page.route((url) => url.pathname === "/api/podcasts", (r) => json(r, { podcasts: [] }));
  await page.route(
    (url) => url.pathname === "/api/search/podcasts",
    (r) =>
      json(r, {
        podcasts: [
          { deezer_id: "9500001", title: "Un podcast trouvé", cover: null, description: "Résultat du banc." },
        ],
      }),
  );
  await page.route((url) => url.pathname === "/api/podcast/" + SHOW, (r) => json(r, SHOW_PAYLOAD));
  await page.route((url) => url.pathname === "/api/podcast/" + SHOW + "/markers", (r) => json(r, { markers: {} }));
  await page.route((url) => url.pathname === `/api/podcast/episode/${EP}/chapters`, (r) => json(r, CHAPTERS));
  await page.route((url) => url.pathname === `/api/podcast/episode/${EP}/markers`, (r) => json(r, { markers: [] }));
  await page.route((url) => url.pathname === "/api/releases", (r) => json(r, RELEASES));
  await page.route((url) => url.pathname === "/api/releases/seen", (r) => json(r, { ok: true }));
  await page.route((url) => url.pathname === "/api/shared/playlist/" + TOKEN, (r) => json(r, SHARED));
  // The About panel's cards (credits, the artist, the mix): answered empty, so
  // the panel settles without a 404 storm. One request each, in every level.
  await page.route((url) => url.pathname.startsWith("/api/radio/track/"), (r) => json(r, { tracks: [] }));
  await page.route((url) => /^\/api\/artist\/[^/]+$/.test(url.pathname), (r) => json(r, { artist: null, top: [], albums: [], related: [] }));
  await page.route((url) => /^\/api\/track\/[^/]+\/credits$/.test(url.pathname), (r) => json(r, { credits: [], info: null }));

  // The import job: POST names it, the first GET is `matching`, the second
  // `done` — and nothing polls after that.
  let polls = 0;
  await page.route((url) => url.pathname === "/api/import/playlist", (r) => json(r, { id: IMPORT_JOB.id }));
  await page.route((url) => url.pathname === "/api/import/playlist/" + IMPORT_JOB.id, (r) => {
    polls++;
    if (polls === 1) return json(r, { ...IMPORT_JOB, state: "matching", done: 12 });
    return json(r, { ...IMPORT_JOB, state: "done", done: 40, items: IMPORT_ITEMS });
  });
  // The episode's audio: the bench's library has none, so a play would 404.
  await page.route((url) => url.pathname.startsWith("/api/stream/"), (r) => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith("/ready")) {
      return json(r, { ready: true, ranges: true, bytes: WAV.length, cached: true, offset: 0, streaming: false });
    }
    return r.fulfill({
      status: 200,
      body: WAV,
      headers: { "Content-Type": "audio/wav", "Accept-Ranges": "none" },
    });
  });

  await awaitWarmUp(page);
}

/** Hash navigation: the app stays mounted, as a tap would leave it. */
async function go(page, hash, selector) {
  await page.evaluate((h) => {
    location.hash = h;
  }, hash);
  if (selector) await page.waitForSelector(selector, { timeout: 30000 });
}

/** The DOM update lands on the next frame; the runner's 50 % picture is taken
 *  right after `act`, so each act ends a beat into the enter it started. */
async function midway(page) {
  await page.waitForTimeout(80);
}

async function frames(page, n = 2) {
  await page.evaluate((count) => window.__motion.frames(count), n);
}

/**
 * Wait until the page is at rest: nothing animating outside a status indicator,
 * and no request started for 1700 ms (longer than the 1.5 s gap between the
 * playlist warm-ups, so a warm-up in flight is waited for, not measured). The
 * window the runner measures then holds this scenario's own requests only —
 * the sidebar's staggered covers and the player's backdrop (which eco never
 * asks for) would otherwise land inside it and make the three levels' request
 * multisets differ.
 */
async function quiet(page) {
  const deadline = Date.now() + 6000;
  while (Date.now() < deadline) {
    const running = await page.evaluate(() =>
      document
        .getAnimations()
        .filter((animation) => {
          const target = animation.effect && animation.effect.target;
          return !(target && target.closest && target.closest("[data-motion]"));
        })
        .map((animation) => (animation.effect && animation.effect.getTiming().duration) || 0),
    );
    const state = page.__t6cQuiet || { last: 0 };
    if (!running.length && Date.now() - state.last > 1700) break;
    await page.waitForTimeout(120);
  }
  await frames(page, 2);
}

/**
 * The app warms its offline copies ten seconds after boot: `/me/favorites`,
 * then one `/api/playlist/<id>` every 1.5 s for up to 25 lists
 * (lib/actions.js#warmPlaylists) — ~40 s of requests that would land inside a
 * measured window and make the three levels' request multisets differ. This
 * waits for that burst to start and finish ONCE per page; the paths are then
 * cached for the session and never asked again. The burst's own gap is 1.5 s,
 * so 4 s of request silence is what tells it has ended.
 */
async function awaitWarmUp(page) {
  const state = page.__t6cQuiet;
  const started = Date.now();
  let seen = false;
  const onRequest = (request) => {
    const path = new URL(request.url()).pathname;
    if (path === "/api/me/favorites" || /^\/api\/playlist\/[^/]+$/.test(path)) seen = true;
  };
  page.on("request", onRequest);
  try {
    while (Date.now() - started < 80000) {
      if (seen && Date.now() - state.last > 4000) break;
      // No warm-up on this server (a small library): do not wait for nothing.
      if (!seen && Date.now() - started > 15000) break;
      await page.waitForTimeout(150);
    }
  } finally {
    page.off("request", onRequest);
  }
}

/** The full-screen player's "À propos" door, whichever shell this viewport has. */
async function openAbout(page, phone) {
  await page.locator("footer.player .now").click();
  await page.waitForSelector(phone ? ".m" : ".d", { timeout: 30000 });
  if (phone) {
    await page.locator(".m .peek").click();
    await page.waitForSelector(".under .seg-tabs", { timeout: 30000 });
  } else {
    await page.locator(".d .tabs button", { hasText: "À propos" }).click();
    await page.waitForSelector(".side .seg-tabs", { timeout: 30000 });
  }
}

export const scenarios = [
  {
    name: "nouveautes-shelves",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/releases", ".shelves .rc");
      await quiet(page);
    },
    async act(page) {
      await page.locator(".all-head button").click();
      await midway(page);
    },
    async settle(page) {
      await page.waitForSelector(".rels .rel", { timeout: 30000 });
      await frames(page, 4);
    },
  },
  {
    name: "podcasts-recherche",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/podcasts", ".search input");
      await quiet(page);
    },
    async act(page) {
      await page.locator(".search input").fill("jazz");
      await midway(page);
    },
    async settle(page) {
      await page.waitForSelector("h2:has-text('Résultats') + .grid .card", { timeout: 30000 });
      await frames(page, 4);
    },
  },
  {
    name: "podcast-chapitres",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/podcast/" + SHOW, ".episodes li");
      await quiet(page);
    },
    async act(page) {
      await page.locator(".episodes li .chapters-chip").first().click();
      await midway(page);
    },
    async settle(page) {
      await page.waitForSelector(".chapters.inline .chapter-row", { timeout: 30000 });
      await frames(page, 4);
    },
  },
  {
    name: "playlist-partagee",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/shared/playlist/" + TOKEN, ".body .hero");
      await quiet(page);
    },
    async act(page) {
      await page.locator(".actions .btn-primary").click();
      await midway(page);
    },
    async settle(page) {
      await page.waitForSelector(".bar.on", { timeout: 30000 });
      await frames(page, 4);
    },
  },
  {
    name: "import-progression",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/import", ".bar input");
      await page.locator(".bar input").fill("https://open.spotify.com/playlist/t6c");
      await quiet(page);
    },
    async act(page) {
      await page.locator(".bar .go").click();
      await midway(page);
    },
    async settle(page) {
      // The job finishes (2 polls): the working screen, then its result.
      await page.waitForSelector(".row.track", { timeout: 30000 });
      await frames(page, 4);
    },
  },
  {
    name: "podcast-panneau",
    midway: true,
    async setup(page) {
      await mock(page);
      await go(page, "#/podcast/" + SHOW, ".episodes li");
      await page.locator(".episodes li .pl").first().click();
      await page.waitForSelector("footer.player .now", { timeout: 30000 });
      await openAbout(page, page.viewportSize().width <= 640);
      await quiet(page);
    },
    async act(page) {
      const scope = page.viewportSize().width <= 640 ? ".under" : ".side";
      await page.locator(`${scope} .seg-tabs button`, { hasText: "Transcription" }).click();
      await midway(page);
    },
    async settle(page) {
      await page.waitForSelector("#tp-transcript:not([hidden])", { timeout: 30000 });
      await frames(page, 4);
    },
  },
];
