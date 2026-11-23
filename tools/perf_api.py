#!/usr/bin/env python3
"""Time the web app's API on a library the size real ones reach.

    python tools/perf_api.py                    # 8000 tracks, the default shape
    python tools/perf_api.py --tracks 20000     # bigger
    python tools/perf_api.py --serve            # ...and keep serving it (for a browser)

A throwaway database is filled through the importer's own upsert path
(``library.upsert_track``, credits included), then every route the player's
screens call is asked for, as the admin, with Deezer switched off — which is
both the offline case and the one where the answer is the database's alone,
so what is measured is this server's own work: queries, serialisation, gzip.

For each route it prints the median wall time over a few runs, the SQL
statement count (the number an N+1 moves by thousands), the response size and
its gzipped size. Nothing here touches the network.

The synthetic library goes in a throwaway ``nsperf-*`` directory under the
system's temporary directory, and every way out of the process removes it —
SIGTERM and SIGINT included, since that is how the browser benches stop their
server. SIGKILL cannot be caught, so a launch first sweeps the directories a
killed bench left behind: each one names its owning pid, and a bench still
running keeps its library.
"""

import argparse
import atexit
import collections
import contextlib
import datetime
import gzip
import json
import os
import random
import shutil
import signal
import statistics
import sys
import tempfile
import time

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
sys.path.insert(0, ROOT)

import peewee  # noqa: E402

from neezer.config import DefaultConfig  # noqa: E402
from neezer.managers.user import UserManager  # noqa: E402
from neezer.web import create_application  # noqa: E402

# The synthetic library lives in a tmpfs on most machines — RAM, not disk. A
# bench that leaves 2.3 GB behind costs the person their memory, so the
# directory is removed on every exit that can be caught (``WorkDir``) and
# swept on the next launch when it could not be (``sweep_orphans``).
WORK_PREFIX = "nsperf-"
OWNER_FILE = "owner"
# A directory with no owner file is judged only once it is this old: a peer
# between its `mkdtemp` and its own owner file looks unowned for an instant.
OWNER_GRACE = 60.0

# The password of every account this bench seeds, and the only one it publishes: the benches
# type it themselves (`webapp/test/harness.mjs`, `admin/run.mjs`, `podcast/run.mjs`). It belongs
# to a throwaway library on loopback, so it is a fixture and not a secret — but it never travels
# in the READY payload, which is a pipe and a CI log keeps what a pipe carried.
BENCH_PASSWORD = "Bench1"


def _pid_alive(pid):
    """Whether ``pid`` is a living process — a zombie is not one.

    A SIGKILLed bench whose launcher has not reaped it yet stays in ``/proc``
    as a zombie, and ``os.kill(pid, 0)`` still succeeds on it; the state in
    ``/proc/<pid>/stat`` is what tells the two apart, and it is the difference
    between a leak nobody sweeps and a bench still serving. Without ``/proc``
    (not Linux) only ``os.kill`` answers.
    """
    if os.path.isdir("/proc"):
        try:
            with open(f"/proc/{pid}/stat", "rb") as handle:
                state = handle.read().rsplit(b")", 1)[1].split()[0]
        except FileNotFoundError:
            return False
        except (IndexError, OSError):
            state = None
        if state is not None:
            return state not in (b"Z", b"X")
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _paths_in_use():
    """Every path a living process sits in or holds open (``/proc`` only).

    This is the second way to recognise a bench: the tools that predate the
    owner file leave no pid behind, but their process still holds the SQLite
    database, the archive and the cache open, and its current directory may be
    the library itself.
    """
    paths = []
    try:
        entries = os.listdir("/proc")
    except OSError:
        return paths
    for entry in entries:
        if not entry.isdigit():
            continue
        base = os.path.join("/proc", entry)
        for item in ("cwd", "fd"):
            try:
                names = [item] if item == "cwd" else os.listdir(os.path.join(base, item))
            except OSError:
                continue
            for name in names:
                try:
                    paths.append(os.readlink(os.path.join(base, item, name)))
                except OSError:
                    continue
    return paths


def sweep_orphans(root=None):
    """Delete the ``nsperf-*`` directories no living bench owns; return them.

    ``WorkDir`` removes its library on SIGTERM, SIGINT, SIGHUP, a normal end
    and an exception. SIGKILL cannot be caught, so a launch starts here: a
    directory is an orphan when its owner file names a process that is gone,
    or — for the directories older tools wrote, which have no owner file —
    when it is past ``OWNER_GRACE`` and no living process holds anything
    inside it. A bench still running is never touched, whichever version wrote
    it, which is what lets several workers run benches side by side.
    """
    root = root or tempfile.gettempdir()
    try:
        names = sorted(os.listdir(root))
    except OSError:
        return []
    removed = []
    in_use = None
    for name in names:
        if not name.startswith(WORK_PREFIX):
            continue
        work = os.path.join(root, name)
        if os.path.islink(work) or not os.path.isdir(work):
            continue
        try:
            with open(os.path.join(work, OWNER_FILE), encoding="utf-8") as handle:
                pid = int(handle.read().split()[0])
        except FileNotFoundError:
            try:
                age = time.time() - os.stat(work).st_mtime
            except OSError:
                continue
            if age < OWNER_GRACE:
                continue
            if in_use is None:
                in_use = _paths_in_use()
            prefix = work + os.sep
            if any(path == work or path.startswith(prefix) for path in in_use):
                continue
        except (IndexError, OSError, ValueError):
            continue
        else:
            if pid == os.getpid() or _pid_alive(pid):
                continue
        shutil.rmtree(work, ignore_errors=True)
        removed.append(work)
    return removed


class WorkDir:
    """The throwaway library, removed on every way out of this process.

    ``atexit`` covers the return, an exception and ``sys.exit``; the signal
    handlers cover SIGTERM — what every bench sends its server first — SIGINT
    and SIGHUP, which would otherwise end the process with its library still
    on disk. SIGKILL cannot be caught: ``sweep_orphans`` collects it.
    """

    SIGNALS = tuple(
        getattr(signal, name)
        for name in ("SIGTERM", "SIGINT", "SIGHUP")
        if hasattr(signal, name)
    )

    def __init__(self, path):
        self.path = path
        self.released = False

    def install(self):
        """Claim the directory for this pid, then arm every clean-up."""
        with open(os.path.join(self.path, OWNER_FILE), "w", encoding="utf-8") as handle:
            handle.write(f"{os.getpid()}\n")
        atexit.register(self.release)
        for sig in self.SIGNALS:
            try:
                signal.signal(sig, self._on_signal)
            except (OSError, ValueError):  # not the main thread, or unsupported
                pass
        return self

    def release(self):
        # The flag is set FIRST: a second signal must not re-enter the removal
        # the interpreter is already part-way through.
        if self.released:
            return
        self.released = True
        shutil.rmtree(self.path, ignore_errors=True)

    def _on_signal(self, signum, _frame):
        self.release()
        # Die of the signal the launcher sent, as an unhandled one would: the
        # parent waits for that exit and the shell reports it.
        signal.signal(signum, signal.SIG_DFL)
        os.kill(os.getpid(), signum)


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql

    def execute_sql(self, sql, *args, **kwargs):
        counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


WORDS = (
    "amour nuit été cœur étoile rêve lumière ciel feu ombre danse vent mer soleil "
    "horizon silence océan éclat encore toujours jamais demain hier ville route "
    "night love fire dream light heart rain gold wild blue storm echo neon"
).split()


def title(r):
    return " ".join(r.choice(WORDS).capitalize() if i == 0 else r.choice(WORDS) for i in range(r.randint(1, 4)))


