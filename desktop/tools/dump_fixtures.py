#!/usr/bin/env python3
"""Write one JSON file per read route of the web app's API, from the REAL server.

The desktop's model (`crates/nz-model`) is proven against these files, not
against hand-written samples: `tests/fixtures.rs` deserializes every one of them
with the type named in its table. In CI this script runs first, against the
server at the commit under test, and the test then reads what was just written —
so a payload change that would break the desktop fails the desktop job
(docs/agents/desktop.md, lot D1).

    python desktop/tools/dump_fixtures.py                  # the committed copy
    NZ_FIXTURES_DIR=/tmp/fx python desktop/tools/dump_fixtures.py
    python desktop/tools/dump_fixtures.py --out /tmp/fx --keep-server

What it does:

1. starts `tools/perf_api.py` on a throwaway SQLite database seeded with 300
   tracks, 5 playlists (one of 200 titles), 120 favourites, a show with eight
   episodes and a ready transcript, and two years of listening history — the
   numbers the lot pins, so the payloads carry real rows instead of empty lists;
2. waits for its `READY <port> <ids-json>` line and logs in as `bench`;
3. GETs one route per read method of `webapp/src/lib/api.js`, with the ids the
   seed printed, and writes each body verbatim;
4. for the three read routes whose payload is EMPTY until something has been
   done (a listening position, a marker, a share link), first does that one
   write on the throwaway bench, then reads what it made.

Every fixture is a real server answer, so nothing here may compare VALUES: the
bench's library is random on purpose (compare types, and note the nullability —
that is what the model's `#[serde(default)]` is for).
"""

import argparse
import http.cookiejar
import json
import os
import subprocess
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request

ROOT = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
PERF_API = os.path.join(ROOT, "tools", "perf_api.py")

# The seed the lot pins. `--recap` is what gives /api/history, /api/recap/years
# and /api/recap/<year> real rows: without it they answer an empty shell, and the
# three types would only ever be proven against `[]`.
SEED = [
    "--tracks", "300",
    "--playlists", "5",
    "--favorites", "120",
    "--big-playlist", "200",
    "--podcasts",
    "--recap",
    # The two shapes the fixture set answered EMPTY, which is how two wrong
    # `nz-model` types survived: `/api/lyrics/<id>` always said `{"lyrics": null}`
    # and `/api/genre/candidates` always `[]`. These seed a real `.lrc` sidecar and
    # a few labelled tracks with embeddings, so the object (and the ranked tuple
    # `[label, score, head, proto]`) is what gets captured.
    "--seed-lyrics",
    "--seed-genre-scores",
]

# How long the bench may take to fill its database and answer READY.
STARTUP_TIMEOUT = 600

DEFAULT_OUT = os.path.join(
    ROOT, "desktop", "crates", "nz-model", "tests", "fixtures"
)