def seed_recap(app, tracks):
    """The Récap's own log (plan ``recap.md`` §8.4), deterministic to the event.

    Two years for ``bench`` — 25 000 events in the finished year, 12 000 in the
    running one — one event every 187 s, cycled over the first 300 titles, one
    skip in ten. 270 of those 300 carry a measured style (four families, 0.8
    confidence), the last 30 none, so the styles card has an honest ~90 %
    coverage. ``thin`` holds 5 listens (12 minutes) in one year and 3 skips in
    another; ``fresh`` has no history at all. Nothing writes outside the
    throwaway bench database.
    """
    import datetime as _dt

    from neezer.db import ListenEvent, Track, TrackAnalysis, User, db, now
    from neezer.managers.user import UserManager

    current = now().year
    past = current - 1
    empty = current - 2
    played = list(Track.select().order_by(Track.id).limit(300))
    if len(played) < 300:
        raise SystemExit(f"the recap seed needs 300 playable titles, the library has {len(played)}")
    with app.app_context():
        admin = User.get(name="bench")
        rows = []
        # year, events, days spanned: the plan's exact numbers (Jan 1 + i*span//n,
        # 10:00 + i%4 minutes, one skip every ten events).
        for year, total, span in ((past, 25_000, 365), (current, 12_000, 316)):
            start = _dt.datetime(year, 1, 1, 10, 0)
            for i in range(total):
                at = start + _dt.timedelta(days=(i * span) // total, minutes=i % 4)
                rows.append(
                    {
                        "user": admin.id,
                        "track": played[i % 300].id,
                        "at": at,
                        "listened": 187,
                        "outcome": -1 if i % 10 == 0 else 1,
                        "source": "bench",
                    }
                )
        t0 = time.time()
        with db.atomic():
            for i in range(0, len(rows), 150):
                ListenEvent.insert_many(rows[i : i + 150]).execute()
        print(f"  {len(rows)} listen events in {time.time() - t0:.1f}s", file=sys.stderr)

        # The measured styles: the first 270 played titles only. Under
        # --archived every title already has its own analysis; keep it.
        known = {
            row.track_id
            for row in TrackAnalysis.select(TrackAnalysis.track).where(
                TrackAnalysis.track.in_([t.id for t in played])
            )
        }
        styles = ("techno", "house", "dnb", "ambient")
        analysis = [
            {
                "track": t.id,
                "bpm": 128.0,
                "bpm_confidence": 0.8,
                "bpm_source": "bench",
                "style": styles[i % len(styles)],
                "style_confidence": 0.8,
            }
            for i, t in enumerate(played[:270])
            if t.id not in known
        ]
        if analysis:
            TrackAnalysis.insert_many(analysis).execute()

        # A year too thin to tell, a year of only skips, and a person with no
        # history at all: the three empty states of the screen.
        thin = UserManager.add("thin", "Thin1")
        thin_year = current - 2
        skips_year = current - 3
        thin_rows = [
            {
                "user": thin.id,
                "track": played[i].id,
                "at": _dt.datetime(thin_year, 3, 10 + i, 20, 0),
                "listened": 144,
                "outcome": 1,
                "source": "bench",
            }
            for i in range(5)
        ] + [
            {
                "user": thin.id,
                "track": played[10 + i].id,
                "at": _dt.datetime(skips_year, 6, 5 + i, 12, 0),
                "listened": 20,
                "outcome": -1,
                "source": "bench",
            }
            for i in range(3)
        ]
        ListenEvent.insert_many(thin_rows).execute()
        UserManager.add("fresh", "Fresh1")
        return {
            "current": current,
            "past": past,
            "empty": empty,
            "thin": {
                "year": thin_year,
                "skips_year": skips_year,
                "listens": 5,
                "minutes": 12,
                "skip_events": 3,
            },
        }


def populate(app, n_tracks, n_playlists, n_fav, big_playlist, recap=False):
    from neezer.db import Playlist, PlaylistTrack, StarredTrack, User
    from neezer.deezer import library

    r = random.Random(7)
    with app.app_context():
        admin = User.get(name="bench")
        root = library.get_root_folder(app.config["DEEZER"]["archive_dir"])
        n_artists = max(20, n_tracks // 16)
        n_albums = max(40, n_tracks // 6)
        artists = [(str(1_000_000 + i), f"{title(r)} {i}") for i in range(n_artists)]
        albums = []
        for i in range(n_albums):
            a = r.choice(artists)
            albums.append((str(2_000_000 + i), f"{title(r)} {i}", a, f"{i:032x}"))
        tracks = []
        t0 = time.time()
        for i in range(n_tracks):
            alb = r.choice(albums)
            raw = {
                "SNG_ID": str(3_000_000 + i),
                "SNG_TITLE": title(r),
                "ART_ID": alb[2][0],
                "ART_NAME": alb[2][1],
                "ALB_ID": alb[0],
                "ALB_TITLE": alb[1],
                "ALB_PICTURE": alb[3],
                "DURATION": r.randint(90, 420),
                "TRACK_NUMBER": 1 + i % 14,
                "DISK_NUMBER": 1,
                "TRACK_TOKEN": f"tok{i}",
            }
            if r.random() < 0.2:
                feat = r.choice(artists)
                raw["ARTISTS"] = [
                    {"ART_ID": alb[2][0], "ART_NAME": alb[2][1], "ROLE_ID": "0"},
                    {"ART_ID": feat[0], "ART_NAME": feat[1], "ROLE_ID": "5"},
                ]
            tracks.append(library.upsert_track(raw, root))
        print(f"  {n_tracks} tracks in {time.time() - t0:.1f}s", file=sys.stderr)
        # Everybody else on the sleeves: two to four people a track out of a
        # pool of writers and producers, one of them prolific (a credits page
        # of hundreds of titles), and the first artist writing for others too.
        from neezer.db import TrackCredit

        pool = [f"{title(r)} {k}" for k in range(400)]
        star_writer, artist_writer = "Max Prolific", artists[0][1]
        rows = []
        for k, t in enumerate(tracks):
            people = [("composer", r.choice(pool)), ("author", r.choice(pool))]
            if r.random() < 0.5:
                people.append(("producer", r.choice(pool)))
            if k % 20 == 0:
                people.append(("composer", star_writer))
            if k % 25 == 1:
                people.append(("producer", artist_writer))
            rows += [
                {"track": t.id, "source": "deezer", "role": role, "name": name,
                 "name_key": name.casefold(), "position": pos}
                for pos, (role, name) in enumerate(people)
            ]
        for i in range(0, len(rows), 500):
            TrackCredit.insert_many(rows[i : i + 500]).execute()
        pls = []
        for p in range(n_playlists):
            pl = Playlist.create(user=admin, name=f"Playlist {p} {title(r)}")
            size = big_playlist if p == 0 else r.randint(5, 150)
            rows = r.sample(tracks, min(size, len(tracks)))
            if rows:
                PlaylistTrack.insert_many(
                    [{"playlist": pl.id, "track": t.id, "index": k} for k, t in enumerate(rows)]
                ).execute()
            pls.append(pl)
        favorites = r.sample(tracks, min(n_fav, len(tracks)))
        if favorites:
            StarredTrack.insert_many(
                [{"user": admin.id, "starred": t.id} for t in favorites]
            ).execute()
        # Followed artists, kept albums, and what the followed put out lately
        # (the home's "Nouveautés" and "Tes artistes", the library's chips).
        from neezer.db import Album, Artist, ArtistRelease, StarredAlbum, StarredArtist

        rows = list(Artist.select().where(Artist.deezer_id.is_null(False)).limit(300))
        StarredArtist.insert_many([{"user": admin.id, "starred": a.id} for a in rows]).execute()
        StarredAlbum.insert_many(
            [{"user": admin.id, "starred": a.id} for a in Album.select().limit(200)]
        ).execute()
        today = datetime.date.today()
        rel = []
        for k, a in enumerate(rows[:60]):
            hits = [{"r": str(9_000_000 + k * 20 + j), "t": title(r), "d": r.randint(120, 300), "e": False,
                     "a": [[a.deezer_id, a.name]]} for j in range(r.choice((1, 1, 4, 12)))]
            rel.append({
                "provider": "deezer", "ref": str(8_000_000 + k), "artist_ref": a.deezer_id,
                "artist_name": a.name, "title": title(r), "kind": "single" if len(hits) == 1 else "album",
                "released": (today - datetime.timedelta(days=k)).isoformat(), "nb_tracks": len(hits),
                "tracks": json.dumps(hits), "found": datetime.datetime.now(),
            })
        ArtistRelease.insert_many(rel).execute()
        out = {
            "big": str(pls[0].id),
            "small": str(pls[-1].id),
            "album": albums[0][0],
            "artist": artists[0][0],
            "track": str(tracks[0].deezer_id) if tracks else "",
            "writer": "Max Prolific",
            "ids": [str(t.deezer_id) for t in tracks[:12]],
        }
        if recap:
            out["recap"] = seed_recap(app, tracks)
        return out


def seed_onboarding(app, tracks):
    """The list, its neighbours and the invitations the onboarding bench redeems.

    The feed is the library's own (Deezer is off): 50 artists with 800 tracks,
    plus ``Camille Fictive`` for the search. Every artist is given two cached
    neighbours among artists that own no track, so a like answers immediately
    with two insertable tiles instead of waiting for Deezer. Three single-use
    tokens: one per viewport and one spare.
    """
    import uuid

    from neezer import invites as invite_mod
    from neezer.db import Artist, ArtistLink, Track, User
    from neezer.deezer import library

    with app.app_context():
        admin = User.get(name="bench")
        tokens = []
        for i in range(3):
            row, token = invite_mod.create(
                admin, note=f"Banc onboarding {i + 1}", expires_in=7 * 24 * 3600
            )
            tokens.append({"id": row.id, "token": token, "note": row.note})

        if tracks:
            root = library.get_root_folder(app.config["DEEZER"]["archive_dir"])
            known = None
            for i in range(2):
                raw = {
                    "SNG_ID": str(9_900_001 + i),
                    "SNG_TITLE": f"Le banc des histoires {i + 1}",
                    "ART_ID": "9900001",
                    "ART_NAME": "Camille Fictive",
                    "ALB_ID": "9900001",
                    "ALB_TITLE": "Banc des artistes",
                    "ALB_PICTURE": "",
                    "DURATION": 240,
                    "TRACK_NUMBER": i + 1,
                    "DISK_NUMBER": 1,
                    "TRACK_TOKEN": f"bench-onboarding-{i}",
                }
                track = library.upsert_track(raw, root)
                Track.update(play_count=50 + i).where(Track.id == track.id).execute()
                known = track.artist
            related = []
            parents = list(Artist.select().where(Artist.deezer_id.is_null(False)))
            for index, parent in enumerate(parents):
                for k in range(2):
                    other = Artist.create(
                        id=uuid.uuid4(),
                        name=f"Proche {parent.name} {k + 1}",
                        deezer_id=str(9_500_000 + index * 10 + k),
                    )
                    related.append(
                        {
                            "provider": "deezer",
                            "artist_ref": str(parent.deezer_id),
                            "other_ref": str(other.deezer_id),
                            "other_name": other.name,
                            "other_picture": None,
                            "score": 1.0 - k / 10.0,
                            "pos": k + 1,
                        }
                    )
            if related:
                ArtistLink.insert_many(related).execute()
            if known is None:
                known = Artist.get_or_none(Artist.name == "Camille Fictive")
        else:
            known = None

        return {
            "tokens": tokens,
            "known_artist": known.name if known is not None else None,
            "artists": Artist.select().count(),
        }


def seed_provider_account(app):
    """One linked provider account for ``bench``, and nothing that reaches out.

    ``/api/me/accounts`` lists what a person linked; without a row the whole
    surface answers an empty list, and a client cannot be tested against the
    ``PUT /api/me/accounts/<id>`` contract (the shape of ``auto``). The row
    carries no secret and the provider is never called: ``li.options`` only
    reads the row's own JSON, and ``li.status`` its in-memory dict.

    Opt-in (``--seed-provider-account``): the default bench output is what every
    other bench and fixture already pins, so it must not grow an entry.
    """
    from neezer.db import ProviderAccount, User, now

    with app.app_context():
        user = User.get(name="bench")
        row = ProviderAccount.create(
            provider="deezer",
            user=user.id,
            label="Banque essentielle",
            mode="arl",
            status="ok",
            checked=now(),
            created=now(),
        )
        return {"id": str(row.id)}


def seed_admin(app, users, invite_count):
    """The accounts and pending invitations Réglages → Utilisateurs lists.

    One argon2 hash is computed and reused. ``membre`` is the non-admin session
    the admin bench logs in as, ``desactive_moi`` the active row a PATCH
    disables, ``deja_desactive`` the disabled row the filter keeps,
    ``supprime_moi`` the row a DELETE removes. Every seeded account takes
    ``BENCH_PASSWORD``, the bench's own published fixture: a credential the
    benches must type never travels in the READY payload, which is a pipe and a
    CI log keeps what a pipe carried (CodeQL
    py/clear-text-logging-sensitive-data).
    """
    import datetime as _dt

    from neezer import invites as invite_mod
    from neezer.db import User, now
    from neezer.managers.user import UserManager, encrypt_password

    crypt, salt = UserManager._hash_fields(BENCH_PASSWORD)
    clear = encrypt_password(BENCH_PASSWORD)
    base = now()

    def add(name, admin=False, disabled=False, minutes=0, mail=None):
        return User.create(
            name=name,
            name_key=name,
            mail=mail,
            password=crypt,
            salt=salt,
            password_clear=clear,
            admin=admin,
            disabled=disabled,
            last_login=base - _dt.timedelta(minutes=minutes) if minutes else None,
            onboarding_step="done",
        )

    with app.app_context():
        admin = User.get(name="bench")
        add("membre", minutes=12)
        add("desactive_moi", minutes=60 * 24 * 4)
        add("deja_desactive", disabled=True, minutes=60 * 24 * 6)
        add("supprime_moi", minutes=60 * 24 * 9)
        camilles = max(0, min(20, users))
        for i in range(camilles):
            add(f"camille{i + 1:02d}", minutes=60 * (i + 1))
        for i in range(max(0, users - camilles)):
            add(f"membre{i + 1:03d}", minutes=60 * (i + 25))
        invites = []
        for i in range(max(0, invite_count)):
            row, _token = invite_mod.create(
                admin, note=f"Invitation {i + 1:02d}", expires_in=7 * 24 * 3600
            )
            invites.append({"id": row.id, "note": row.note})
        return {
            "member": "membre",
            "delete": "supprime_moi",
            "disable": "desactive_moi",
            "disabled": "deja_desactive",
            "camille": "camille01",
            "invites": invites,
            "admin_id": str(admin.id),
        }


def _silent_wav(path, seconds=1):
    """A silent WAV: the routes that matter here only check that the archived
    file exists (the bench answers /api/stream itself)."""
    import wave

    os.makedirs(os.path.dirname(path), exist_ok=True)
    with wave.open(path, "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(1)
        handle.setframerate(8000)
        handle.writeframes(b"\x80" * 8000 * max(1, seconds))
    return path


def _archived(app, ref, stem, seconds=1, touch=True):
    """Point the row named by `ref` (a Deezer id) at a real file under `stem`,
    and hand the ROW back re-read: `save_embedding` and `read_sidecar` derive
    their sidecar from `track.path`, and a stale object still holds the old one.

    `touch` stamps `last_modification`, which is what `/genre/candidates` scans
    and what makes a row a LOCAL search hit: the lyrics seed leaves it alone, so
    it adds no search result to the fixtures other lots pinned.
    """
    import uuid as _uuid

    from neezer.db import Track
    from neezer.deezer import ids as dzids

    path = _silent_wav(stem + ".wav", seconds=seconds)
    key = dzids.track_uuid(str(ref))
    stamp = 1700000000 if touch else 0
    Track.update(path=path, last_modification=stamp).where(Track.id == key).execute()
    row = Track.get_or_none(Track.id == key)
    assert row is not None and row.path == path, f"archivage impossible pour {ref}"
    assert _uuid.UUID(str(row.id))  # the row id is a UUID, never a Deezer number
    return row


def seed_lyrics(app, ids):
    """Two archived tracks WITH a `.lrc` sidecar, so `/api/lyrics/<id>` answers

        {"lyrics": {"text": ..., "synced": [{"time", "text"}], "source": "archive"}}

    and, for the second one, the plain-text variant (`synced: []`). The fixture
    this route had before answered `null` (nothing archived), which is how
    `nz_model::Lyrics` kept declaring the words as a bare string: the object was
    never seen. Pure stdlib: a WAV written here, no ffmpeg, no LRCLIB.

    The two refs are chosen away from the genre seed's (a second seed that
    archives the same row would silently repoint it).
    """
    with app.app_context():
        archive = app.config["DEEZER"]["archive_dir"]
        specs = [
            ("synced", "[00:01.50]Premier vers\n[00:03.25]Deuxième vers\n[00:07.00]Dernier vers\n"),
            ("plain", "Des paroles sans temps\nDeuxième ligne\n"),
        ]
        out = []
        for index, (kind, text) in enumerate(specs):
            ref = ids["ids"][index]
            stem = os.path.join(archive, f"lyrics-{kind}")
            row = _archived(app, ref, stem, seconds=30, touch=False)
            with open(stem + ".lrc", "w", encoding="utf-8") as handle:
                handle.write(text)
            out.append({"id": str(row.id), "ref": str(ref), "kind": kind})
        return out


def seed_genre_scores(app, ids):
    """Vectors for a few tracks and three hand labels, so `/genre/candidates` and
    `/genre/track/<id>` answer a RANKED list instead of `[]`.

    `genre.rank_scores` needs prototypes, and prototypes come from labelled tracks
    that have an embedding (`genre.centroids` -> `_labelled_rows`): no trained
    head is required, so this writes v2-width float16 sidecars (the shape
    `embedding.common_part` reads) and the `GenreTag`/`TrackTag` rows. The
    fixture this route had before was `{"candidates": []}`, which is how
    `nz_model::GenreScore` kept declaring an OBJECT where the server sends the
    tuple `[label, score, head, proto]`.

    `/genre/candidates` only scans rows with `last_modification > 0` (a track the
    library has touched), so the archived rows get that stamp too.
    """
    from neezer.db import GenreTag, TrackTag
    from neezer.deezer import embedding as emb

    with app.app_context():
        archive = app.config["DEEZER"]["archive_dir"]
        width = emb.V2_EMBED_DIM
        third = width // 3

        def vector(weights, tilt=0.0):
            """Three blocks, one per genre, so the prototypes point apart; `tilt`
            nudges a target's vector so its ranking is not an exact tie."""
            vec = [0.0] * width
            for block, weight in enumerate(weights):
                for i in range(block * third, (block + 1) * third):
                    vec[i] = weight / max(1, third) ** 0.5 + tilt * ((i % 7) - 3) * 0.0001
            return vec

        blocks = {
            "Hardtek": (0.9, 0.1, 0.1),
            "Ambient": (0.1, 0.9, 0.1),
            "Techno": (0.1, 0.1, 0.9),
        }
        labelled = []
        # 3, 4, 5: the lyrics seed owns 0 and 1.
        for index, (name, weights) in enumerate(blocks.items()):
            ref = ids["ids"][3 + index]
            row = _archived(app, ref, os.path.join(archive, f"genre-{index}"))
            tag, _ = GenreTag.get_or_create(
                name=name, defaults={"color": None, "archetype": None}
            )
            TrackTag.get_or_create(track=row, tag=tag)
            assert emb.save_embedding(row, vector(weights)), f"embedding non écrit pour {name}"
            labelled.append({"name": name, "ref": str(ref), "scores": 0})

        # The fixture's own track: an embedding, untagged, so `/genre/track/<id>`
        # ranks the vocabulary for it and `/genre/candidates` lists it.
        ref = ids["ids"][6]
        target = _archived(app, ref, os.path.join(archive, "genre-target"))
        assert emb.save_embedding(target, vector((0.5, 0.3, 0.2), tilt=1.0)), "embedding non écrit"
        return {"labelled": labelled, "track": str(target.id), "ref": str(ref)}


def seed_podcasts(app):
    """A show with two scanned episodes, a ready transcript, a running one.

    The audio files are one-second WAVs: the chapter and transcript routes only
    check that the archived file exists, and the bench answers ``/api/stream``
    itself. The transcript has 240 15-second lines, so 10:00 is exactly a
    segment boundary; every twentieth line says « mémoire ».
    """
    import datetime as _dt
    import uuid
    import wave

    from neezer.db import (
        PodcastChannel,
        PodcastChapter,
        PodcastChapterScan,
        PodcastEpisode,
        PodcastTranscript,
        PodcastTranscriptSegment,
        User,
        now,
    )
    from neezer.deezer.library import PODCAST_DIR_NAME
    from neezer.managers.user import UserManager
    from neezer.transcribe import job

    with app.app_context():
        admin = User.get(name="bench")
        listeners = []
        for i in range(3):
            name = f"podcast{i + 1}"
            UserManager.add(name, BENCH_PASSWORD)
            listeners.append({"name": name})
        archive = app.config["DEEZER"]["archive_dir"]
        folder = os.path.join(archive, PODCAST_DIR_NAME, "banc")
        os.makedirs(folder, exist_ok=True)
        channel = PodcastChannel.create(
            id=uuid.uuid4(),
            user=admin,
            deezer_id="9400001",
            url="https://www.deezer.com/show/9400001",
            title="Les Histoires de Neezer",
            description="Une heure de récits, découpée en chapitres et transcrite sur le serveur.",
            subscribed=True,
            auto_archive=False,
            auto_transcribe=False,
        )
        specs = [
            ("Épisode 1 · La maison des histoires", 3),
            ("Épisode 2 · Cent sections", 100),
            ("Épisode 3 · En cours de transcription", 0),
            ("Épisode 4 · La transcription a échoué", 0),
            ("Épisode 5 · Sans transcription", 0),
            ("Épisode 6 · Le retour", 0),
            ("Épisode 7 · Les archives", 0),
            ("Épisode 8 · Le dernier mot", 0),
        ]
        episodes = []
        for index, (title, _chapters) in enumerate(specs):
            path = os.path.join(folder, f"episode-{index + 1}.wav")
            if not os.path.exists(path):
                with wave.open(path, "wb") as handle:
                    handle.setnchannels(1)
                    handle.setsampwidth(1)
                    handle.setframerate(8000)
                    handle.writeframes(b"\x80" * 8000)
            episodes.append(
                PodcastEpisode.create(
                    id=uuid.uuid4(),
                    channel=channel,
                    deezer_id=str(9_410_000 + index),
                    title=title,
                    description="Un épisode du banc, archivé pour la page d'émission et le panneau.",
                    duration=3600,
                    publish_date=now() - _dt.timedelta(days=index + 1),
                    stream_url=f"https://example.invalid/episode-{index + 1}.mp3",
                    image_md5=None,
                    path=path,
                    bitrate=64,
                    status="completed",
                )
            )

        for index, count in ((0, 3), (1, 100)):
            episode = episodes[index]
            starts = [0, 600_000, 1_800_000]
            ends = [600_000, 1_800_000, 3_600_000]
            rows = []
            for k in range(count):
                rows.append(
                    {
                        "id": uuid.uuid4(),
                        "episode": episode.id,
                        "idx": k,
                        "start_ms": starts[k] if count == 3 else k * 36_000,
                        "end_ms": ends[k] if count == 3 else (k + 1) * 36_000,
                        "title": (
                            ["Introduction", "La maison", "Les invités"][k]
                            if count == 3
                            else f"Section {k + 1}"
                        ),
                        "source": "id3",
                    }
                )
            PodcastChapter.insert_many(rows).execute()
            PodcastChapterScan.create(
                episode=episode.id,
                scanned_at=now(),
                source="id3",
                n_chapters=count,
            )

        ready = episodes[0]
        transcript = PodcastTranscript.create(
            episode=ready.id,
            status="ready",
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            engine_version="1.7.4",
            duration_ms=3_600_000,
            done_ms=3_600_000,
            n_segments=240,
            source="server",
            user=admin,
            queued_at=now() - _dt.timedelta(hours=3),
            started_at=now() - _dt.timedelta(hours=2),
            finished_at=now() - _dt.timedelta(hours=1),
        )
        lines = []
        for k in range(240):
            start = k * 15_000
            text = (
                f"Ligne {k + 1}. On parle de mémoire et d'histoires, de la maison, "
                "des invités et de ce qui reste après la dernière phrase."
            )
            lines.append(
                {
                    "transcript": transcript.episode_id,
                    "idx": k,
                    "start_ms": start,
                    "end_ms": start + 15_000,
                    "text": text,
                    "search_text": job.fold(text),
                }
            )
        PodcastTranscriptSegment.insert_many(lines).execute()

        running = episodes[2]
        PodcastTranscript.create(
            episode=running.id,
            status="running",
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            duration_ms=3_600_000,
            done_ms=1_512_000,
            n_segments=0,
            source="client",
            worker_name="portable-banc",
            user=admin,
            queued_at=now() - _dt.timedelta(minutes=30),
            started_at=now() - _dt.timedelta(minutes=20),
        )
        broken = episodes[3]
        PodcastTranscript.create(
            episode=broken.id,
            status="error",
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            duration_ms=3_600_000,
            done_ms=91_000,
            n_segments=0,
            error_code="engine_crashed",
            error_message="le moteur whisper.cpp s'est arrêté (code 139)",
            source="server",
            user=admin,
            queued_at=now() - _dt.timedelta(hours=4),
            started_at=now() - _dt.timedelta(hours=3),
        )
        return {
            "show": str(channel.id),
            "episodes": [str(episode.id) for episode in episodes],
            "chapters3": str(episodes[0].id),
            "chapters100": str(episodes[1].id),
            "ready": str(episodes[0].id),
            "running": str(episodes[2].id),
            "error": str(episodes[3].id),
            "listeners": listeners,
        }


def make_archived(app, clusters=40):
    """The library as a used server holds it: every track on disk, a third of
    them played at some point, each measured (style, tempo) and carrying a
    vector, so the radio, the flow and the home have something to read. Prints
    how long the full neighbours rebuild takes."""
    import datetime

    import numpy as np

    from neezer import similar
    from neezer.db import Track, TrackAnalysis, now
    from neezer.deezer import embedding as emb

    styles = ["frenchcore", "techno", "house", "trance", "dnb", "hardstyle", "ambient",
              "pop", "rock", "jazz", "hiphop", "dubstep", "psytrance", "uptempo"]
    r = random.Random(11)
    rs = np.random.RandomState(3)
    centres = rs.randn(clusters, emb.V2_EMBED_DIM)
    t0 = time.time()
    with app.app_context():
        base = now()
        rows = []
        analyses = []
        for t in Track.select():
            plays = r.randint(1, 30) if r.random() < 0.33 else 0
            last = base - datetime.timedelta(days=r.randint(0, 400)) if plays else None
            c = r.randrange(clusters)
            rows.append((t, plays, last, base - datetime.timedelta(days=r.randint(0, 300))))
            loud = (c % 10) / 9 * 0.8 + r.random() * 0.2  # the energy's measures
            analyses.append({"track": t.id, "bpm": 80 + (c % 12) * 10 + r.random() * 3,
                             "bpm_confidence": 0.8, "bpm_source": "bench",
                             "style": styles[c % len(styles)],
                             # The Récap's style card counts a title only when
                             # its verdict is SURE (`recap._genres` drops a
                             # style under STYLE_MIN_CONF): without this column
                             # an archived library read as 0 % measured, and
                             # `--archived --recap` failed its own check.
                             "style_confidence": 0.9,
                             "data": json.dumps({"lufs": -20 + 14 * loud, "lra": 14 - 10 * loud,
                                                 "centroid": 1200 + 2500 * loud,
                                                 "fluxPeak": 0.2 + 0.7 * loud,
                                                 "entropy": 0.5 + 0.3 * loud})})
            vec = centres[c] + rs.randn(emb.V2_EMBED_DIM) * 0.4
            os.makedirs(os.path.dirname(t.path), exist_ok=True)
            emb.save_embedding(t, [float(x) for x in vec])
        with Track._meta.database.atomic():
            for t, plays, last, created in rows:
                Track.update(last_modification=1700000000, play_count=plays, last_play=last,
                             created=created).where(Track.id == t.id).execute()
            # The listening seed (``--recap``) already measured the 300 titles
            # its own events play, and a row per track is UNIQUE. Those rows are
            # UPDATED in place: an INSERT OR REPLACE deletes the row first, and
            # the cascade took the title's embedding with it — measured, that
            # dropped the recap's style coverage from 1.00 to 0.00.
            existing = {
                row.track_id
                for row in TrackAnalysis.select(TrackAnalysis.track).where(
                    TrackAnalysis.track.in_([row["track"] for row in analyses])
                )
            }
            fresh = [row for row in analyses if row["track"] not in existing]
            for row in analyses:
                if row["track"] in existing:
                    fields = {key: value for key, value in row.items() if key != "track"}
                    TrackAnalysis.update(**fields).where(
                        TrackAnalysis.track == row["track"]
                    ).execute()
            for i in range(0, len(fresh), 500):
                TrackAnalysis.insert_many(fresh[i:i + 500]).execute()
        print(f"  archived + measured + vectors in {time.time() - t0:.1f}s", file=sys.stderr)
        t0 = time.time()
        snap = similar.rebuild_job().run()
        print(
            f"  neighbours rebuilt for {snap['done']} tracks in {time.time() - t0:.1f}s "
            f"({snap['failed']} failed)",
            file=sys.stderr,
        )
        similar.drop_index()


def seed_releases(app, count):
    """``count`` recent releases for the artists 'bench' already follows.

    The shipped ``GET /api/releases`` is capped at 120 cards and 400 titles
    (``discover.RELEASES_MAX`` / ``RELEASE_TRACKS_MAX``). A page that lags
    "because of the number of items" has to be measured above that cap, so the
    bench lifts BOTH **in its own process only** (the constants are read at
    request time, and ``_user_releases``'s default ``limit`` was bound when the
    module loaded) and seeds the rows a much bigger watcher would have found.
    The server contract itself is untouched: a shipped server still answers 120.
    """
    from neezer.db import Album, Artist, ArtistRelease, StarredArtist
    from neezer.webui import discover as discover_module

    r = random.Random(11)
    with app.app_context():
        discover_module.RELEASES_MAX = count
        discover_module.RELEASE_TRACKS_MAX = count
        discover_module._user_releases.__defaults__ = (count, False)
        artists = list(
            Artist.select()
            .join(StarredArtist, on=(StarredArtist.starred == Artist.id))
            .where(Artist.deezer_id.is_null(False))
            .distinct()
            .limit(60)
        )
        if not artists:
            raise SystemExit("--releases needs 'bench' to follow some artists")
        # The cards' sleeves: the library's own albums (with `--covers` they all
        # carry the real JPEG), so a client that asks for a few thousand release
        # covers is measurable instead of painting placeholders. Deezer is off,
        # and a release ref has no archived file.
        sleeves = [f"/api/cover/{album.id}" for album in Album.select().limit(40)]
        today = datetime.date.today()
        rows = []
        for k in range(count):
            artist = artists[k % len(artists)]
            hits = [
                {
                    "r": str(7_000_000 + k * 4 + j),
                    "t": title(r),
                    "d": r.randint(120, 300),
                    "e": False,
                    "a": [[artist.deezer_id, artist.name]],
                }
                for j in range(1 + k % 3)
            ]
            rows.append(
                {
                    "provider": "deezer",
                    "ref": str(6_000_000 + k),
                    "artist_ref": artist.deezer_id,
                    "artist_name": artist.name,
                    "title": f"{title(r)} {k}",
                    "kind": "album" if len(hits) > 1 else "single",
                    # Inside the 30-day window `_user_releases` asks for.
                    "released": (today - datetime.timedelta(days=k % 30)).isoformat(),
                    "nb_tracks": len(hits),
                    "tracks": json.dumps(hits),
                    "cover": sleeves[k % len(sleeves)] if sleeves else None,
                    "found": datetime.datetime.now(),
                }
            )
        for start in range(0, len(rows), 500):
            ArtistRelease.insert_many(rows[start : start + 500]).execute()
    print(f"  {count} releases for the feed", file=sys.stderr)
    return count


def seed_covers(app, jpeg_path):
    """Give every album a REAL ``cover.jpg``, so the client's sleeve path is measurable.

    The bench's rows carry a Deezer ``ALB_PICTURE`` but no file, and Deezer is
    off: ``/api/cover/<album>`` therefore answers 404 and every screen draws its
    placeholder. A client's decoded-image cache, its atlas tiles and its
    eviction can only be measured against real bytes, so this writes one real
    JPEG as every album's folder cover (``library.save_album_cover``, the same
    path the archiver uses) and reports how many albums got one.
    """
    from neezer.db import Album, Folder
    from neezer.deezer import library

    with open(jpeg_path, "rb") as fh:
        data = fh.read()
    written = 0
    with app.app_context():
        for album in Album.select():
            first = album.tracks.first()
            if first is None or first.folder is None:
                continue
            folder = Folder[first.folder]
            if library.save_album_cover(folder, data):
                written += 1
    print(f"  {written} albums carry a real cover.jpg ({len(data)} B)", file=sys.stderr)
    return written


def main():
    stale = sweep_orphans()
    if stale:
        print(
            f"  swept {len(stale)} orphaned library(ies) left by a killed bench "
            f"in {tempfile.gettempdir()}",
            file=sys.stderr,
        )
    ap = argparse.ArgumentParser()
    ap.add_argument("--tracks", type=int, default=8000)
    ap.add_argument("--playlists", type=int, default=60)
    ap.add_argument("--favorites", type=int, default=4000)
    ap.add_argument("--big-playlist", type=int, default=4000)
    ap.add_argument("--runs", type=int, default=5)
    ap.add_argument("--serve", action="store_true", help="keep serving on a port afterwards")
    ap.add_argument("--port", type=int, default=0)
    ap.add_argument(
        "--covers",
        metavar="JPEG",
        help="give every album this real JPEG as its cover.jpg, so the client's "
        "decoded-sleeve cache and GPU atlas are measured against real bytes "
        "instead of placeholders (the bench otherwise 404s every /api/cover)",
    )
    ap.add_argument(
        "--audio",
        help="a WAV file archived as the first 12 tracks (ids[\"ids\"]), so /api/stream "
        "serves them from disk with real byte ranges",
    )
    ap.add_argument(
        "--archived",
        action="store_true",
        help="mark the whole library archived, played, measured and given vectors, "
        "time the neighbours' rebuild, and ask the radio / flow / home routes too",
    )
    ap.add_argument(
        "--recap",
        action="store_true",
        help="seed the Récap's own listening log (two years for 'bench', a thin and a "
        "skipped-only year for 'thin', no history for 'fresh'), then measure "
        "/api/recap/years and /api/recap/<year>: cold, warm, 304, statements, bytes, "
        "gzip, and check the exact totals the plan pins",
    )
    ap.add_argument(
        "--dist",
        help="serve the SPA from this directory instead of neezer/webui/dist "
        "(a vite build --outDir elsewhere), for a browser bench",
    )
    ap.add_argument(
        "--onboarding",
        action="store_true",
        help="seed invitation tokens (and, with tracks, a searchable artist plus "
        "two cached neighbours for every artist) for test/onboarding",
    )
    ap.add_argument(
        "--admin-users",
        type=int,
        default=0,
        metavar="N",
        help="seed N member accounts for Réglages → Utilisateurs, plus 'membre', "
        "'desactive_moi' and 'supprime_moi'",
    )
    ap.add_argument(
        "--admin-invites",
        type=int,
        default=0,
        metavar="M",
        help="seed M pending invitations for Réglages → Utilisateurs",
    )
    ap.add_argument(
        "--podcasts",
        action="store_true",
        help="seed a show with chapters, a ready transcript, a running and a "
        "failed one, and expose the transcription engine as external",
    )
    ap.add_argument(
        "--seed-lyrics",
        action="store_true",
        help="archive two tracks with a .lrc sidecar (one synced, one plain), so "
        "/api/lyrics/<id> answers the OBJECT every player reads instead of null",
    )
    ap.add_argument(
        "--seed-genre-scores",
        action="store_true",
        help="label three tracks and give four tracks a v2 embedding, so "
        "/api/genre/candidates and /api/genre/track/<id> answer a ranked list "
        "([[label, score, head, proto]]) instead of []",
    )
    ap.add_argument(
        "--seed-provider-account",
        action="store_true",
        help="link one Deezer provider account to 'bench' so /api/me/accounts "
        "lists a row and PUT /api/me/accounts/<id> can be exercised; no "
        "provider is ever called",
    )
    ap.add_argument(
        "--releases",
        type=int,
        default=0,
        metavar="N",
        help="seed N recent releases and lift the per-payload cap IN THIS BENCH "
        "PROCESS, so a client's Nouveautés page is measured on a few thousand "
        "releases; the shipped server still answers its own 120",
    )
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="nsperf-")
    # Every exit this process can see removes the library (SIGTERM included);
    # a SIGKILL leaves the owner file behind for the next launch's sweep.
    guard = WorkDir(work).install()
    db = os.path.join(work, "db.sqlite")
    cache = os.path.join(work, "cache")
    archive = os.path.join(work, "archive")
    os.makedirs(cache)
    os.makedirs(archive)

    class Config(DefaultConfig):
        def __init__(self):
            super().__init__()
            self.BASE = dict(self.BASE, database_uri="sqlite:///" + db)
            self.WEBAPP = dict(self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True)
            self.DEEZER = dict(getattr(self, "DEEZER", {}), archive_dir=archive, enabled=False)
            if args.podcasts:
                self.TRANSCRIBE = dict(
                    getattr(self, "TRANSCRIBE", {}), enabled=True, runner="external"
                )

    app = create_application(Config())
    if args.dist:
        # The SPA routes read the module global at request time; the version
        # route bound the constant at import, so patch both.
        import neezer.webui.spa as _spa

        _spa.DIST_DIR = os.path.abspath(args.dist)
        try:
            from neezer.webui import version as _spa_version

            _spa_version.DIST_DIR = _spa.DIST_DIR
        except Exception:  # the version route is not what a bench measures
            pass
    with app.app_context():
        UserManager.add("bench", BENCH_PASSWORD, admin=True)
        from neezer.db import db as _db

        # One transaction for the whole fill: SQLite commits one statement at a
        # time otherwise, and the fill is thousands of upserts.
        _db.execute_sql("PRAGMA synchronous=OFF")
    print(f"populating ({args.tracks} tracks)...", file=sys.stderr)
    ids = populate(app, args.tracks, args.playlists, args.favorites, args.big_playlist, recap=args.recap)
    if args.covers:
        seed_covers(app, args.covers)
    if args.audio:
        from neezer.db import Track
        from neezer.deezer import ids as dzids

        with app.app_context():
            for sng in ids["ids"]:
                t = Track[dzids.track_uuid(sng)]
                # The served type comes from the extension.
                path = os.path.splitext(t.path)[0] + ".wav"
                os.makedirs(os.path.dirname(path), exist_ok=True)
                shutil.copyfile(args.audio, path)
                Track.update(path=path).where(Track.id == t.id).execute()
    if args.archived:
        make_archived(app)
    if args.onboarding:
        print("seeding onboarding…", file=sys.stderr)
        ids["onboarding"] = seed_onboarding(app, args.tracks)
    if args.admin_users or args.admin_invites:
        print("seeding admin accounts…", file=sys.stderr)
        ids["admin"] = seed_admin(app, args.admin_users, args.admin_invites)
    if args.podcasts:
        print("seeding podcasts…", file=sys.stderr)
        ids["podcast"] = seed_podcasts(app)
    if args.seed_lyrics:
        print("seeding lyrics…", file=sys.stderr)
        ids["lyrics"] = seed_lyrics(app, ids)
    if args.seed_genre_scores:
        print("seeding genre scores…", file=sys.stderr)
        ids["genre"] = seed_genre_scores(app, ids)
    if args.seed_provider_account:
        print("seeding a provider account…", file=sys.stderr)
        ids["account"] = seed_provider_account(app)
    if args.releases:
        print(f"seeding {args.releases} releases…", file=sys.stderr)
        ids["releases"] = seed_releases(app, args.releases)
    # Each request opens its own connection (web.py), as the server does.
    from neezer.db import close_connection

    close_connection()

    client = app.test_client()
    rv = client.post("/api/login", json={"username": "bench", "password": BENCH_PASSWORD})
    assert rv.status_code == 200, rv.data

    routes = [
        ("GET", "/api/me", None),
        ("GET", "/api/releases", None),
        ("GET", "/api/me/artists", None),
        ("GET", "/api/me/albums", None),
        ("GET", "/api/recommend?d=0.30", None),
        ("GET", "/api/me/playlists", None),
        ("GET", "/api/genre/tree", None),
        ("GET", "/api/me/favorites", None),
        ("GET", "/api/me/favorite-ids", None),
        ("GET", f"/api/playlist/{ids['big']}", None),
        ("GET", f"/api/playlist/{ids['small']}", None),
        ("GET", f"/api/playlist/{ids['small']}/recommendations", None),
        ("GET", "/api/history", None),
        ("GET", "/api/moods", None),
        ("GET", f"/api/album/{ids['album']}", None),
        ("GET", f"/api/artist/{ids['artist']}", None),
        ("GET", "/api/search?q=amour", None),
        ("GET", "/api/search?q=night%20fire", None),
        ("POST", "/api/gains", {"ids": ids["ids"]}),
        ("GET", f"/api/lyrics/{ids['track']}", None),
        ("GET", f"/api/track/{ids['track']}/credits", None),
        ("GET", "/api/credits/person?name=" + ids["writer"].replace(" ", "%20"), None),
        ("GET", "/api/me/local", None),
        ("GET", "/api/podcasts", None),
        ("GET", "/api/deezer/status", None),
        ("GET", "/api/version", None),
        ("GET", "/api/storage", None),
    ]
    if args.onboarding:
        routes += [
            ("GET", "/api/onboarding", None),
            ("GET", "/api/onboarding/artists?limit=36", None),
            ("GET", "/api/onboarding/search?q=camille&limit=25", None),
        ]
    if args.admin_users or args.admin_invites:
        routes += [
            ("GET", "/api/admin/users?limit=25", None),
            ("GET", "/api/admin/users?q=cam&limit=25", None),
            ("GET", "/api/admin/invites?state=pending&limit=200", None),
        ]
    if args.podcasts:
        show = ids["podcast"]["show"]
        ready = ids["podcast"]["ready"]
        wide = ids["podcast"]["chapters100"]
        routes += [
            ("GET", f"/api/podcast/{show}", None),
            ("GET", f"/api/podcast/episode/{ready}/chapters", None),
            ("GET", f"/api/podcast/episode/{wide}/chapters", None),
            ("GET", f"/api/podcast/episode/{ready}/transcript?limit=200", None),
            ("GET", f"/api/podcast/episode/{ready}/transcript?at=600&limit=200", None),
            ("GET", f"/api/podcast/episode/{ready}/transcript?q=memoire&limit=100", None),
            ("GET", f"/api/podcast/{show}/transcript/search?q=memoire&limit=50", None),
            ("GET", "/api/transcripts/search?q=memoire&limit=20", None),
            ("GET", "/api/transcribe/status", None),
            ("GET", "/api/transcribe/settings", None),
        ]
    if args.archived:
        from neezer.db import Track

        with app.app_context():
            t = Track.select().where(Track.play_count > 0).first()
            seed, artist = str(t.id), str(t.artist_id)
        routes += [
            ("GET", "/api/moods/energie?limit=40", None),
            ("GET", "/api/radio/genre/Techno", None),
            ("GET", f"/api/radio/track/{seed}", None),
            ("GET", f"/api/radio/artist/{artist}", None),
            ("GET", "/api/flow", None),
            # the Flow one title at a time: the first call builds the session's
            # pool (and, after a boot, the energy map), the others reuse them
            ("POST", "/api/flow/next", {"sid": "bench", "d": 0.35, "energy": 0.7, "n": 2}),
            ("POST", "/api/flow/next", {"sid": "bench", "d": 0.35, "energy": 0.7,
                                        "heard": [{"id": seed, "s": 200}], "more": [seed]}),
        ]
        close_connection()
    print(f"{'route':42s} {'median':>9s} {'min':>8s} {'sql':>5s} {'bytes':>9s} {'gzip':>8s}  status")
    for method, url, body in routes:
        times = []
        stmts = None
        status = None
        size = gz = 0
        for k in range(args.runs):
            with count_statements() as counts:
                t0 = time.perf_counter()
                rv = client.open(url, method=method, json=body, headers={"Accept-Encoding": "identity"})
                data = rv.get_data()
                times.append((time.perf_counter() - t0) * 1000)
            if stmts is None:
                stmts = sum(counts.values())
                status = rv.status_code
                size = len(data)
                gz = len(gzip.compress(data, 6))
        print(
            f"{method + ' ' + url[:36]:42s} {statistics.median(times):8.1f}ms {min(times):7.1f}ms "
            f"{stmts:5d} {size:9d} {gz:8d}  {status}"
        )

    if args.recap:
        import gzip as _gzip

        from neezer import recap as recap_mod
        from neezer.db import close_connection as _close
        from neezer.db import now as _now

        # Each probe opens its own connection, as the server does.
        _close()
        info = ids["recap"]
        current, past, empty = info["current"], info["past"], info["empty"]
        today = _now().date().isoformat()
        failures = []

        def ok(cond, what):
            print(f"  {'ok  ' if cond else 'FAIL'} {what}")
            if not cond:
                failures.append(what)

        def probe(url, headers=None):
            with count_statements() as counts:
                t0 = time.perf_counter()
                rv = client.get(
                    url, headers={"Accept-Encoding": "identity", **(headers or {})}
                )
                body = rv.get_data()
                ms = (time.perf_counter() - t0) * 1000
            return rv, body, ms, sum(counts.values())

        urls = (
            "/api/recap/years",
            f"/api/recap/{current}",
            f"/api/recap/{past}",
            f"/api/recap/{empty}",
        )
        measured = {}
        print(
            f"\nrecap seed: {past} 25 000 events, {current} 12 000 events, "
            f"{empty} none; 300 played titles, 270 measured\n"
        )
        print(
            f"{'route':26s} {'cold':>9s} {'warm':>9s} {'sql c/w':>9s} {'bytes':>8s} "
            f"{'gzip':>7s}  status/304"
        )
        for url in urls:
            recap_mod.invalidate()
            rv1, body1, cold, st1 = probe(url)
            etag = rv1.headers.get("ETag", "")
            rv2, _body2, warm, st2 = probe(url)
            rv3, body3, _ms3, st3 = probe(url, {"If-None-Match": etag})
            gz = len(_gzip.compress(body1, 6))
            measured[url] = (rv1, body1, cold, st1, warm, st2, rv3, body3, st3, gz)
            print(
                f"{url:26s} {cold:8.1f}ms {warm:8.1f}ms {st1:4d}/{st2:<4d} "
                f"{len(body1):8d} {gz:7d}  {rv1.status_code}/{rv3.status_code}"
            )

        def check_year(year, expected, url):
            rv, body, cold, st1, _warm, st2, rv304, body304, _st3, gz = measured[url]
            payload = json.loads(body)
            totals = payload["totals"]
            ok(rv.status_code == 200, f"{year}: 200")
            # The plan's 4 (daily aggregate, per-title aggregate, track payloads,
            # credits) plus the one session lookup every /api route pays —
            # login_required reads the user row. The tests pin the same 4 + 1.
            ok(st1 == 5, f"{year}: 5 statements cold = 4 + the session lookup (measured {st1})")
            ok(st2 <= 3, f"{year}: <= 3 statements warm, aggregates cached (measured {st2})")
            ok(
                rv304.status_code == 304 and not body304,
                f"{year}: conditional GET is a 304 with no body ({rv304.status_code})",
            )
            ok(rv.headers.get("Cache-Control") == "private, no-cache", f"{year}: private, no-cache")
            ok(rv.headers.get("ETag", "").startswith("W/"), f"{year}: weak ETag")
            ok(len(body) <= 12_000, f"{year}: {len(body)} bytes <= 12 000")
            ok(gz <= 4_000, f"{year}: {gz} gzip bytes <= 4 000")
            ok(cold <= 600, f"{year}: {cold:.1f} ms cold (plan <= 150, hard cap 600)")
            for key, want in expected.items():
                ok(totals[key] == want, f"{year}: totals.{key} = {totals.get(key)} (want {want})")
            ok(totals["neutrals"] == 0, f"{year}: no neutral event")
            ok(len(payload["months"]) == 12, f"{year}: 12 months")
            ok(
                sum(m["listens"] for m in payload["months"]) == expected["listens"],
                f"{year}: the 12 months sum to the year's listens",
            )
            ok(
                payload["cards"]
                == ["year", "minutes", "artists", "tracks", "albums", "genres", "streak",
                    "firstlast"],
                f"{year}: the 8 server cards, in the plan's order",
            )
            ok(payload["partial"] is (year == current), f"{year}: partial = {year == current}")
            ok(
                payload["period"]["to"] == (today if year == current else f"{year}-12-31"),
                f"{year}: period.to = {payload['period']['to']}",
            )
            ok(
                totals["days_active"] == (316 if year == current else 365),
                f"{year}: days_active = {totals['days_active']}",
            )
            ok(
                payload["streak"]["days"] == (316 if year == current else 365),
                f"{year}: streak = {payload['streak']['days']} days",
            )
            for kind in ("tracks", "artists", "albums"):
                rows = payload["tops"][kind]
                ok(0 < len(rows) <= 5, f"{year}: tops.{kind} has {len(rows)} rows (1..5)")
                ok(
                    all(a["listens"] >= b["listens"] for a, b in zip(rows, rows[1:])),
                    f"{year}: tops.{kind} descends by listens",
                )
                ids_ = [row["track"]["id"] if kind == "tracks" else row["id"] for row in rows]
                ok(len(set(ids_)) == len(ids_), f"{year}: tops.{kind} has no duplicate")
            genres = payload["tops"]["genres"]
            coverage = genres["measured_listens"] / totals["listens"] if totals["listens"] else 0
            ok(genres["available"] is True, f"{year}: styles card available")
            if args.archived:
                ok(coverage > 0.99, f"{year}: coverage {coverage:.4f} (--archived measured every title)")
            else:
                ok(0.89 <= coverage <= 0.91, f"{year}: coverage {coverage:.4f} in [0.89, 0.91]")
            ok(0 < len(genres["rows"]) <= 5, f"{year}: at most 5 style rows")
            ok(
                sum(row["listens"] for row in genres["rows"]) <= genres["measured_listens"],
                f"{year}: style rows never exceed the measured listens",
            )
            ok(
                payload["first"] is not None
                and payload["last"] is not None
                and payload["first"]["at"] < payload["last"]["at"],
                f"{year}: first before last",
            )

        # The exact totals of the seed, written as literals (never derived from
        # the seeding code): 25 000 and 12 000 events of 187 s, one skip in ten.
        past_expected = {"events": 25_000, "listens": 22_500, "skips": 2_500,
                         "seconds": 4_675_000, "minutes": 77_916}
        current_expected = {"events": 12_000, "listens": 10_800, "skips": 1_200,
                            "seconds": 2_244_000, "minutes": 37_400}
        check_year(past, past_expected, f"/api/recap/{past}")
        check_year(current, current_expected, f"/api/recap/{current}")

        rv, body, cold, st1, warm, st2, rv304, body304, st3, gz = measured["/api/recap/years"]
        years = json.loads(body)["years"]
        ok(rv.status_code == 200, "years: 200")
        ok(st1 == 2, f"years: 2 statements cold = 1 + the session lookup (measured {st1})")
        ok(st2 == 1, f"years: 1 statement warm, the session lookup alone (measured {st2})")
        ok(rv304.status_code == 304 and not body304, f"years: conditional GET is a 304 ({rv304.status_code})")
        ok(len(body) <= 1_000, f"years: {len(body)} bytes <= 1 000")
        ok(gz <= 400, f"years: {gz} gzip bytes <= 400")
        ok(cold <= 100, f"years: {cold:.1f} ms cold (plan <= 25, hard cap 100)")
        ok([row["year"] for row in years] == [current, past], f"years: exactly [{current}, {past}]")
        ok(all(row["events"] > 0 for row in years), "years: only years that hold an event")
        ok(empty not in [row["year"] for row in years], f"years: no {empty}")
        rows = json.loads(body)
        ok(rows["latest"] == current and rows["now_year"] == current, f"years: latest/now_year = {current}")
        ok(rows["min_year"] == 2000, "years: min_year = 2000")
        by_year = {row["year"]: row for row in years}
        ok(
            by_year[current]["events"] == 12_000 and by_year[current]["minutes"] == 37_400,
            "years: the running year's row carries its real totals",
        )
        ok(
            by_year[past]["events"] == 25_000 and by_year[past]["minutes"] == 77_916,
            "years: the finished year's row carries its real totals",
        )
        ok(by_year[current]["partial"] is True and by_year[past]["partial"] is False,
           "years: only the running year is partial")

        rv, body, cold, st1, warm, st2, rv304, body304, st3, gz = measured[
            f"/api/recap/{empty}"
        ]
        empty_payload = json.loads(body)
        ok(rv.status_code == 200, f"empty {empty}: 200, not an error")
        ok(st1 == 2, f"empty {empty}: 2 statements = 1 + the session lookup (measured {st1})")
        ok(st2 == 1, f"empty {empty}: 1 statement warm, the session lookup alone (measured {st2})")
        ok(rv304.status_code == 304 and not body304, f"empty {empty}: conditional GET is a 304")
        # 1 040 bytes measured with the always-12-month shape the client reads:
        # the plan's 700 was an estimate, the honest bound is the shape's size.
        ok(len(body) <= 1_200, f"empty {empty}: {len(body)} bytes <= 1 200 (plan said 700)")
        ok(gz <= 500, f"empty {empty}: {gz} gzip bytes <= 500 (plan said 300)")
        ok(cold <= 40, f"empty {empty}: {cold:.1f} ms cold (plan <= 10, hard cap 40)")
        ok(empty_payload["totals"]["events"] == 0, f"empty {empty}: events = 0")
        ok(empty_payload["cards"] == [], f"empty {empty}: no card")
        ok(empty_payload["enough"] is False, f"empty {empty}: not enough")
        ok(empty_payload["streak"] is None and empty_payload["first"] is None,
           f"empty {empty}: streak and first are null")
        ok(empty_payload["tops"]["tracks"] == [], f"empty {empty}: no top")
        ok(
            len(empty_payload["months"]) == 12
            and all(m["listens"] == 0 and m["seconds"] == 0 for m in empty_payload["months"]),
            f"empty {empty}: 12 zero months",
        )

        if failures:
            print(f"\nFAIL: {len(failures)} recap check(s) failed")
            for what in failures:
                print(f"  - {what}")
            sys.exit(1)
        print("\nrecap: all checks passed")

    if args.serve:
        from werkzeug.serving import make_server

        server = make_server("127.0.0.1", args.port, app, threaded=True)
        print("READY", server.server_port, json.dumps(ids), flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
    guard.release()


if __name__ == "__main__":
    main()