class Bench:
    """A perf_api server, its session, and one JSON getter."""

    def __init__(self, keep=False):
        self.keep = keep
        self.process = None
        self.port = None
        self.ids = {}
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar)
        )

    # -- lifecycle ---------------------------------------------------------
    def start(self):
        self.process = subprocess.Popen(
            [sys.executable, PERF_API, *SEED, "--serve", "--port", "0"],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stdin=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        watchdog = threading.Timer(STARTUP_TIMEOUT, self._timeout)
        watchdog.daemon = True
        watchdog.start()
        try:
            for line in self.process.stdout:
                line = line.strip()
                if line.startswith("READY "):
                    _, port, raw_ids = line.split(" ", 2)
                    self.port = int(port)
                    self.ids = json.loads(raw_ids)
                    break
                if line:
                    print("  bench:", line, file=sys.stderr)
            else:
                raise SystemExit(
                    "perf_api.py exited before READY; its stderr is above"
                )
        finally:
            watchdog.cancel()
        if self.port is None:
            raise SystemExit("perf_api.py never answered READY")
        return self

    def _timeout(self):
        """A start that hangs: SIGTERM first (it removes its library on it, and
        /tmp is RAM), and insist only after a wait."""
        if self.process is None or self.process.poll() is not None:
            return
        self.process.terminate()
        insist = threading.Timer(30, self._kill_if_running)
        insist.daemon = True
        insist.start()

    def _kill_if_running(self):
        if self.process is not None and self.process.poll() is None:
            self.process.kill()

    def stop(self):
        if self.process is None or self.keep:
            if self.keep:
                print(
                    f"  bench still serving on 127.0.0.1:{self.port} (--keep-server)",
                    file=sys.stderr,
                )
            return
        self.process.terminate()  # SIGTERM: perf_api.py removes its library on it
        try:
            self.process.wait(timeout=20)
        except subprocess.TimeoutExpired:
            self.process.kill()

    # -- requests ----------------------------------------------------------
    def call(self, path, method="GET", body=None):
        url = f"http://127.0.0.1:{self.port}/api{path}"
        data = None
        headers = {"Accept-Encoding": "identity"}
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(url, data=data, method=method, headers=headers)
        try:
            with self.opener.open(request, timeout=60) as answer:
                return answer.status, answer.read()
        except urllib.error.HTTPError as error:
            return error.code, error.read()

    def login(self):
        status, raw = self.call(
            "/login", "POST", {"username": "bench", "password": "Bench1"}
        )
        if status != 200:
            raise SystemExit(f"login failed: {status} {raw[:200]!r}")


def quote(value):
    return urllib.parse.quote(str(value), safe="")


def routes(ids):
    """`(name, path, method, body, optional)` for every read route of api.js.

    One entry per `api.*` READ method (`webapp/src/lib/api.js`). Mutations are
    absent on purpose: the model types what the app reads, and a mutation's
    answer is nothing (`204`) or the next read.
    """
    tracks = ids["ids"]
    podcast = ids["podcast"]
    ready = podcast["ready"]
    show = podcast["show"]

    return [
        # auth, session, who may use what
        ("me", "/me", "GET", None, False),
        ("access", "/access", "GET", None, False),
        ("my-accounts", "/me/accounts", "GET", None, False),
        # onboarding
        ("onboarding", "/onboarding", "GET", None, False),
        ("onboarding-artists", "/onboarding/artists?limit=36", "GET", None, False),
        ("onboarding-search", "/onboarding/search?q=camille&limit=25", "GET", None, False),
        (
            "onboarding-account-options",
            "/onboarding/account/options",
            "GET",
            None,
            False,
        ),
        # library
        ("my-playlists", "/me/playlists", "GET", None, False),
        ("my-favorites", "/me/favorites", "GET", None, False),
        ("my-artists", "/me/artists", "GET", None, False),
        ("my-albums", "/me/albums", "GET", None, False),
        ("my-local", "/me/local", "GET", None, False),
        ("favorite-ids", "/me/favorite-ids", "GET", None, False),
        ("my-dislikes", "/me/dislikes", "GET", None, False),
        ("unavailable", "/unavailable", "GET", None, False),
        # playlists
        ("playlist", f"/playlist/{ids['big']}", "GET", None, False),
        (
            "playlist-recommendations",
            f"/playlist/{ids['small']}/recommendations?limit=12",
            "GET",
            None,
            False,
        ),
        ("playlist-version", f"/playlist/{ids['big']}/version", "GET", None, False),
        ("playlist-members", f"/playlist/{ids['big']}/members", "GET", None, False),
        ("playlist-share", f"/playlist/{ids['small']}/share", "GET", None, False),
        # entities
        ("artist", f"/artist/{ids['artist']}", "GET", None, False),
        ("artist-tracks", f"/artist/{ids['artist']}/tracks", "GET", None, False),
        (
            "artist-discography",
            f"/artist/{ids['artist']}/discography",
            "GET",
            None,
            False,
        ),
        ("album", f"/album/{ids['album']}", "GET", None, False),
        # A server with nothing archived and Deezer off answers five empty
        # families here (see seed_reads): its track rows are the same `Track` the
        # playlist and favourites fixtures prove.
        ("search", "/search?q=night", "GET", None, False),
        ("lyrics", f"/lyrics/{ids['track']}", "GET", None, False),
        # The two REAL answers: an archived track with a synced sidecar, another
        # with plain text (the SPA shows it unsynced).
        (
            "lyrics-synced",
            f"/lyrics/{ids['lyrics'][0]['ref']}",
            "GET",
            None,
            False,
        ),
        ("lyrics-plain", f"/lyrics/{ids['lyrics'][1]['ref']}", "GET", None, False),
        # `/genre/track/<id>` on a LABELLED track: the tag arrives as the OBJECT
        # `_tag_json` writes, not its name.
        (
            "genre-track-tagged",
            f"/genre/track/{ids['genre']['labelled'][0]['ref']}",
            "GET",
            None,
            False,
        ),
        ("gain", f"/gain/{ids['track']}", "GET", None, False),
        ("audio-edges", f"/audio/edges/{ids['track']}?db=-45", "GET", None, False),
        ("gains", "/gains", "POST", {"ids": tracks}, False),
        ("analyses", "/analyses", "POST", {"ids": tracks}, False),
        ("track-analysis", f"/analysis/{ids['track']}", "GET", None, False),
        ("track-credits", f"/track/{ids['track']}/credits", "GET", None, False),
        (
            "credits-person",
            "/credits/person?name=" + quote(ids["writer"]),
            "GET",
            None,
            False,
        ),
        # discovery
        ("flow", "/flow", "GET", None, False),
        ("flow-styles", "/flow/styles", "GET", None, False),
        ("recommend", "/recommend?d=0.30", "GET", None, False),
        ("discover-prefs", "/discover/prefs", "GET", None, False),
        ("history", "/history?limit=5", "GET", None, False),
        ("recap-years", "/recap/years", "GET", None, False),
        ("recap", f"/recap/{ids['recap']['current']}", "GET", None, False),
        ("moods", "/moods", "GET", None, False),
        ("mood", "/moods/energie", "GET", None, False),
        ("releases", "/releases", "GET", None, False),
        ("releases-settings", "/releases/settings", "GET", None, False),
        ("radio-track", f"/radio/track/{ids['track']}", "GET", None, False),
        ("radio-artist", f"/radio/artist/{ids['artist']}", "GET", None, False),
        ("radio-genre", "/radio/genre/Techno", "GET", None, False),
        # podcasts and transcripts
        ("podcasts", "/podcasts", "GET", None, False),
        ("podcast", f"/podcast/{show}?limit=5", "GET", None, False),
        ("search-podcasts", "/search/podcasts?q=histoire", "GET", None, False),
        ("podcast-progress", "/podcast/progress", "GET", None, False),
        ("show-markers", f"/podcast/{show}/markers", "GET", None, False),
        (
            "episode-markers",
            f"/podcast/episode/{ready}/markers",
            "GET",
            None,
            False,
        ),
        ("episode-chapters", f"/podcast/episode/{ready}/chapters", "GET", None, False),
        (
            "episode-transcript",
            f"/podcast/episode/{ready}/transcript?limit=5",
            "GET",
            None,
            False,
        ),
        (
            "show-transcript-search",
            f"/podcast/{show}/transcript/search?q=memoire&limit=5",
            "GET",
            None,
            False,
        ),
        ("transcripts-search", "/transcripts/search?q=memoire&limit=5", "GET", None, False),
        ("transcribe-status", "/transcribe/status", "GET", None, False),
        ("transcribe-settings", "/transcribe/settings", "GET", None, False),
        # sources, settings and the server's own status
        ("sources", "/sources", "GET", None, False),
        ("settings", "/settings", "GET", None, False),
        ("deezer-status", "/deezer/status", "GET", None, False),
        ("storage", "/storage", "GET", None, False),
        ("archive-status", "/archive/status", "GET", None, False),
        ("archive-rules", "/archive/rules", "GET", None, False),
        ("archive-cleanup-preview", "/archive/cleanup/preview", "GET", None, False),
        ("archive-migrate", "/archive/migrate", "GET", None, False),
        ("sync-status", "/sync/status", "GET", None, False),
        ("upload-usage", "/upload/usage", "GET", None, False),
        ("version", "/version", "GET", None, False),
        ("services", "/services", "GET", None, False),
        ("export-formats", "/export/formats", "GET", None, False),
        ("automatch", "/automatch", "GET", None, False),
        ("probe-track", f"/track/{ids['track']}/probe", "GET", None, False),
        (
            "replace-candidates",
            f"/replace/candidates/{ids['track']}",
            "GET",
            None,
            False,
        ),
        ("stream-ready", f"/stream/{ids['track']}/ready?q=OPUS_128", "GET", None, False),
        # administration
        ("admin-users", "/admin/users?limit=25", "GET", None, False),
        ("admin-invites", "/admin/invites?state=pending&limit=25", "GET", None, False),
        ("genre-status", "/genre/status", "GET", None, False),
        ("genre-tree", "/genre/tree", "GET", None, False),
        ("genre-candidates", "/genre/candidates?limit=5", "GET", None, False),
        ("genre-labelled", "/genre/labelled", "GET", None, False),
        ("genre-pool", "/genre/pool?limit=5", "GET", None, False),
        # The track the genre seed gave a vector: a REAL ranking
        # (`[[label, score, head, proto]]`), where `tracks[0]` has none.
        ("genre-track", f"/genre/track/{ids['genre']['ref']}", "GET", None, False),
        ("genre-model", "/genre/model", "GET", None, False),
        ("genre-relabel", "/genre/relabel", "GET", None, False),
        ("genre-embed", "/genre/embed", "GET", None, False),
        ("genre-construction", "/genre/construction", "GET", None, False),
        ("genre-references", "/genre/references", "GET", None, False),
        (
            "genre-references-for",
            "/genre/references?genre=techno",
            "GET",
            None,
            False,
        ),
        ("genre-extractor-test", "/genre/extractor/test", "GET", None, False),
        ("genre-share-status", "/genre/share/import", "GET", None, False),
        ("similar-rebuild", "/similar/rebuild", "GET", None, False),
        ("enrich-backfill", "/enrich/backfill", "GET", None, False),
        ("credits-backfill", "/credits/backfill", "GET", None, False),
        ("analysis-plan", "/analysis/plan", "GET", None, False),
        ("analysis-todo", "/analysis/todo?limit=5", "GET", None, False),
        ("analysis-backfill", "/analysis/backfill", "GET", None, False),
        ("explorer", "/explorer", "GET", None, False),
        # remote control and listen party
        ("remote-links", "/remote/links", "GET", None, False),
        ("party-mine", "/party/mine", "GET", None, False),
    ]


def shared_playlist(bench):
    """Mint one public link on the bench and read the page it opens.

    The read is `api.sharedPlaylist(token)`, and the token only exists once an
    owner asked for a link — on this throwaway database, for this run only.
    """
    status, raw = bench.call(
        f"/playlist/{bench.ids['small']}/share", "POST", {"ttl": 3600}
    )
    if status != 200:
        return None, f"share link refused ({status})"
    token = (json.loads(raw).get("link") or {}).get("token")
    if not token:
        return None, "share link had no token"
    status, raw = bench.call("/shared/playlist/" + quote(token))
    if status != 200:
        return None, f"shared page answered {status}"
    return raw, None


def party_state(bench):
    """Host a listen party, publish one timeline, and read what a guest sees.

    `/api/party/<id>/state` answers the party's PUBLIC timeline, and it says
    nothing at all until its host published one — the fixture this lot lacked.
    A guest has no session, so the read is the same route without a cookie.
    """
    status, raw = bench.call("/party", "POST", {})
    if status != 200:
        return None, f"party start answered {status}"
    pid = (json.loads(raw) or {}).get("id")
    if not pid:
        return None, "the party had no id"
    # The publish anchor must sit within ten minutes of the SERVER's clock, so
    # the first read is what says what that clock is (`now`, milliseconds).
    # The guest's read is `GET /party/<id>` (the SPA's `lib/party/guest.js`);
    # `/state` is the host's POST.
    status, raw = bench.call(f"/party/{quote(pid)}")
    if status != 200:
        return None, f"party state answered {status}"
    now = (json.loads(raw) or {}).get("now")
    # `server_ms()` is a FLOAT (`1794878678797.0752`): an int-shaped anchor is
    # what made `nz-model`'s `PartyAnchor::t` a `u64` and unreadable.
    if not isinstance(now, (int, float)):
        return None, f"the party state carried no server clock: {raw[:200]!r}"
    track = {
        "id": str(bench.ids["track"]),
        "title": "Balancing the Dark",
        "artist": "Kaya Project",
        "album": "Songs from the Inside",
        "duration": 312,
        "podcast": False,
        "gain": -8.5,
    }
    status, raw = bench.call(
        f"/party/{quote(pid)}/state",
        "POST",
        {
            "track": track,
            "playing": True,
            "buf": False,
            "t": now,
            "p": 42.25,
            "xfade": 2,
            "next": {
                "track": dict(track, id=str(bench.ids["ids"][1]), title="Le suivant"),
                "at": 310,
                "start": 0.4,
                "fade": 2,
                "gap": 0.18,
            },
            "norm": "medium",
        },
    )
    if status != 200:
        return None, f"the publish answered {status}: {raw[:160]!r}"
    status, raw = bench.call(f"/party/{quote(pid)}")
    if status != 200:
        return None, f"the party state answered {status} after a publish"
    return raw, None


def seed_reads(bench):
    """The three writes a read route needs to have anything to answer with."""
    ready = bench.ids["podcast"]["ready"]
    bench.call(
        "/podcast/progress",
        "POST",
        {"episode_id": ready, "position": 600, "duration": 3600, "finished": False},
    )
    bench.call(
        f"/podcast/episode/{ready}/markers",
        "POST",
        {"position": 600_000, "label": "La maison"},
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--out",
        default=os.environ.get("NZ_FIXTURES_DIR") or DEFAULT_OUT,
        help="where to write the JSON files (default: the committed copy)",
    )
    parser.add_argument(
        "--keep-server",
        action="store_true",
        help="leave the bench running (prints its port) for a manual look",
    )
    args = parser.parse_args()

    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)

    bench = Bench(keep=args.keep_server)
    failures = []
    written = 0
    try:
        print("starting the bench (300 tracks, 5 playlists, podcasts, recap)…", file=sys.stderr)
        bench.start()
        print(f"bench ready on 127.0.0.1:{bench.port}", file=sys.stderr)
        bench.login()
        seed_reads(bench)

        todo = routes(bench.ids)
        for name, path, method, body, optional in todo:
            status, raw = bench.call(path, method, body)
            if status == 200:
                with open(os.path.join(out, name + ".json"), "wb") as handle:
                    handle.write(raw)
                written += 1
                print(f"  {name:28s} {len(raw):8d} B", file=sys.stderr)
            elif not optional:
                failures.append(f"{name}: {method} {path} answered {status}: {raw[:160]!r}")
                print(f"  {name:28s} FAILED {status}", file=sys.stderr)

        raw, why = shared_playlist(bench)
        if raw is None:
            failures.append(f"shared-playlist: {why}")
        else:
            with open(os.path.join(out, "shared-playlist.json"), "wb") as handle:
                handle.write(raw)
            written += 1
            print(f"  {'shared-playlist':28s} {len(raw):8d} B", file=sys.stderr)

        # The party's public timeline: a guest's read route that has nothing to
        # answer before its host published one (same class as the three writes).
        raw, why = party_state(bench)
        if raw is None:
            failures.append(f"party-state: {why}")
        else:
            with open(os.path.join(out, "party-state.json"), "wb") as handle:
                handle.write(raw)
            written += 1
            print(f"  {'party-state':28s} {len(raw):8d} B", file=sys.stderr)

        # Nothing stale: a fixture the server no longer answers would otherwise
        # keep passing the Rust test against a payload nobody sends any more.
        expected = {name for name, *_ in todo} | {"shared-playlist", "party-state"}
        for name in sorted(os.listdir(out)):
            if name.endswith(".json") and name[:-5] not in expected:
                os.unlink(os.path.join(out, name))
                print(f"  removed stale fixture {name}", file=sys.stderr)
    finally:
        bench.stop()

    print(f"{written} fixtures in {out}", file=sys.stderr)
    if failures:
        print(f"\n{len(failures)} route(s) failed:", file=sys.stderr)
        for failure in failures:
            print("  - " + failure, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
