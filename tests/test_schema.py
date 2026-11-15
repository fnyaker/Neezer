# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Guards for the two ways schema work goes wrong silently.

Both of these shipped a 500 to production that the rest of the suite could not
see, because the suite runs on SQLite and the deployment runs on Postgres.
"""

import os
import os.path
import re
import tempfile
import unittest

from datetime import datetime, timedelta

from peewee import IntegrityError, SqliteDatabase

from neezer import db as dbmod
from neezer.db import (
    SCHEMA_VERSION,
    Playlist,
    PlaylistFolder,
    PlaylistMember,
    PlaylistPlacement,
    PodcastChannel,
    PodcastChapter,
    PodcastChapterScan,
    PodcastTranscript,
    PodcastTranscriptLease,
    PodcastTranscriptSegment,
    TranscribeWorker,
    Track,
    db,
    init_database,
    release_database,
)

MIGRATION_ROOT = os.path.join(os.path.dirname(dbmod.__file__), "schema", "migration")
PROVIDERS = ("sqlite", "postgres", "mysql")
# Where this fork's own migrations start. Everything older is upstream history,
# which legitimately includes engine-specific migrations.
FORK_EPOCH = "20260605"


def _dates(provider):
    """The migration dates shipped for one engine."""
    folder = os.path.join(MIGRATION_ROOT, provider)
    out = set()
    for name in os.listdir(folder):
        if name[0] in ("_", "."):
            continue
        date, ext = os.path.splitext(name)
        if ext in (".sql", ".py"):
            out.add(date)
    return out


class SchemaVersionTestCase(unittest.TestCase):
    def test_every_engine_ships_this_forks_migrations(self):
        """A migration added for one engine and forgotten for another leaves
        that engine's databases missing a column — which surfaces as a 500 on
        every query touching the table, and only for the people running it.

        Scoped to this fork's own migrations: upstream legitimately ships
        engine-SPECIFIC ones (binary-id conversions for MySQL/SQLite, a
        Postgres-only fix), and those are history we don't police.
        """
        ours = {p: {d for d in _dates(p) if d >= FORK_EPOCH} for p in PROVIDERS}
        # 20260614 is a Postgres-only repair (a citext/collation fix); it has no
        # SQLite or MySQL counterpart by design.
        for p in ("postgres", "mysql"):
            ours[p].discard("20260614")
        for provider in PROVIDERS[1:]:
            self.assertEqual(
                ours[provider],
                ours["sqlite"],
                f"{provider} is missing (or has extra) migrations vs sqlite",
            )

    def test_schema_version_matches_the_latest_migration(self):
        """SCHEMA_VERSION is what decides which migrations still have to run. If
        it lags behind the newest file, that migration never runs; if it leads,
        the version is recorded as done before the work exists."""
        self.assertEqual(SCHEMA_VERSION, max(_dates("sqlite")))

    def test_the_four_data_slices_own_four_consecutive_dates(self):
        """The plans initially gave the three slices the same dates; the merge
        resolved them to 20261114..20261117, one slice per date, on every
        engine. A duplicate date would silently skip one slice's migration."""
        self.assertGreaterEqual(SCHEMA_VERSION, "20261117")
        for date in ("20261114", "20261115", "20261116", "20261117"):
            for provider in PROVIDERS:
                folder = os.path.join(MIGRATION_ROOT, provider)
                self.assertIn(
                    date,
                    {
                        os.path.splitext(n)[0]
                        for n in os.listdir(folder)
                        if os.path.splitext(n)[1] in (".sql", ".py")
                    },
                    f"{provider} has no {date} migration",
                )

    def test_every_model_table_is_in_every_base_schema(self):
        """A table added to the models but not to schema/<engine>.sql gives a
        fresh install a model it cannot query — and a fresh install never runs
        migrations, so no upgrade test would see it."""
        for provider in PROVIDERS:
            path = os.path.join(
                os.path.dirname(dbmod.__file__), "schema", f"{provider}.sql"
            )
            with open(path, encoding="utf-8") as fh:
                sql = fh.read()
            for model in dbmod._migration_order():
                table = model._meta.table_name
                self.assertRegex(
                    sql,
                    r'CREATE TABLE (?:IF NOT EXISTS )?"?%s"?\s*\(' % re.escape(table),
                    f"{table} missing from {provider}.sql",
                )

    def test_model_columns_exist_in_the_base_schema(self):
        """A field added to a model but not to schema/<engine>.sql gives a fresh
        install a table it cannot query."""
        for provider in PROVIDERS:
            path = os.path.join(
                os.path.dirname(dbmod.__file__), "schema", f"{provider}.sql"
            )
            with open(path, encoding="utf-8") as fh:
                sql = fh.read()
            for model in (
                Track,
                PodcastChannel,
                Playlist,
                PlaylistMember,
                PlaylistFolder,
                PlaylistPlacement,
                PodcastChapter,
                PodcastChapterScan,
                PodcastTranscript,
                PodcastTranscriptSegment,
                TranscribeWorker,
                PodcastTranscriptLease,
            ):
                table = model._meta.table_name
                m = re.search(
                    r"CREATE TABLE IF NOT EXISTS %s \((.*?)\n\)" % table, sql, re.S
                )
                self.assertIsNotNone(m, f"{table} missing from {provider}.sql")
                body = m.group(1)
                for field in model._meta.sorted_fields:
                    column = field.column_name
                    self.assertRegex(
                        body,
                        r"\b%s\b" % re.escape(column),
                        f"{table}.{column} missing from {provider}.sql",
                    )


class ScriptSplitTestCase(unittest.TestCase):
    """A schema script is cut into statements on ";" — and a ";" inside a comment
    used to cut the comment instead, running its second half as SQL. The
    20260730 migration has exactly such a comment, so every SQLite database
    still upgrading through it failed its boot on a syntax error."""

    SQL_START = re.compile(
        r"^(CREATE|ALTER|UPDATE|INSERT|DELETE|DROP|PRAGMA|BEGIN|COMMIT|SET|"
        r"WITH|SELECT|REPLACE|START|VACUUM|REPAIR|OPTIMIZE)\b",
        re.I,
    )

    def test_a_semicolon_in_a_comment_is_part_of_the_comment(self):
        sql = (
            "ALTER TABLE t ADD COLUMN c INT;\n"
            "-- recoverable from the path; everything else stays NULL\n"
            "UPDATE t SET c = 1;\n"
        )
        self.assertEqual(
            dbmod.split_sql_script(sql),
            ["ALTER TABLE t ADD COLUMN c INT", "UPDATE t SET c = 1"],
        )

    def test_a_statement_after_a_comment_is_kept(self):
        self.assertEqual(
            dbmod.split_sql_script("-- a table\nCREATE TABLE x (a INT);"),
            ["CREATE TABLE x (a INT)"],
        )

    def test_every_shipped_script_splits_into_statements(self):
        root = os.path.join(os.path.dirname(dbmod.__file__), "schema")
        scripts = [os.path.join(root, f"{p}.sql") for p in PROVIDERS]
        for provider in PROVIDERS:
            folder = os.path.join(MIGRATION_ROOT, provider)
            scripts += [
                os.path.join(folder, n) for n in os.listdir(folder) if n.endswith(".sql")
            ]
        self.assertGreater(len(scripts), 30)
        for path in scripts:
            with open(path, encoding="utf-8") as fh:
                statements = dbmod.split_sql_script(fh.read())
            for statement in statements:
                self.assertRegex(
                    statement,
                    self.SQL_START,
                    f"{os.path.relpath(path, root)}: not a statement: {statement[:60]!r}",
                )


class QueryShapeTestCase(unittest.TestCase):
    """Python binds `&` tighter than `>`, so

        Track.deezer_id.is_null(False) & Track.last_modification > 0

    builds ``(deezer_id IS NOT NULL AND last_modification) > 0``. SQLite
    evaluates that happily and Postgres rejects it outright, so the mistake
    passes every test here and 500s in production. These assert the SHAPE of the
    SQL, which is engine-independent.
    """

    @classmethod
    def setUpClass(cls):
        if db.obj is None:
            db.initialize(SqliteDatabase(":memory:"))

    def _where(self, expr):
        return Track.select().where(expr).sql()[0].split("WHERE", 1)[1]

    def test_a_comparison_combined_with_and_must_be_parenthesized(self):
        unparenthesized = self._where(
            Track.deezer_id.is_null(False) & Track.last_modification > 0
        )
        # The bug, spelled out: the comparison swallowed the whole AND.
        self.assertTrue(unparenthesized.strip().endswith("> ?)"))

        correct = self._where(
            Track.deezer_id.is_null(False) & (Track.last_modification > 0)
        )
        self.assertIn('("t1"."last_modification" > ?)', correct)

    def test_the_archived_count_query_is_well_formed(self):
        """The actual expression /api/storage runs."""
        where = self._where(
            Track.deezer_id.is_null(False) & (Track.last_modification > 0)
        )
        self.assertIn('("t1"."deezer_id" IS NOT NULL)', where)
        self.assertIn('("t1"."last_modification" > ?)', where)


class MigrationRepairTestCase(unittest.TestCase):
    """The exact failure this repair exists for.

    ``podcast_channel.subscribed`` was added by appending to an ALREADY-RELEASED
    migration file. The runner skips any migration dated at or before the
    recorded schema version, so servers that had applied 20260806 never got the
    column — and every query touching the table 500'd, because peewee selects all
    of a model's fields.

    This reconstructs such a database and checks that booting the current code
    repairs it.
    """

    def setUp(self):
        self.fd, self.path = tempfile.mkstemp(suffix=".db")
        self.uri = "sqlite:///" + self.path

    def tearDown(self):
        release_database()
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def test_a_database_missing_the_column_gets_it_back(self):
        import sqlite3

        # A current database…
        init_database(self.uri)
        release_database()

        # …rewound to the state the edit left real servers in: 20260807 applied
        # (so `gone` is there) but `subscribed` never created, because the file
        # that would have created it was already marked done.
        conn = sqlite3.connect(self.path)
        conn.execute("ALTER TABLE podcast_channel DROP COLUMN subscribed")
        conn.execute("UPDATE meta SET value = '20260807' WHERE key = 'schema_version'")
        conn.commit()
        cols = {r[1] for r in conn.execute("PRAGMA table_info(podcast_channel)")}
        conn.close()
        self.assertNotIn("subscribed", cols)  # the broken state, reproduced

        # Booting the current code must repair it, not crash on it.
        init_database(self.uri)
        self.assertEqual(PodcastChannel.select().count(), 0)  # the query that 500'd

        conn = sqlite3.connect(self.path)
        cols = {r[1] for r in conn.execute("PRAGMA table_info(podcast_channel)")}
        version = conn.execute(
            "SELECT value FROM meta WHERE key = 'schema_version'"
        ).fetchone()[0]
        conn.close()
        self.assertIn("subscribed", cols)
        self.assertIn("gone", cols)
        self.assertEqual(version, SCHEMA_VERSION)

    def test_the_repair_is_idempotent(self):
        """It also runs on databases that already have the column (a fresh
        install, or one that applied 20260806 after the edit): it must do
        nothing rather than fail the boot."""
        import importlib

        _mod = importlib.import_module("neezer.schema.migration.sqlite.20260808")

        init_database(self.uri)
        release_database()
        # Applying it twice on a database that already has the column.
        for _ in range(2):
            _mod.apply({"database": self.path})
        init_database(self.uri)
        self.assertEqual(PodcastChannel.select().count(), 0)


class ShowArchiveSwitchUpgradeTestCase(unittest.TestCase):
    """20261029: each show's switch now decides what the archive keeps. Before
    it, every show was archived whole, so a server that upgrades must find all
    its shows switched ON — not silently stop archiving their new episodes."""

    def setUp(self):
        self.fd, self.path = tempfile.mkstemp(suffix=".db")
        self.uri = "sqlite:///" + self.path

    def tearDown(self):
        release_database()
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def test_an_upgraded_server_keeps_archiving_every_show(self):
        import sqlite3

        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        for n in ("1", "2"):
            # As 20261028 left them: the column there, every show at its default.
            PodcastChannel.create(user=alice, url=f"u{n}", title=f"Show {n}", status="completed")
        release_database()
        conn = sqlite3.connect(self.path)
        conn.execute("UPDATE meta SET value = '20261028' WHERE key = 'schema_version'")
        conn.commit()
        conn.close()

        init_database(self.uri)

        conn = sqlite3.connect(self.path)
        switched = [r[0] for r in conn.execute("SELECT auto_archive FROM podcast_channel")]
        version = conn.execute(
            "SELECT value FROM meta WHERE key = 'schema_version'"
        ).fetchone()[0]
        conn.close()
        self.assertEqual(switched, [1, 1])
        self.assertEqual(version, SCHEMA_VERSION)


class SkippedMigrationRepairTestCase(unittest.TestCase):
    """A database a build from another branch moved past a migration merged
    later is marked up to date without it: the stored version alone never runs
    it. This happened to ``provider_account.options`` (lot P2): every visit to
    Mes comptes answered 500 while playback, which never reads it, worked."""

    def setUp(self):
        self.fd, self.path = tempfile.mkstemp(suffix=".db")
        self.uri = "sqlite:///" + self.path

    def tearDown(self):
        release_database()
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def _sql(self, *statements):
        import sqlite3

        conn = sqlite3.connect(self.path)
        out = [conn.execute(s).fetchall() for s in statements]
        conn.commit()
        conn.close()
        return out

    def test_a_skipped_column_and_table_are_put_back(self):
        from neezer.db import ProviderAccount

        init_database(self.uri)
        release_database()
        # The broken state: the version says current, 20261027 never ran.
        self._sql(
            "ALTER TABLE provider_account DROP COLUMN options",
            "DROP TABLE library_item",
        )
        self.assertEqual(
            self._sql("SELECT value FROM meta WHERE key = 'schema_version'")[0][0][0],
            SCHEMA_VERSION,
        )

        init_database(self.uri)
        self.assertEqual(list(ProviderAccount.select()), [])  # the query that 500'd
        self.assertEqual(dbmod._missing_columns(), [])

    def test_the_data_migrations_are_not_replayed(self):
        """20261029 switches every show on. Replayed on a server that only
        missed another migration, it would undo each show a person switched off."""
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        PodcastChannel.create(
            user=alice, url="u", title="Show", status="completed", auto_archive=False
        )
        release_database()
        self._sql("ALTER TABLE provider_account DROP COLUMN options")

        init_database(self.uri)
        self.assertEqual([c.auto_archive for c in PodcastChannel.select()], [False])
        self.assertEqual(dbmod._missing_columns(), [])

    def test_a_skipped_data_migration_runs_with_its_column(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        PodcastChannel.create(user=alice, url="u", title="Show", status="completed")
        release_database()
        self._sql("ALTER TABLE podcast_channel DROP COLUMN auto_archive")

        init_database(self.uri)
        # As an ordinary upgrade through 20261028 then 20261029 leaves it.
        self.assertEqual([c.auto_archive for c in PodcastChannel.select()], [True])

    def test_a_complete_database_is_left_alone(self):
        init_database(self.uri)
        with self.assertNoLogs("neezer.db", level="WARNING"):
            release_database()
            init_database(self.uri)


class _TempDatabaseTestCase(unittest.TestCase):
    """A throwaway SQLite file per test, and a raw handle to rewind it."""

    def setUp(self):
        self.fd, self.path = tempfile.mkstemp(suffix=".db")
        self.uri = "sqlite:///" + self.path

    def tearDown(self):
        release_database()
        os.close(self.fd)
        try:
            os.remove(self.path)
        except OSError:
            pass

    def _sql(self, *statements):
        import sqlite3

        conn = sqlite3.connect(self.path)
        out = [conn.execute(s).fetchall() for s in statements]
        conn.commit()
        conn.close()
        return out

    def _rewind(self, version):
        self._sql(f"UPDATE meta SET value = '{version}' WHERE key = 'schema_version'")


class NewMigrationsReplayTestCase(_TempDatabaseTestCase):
    """20261114..20261117 are the four data slices of this lot. Every one of them
    is replayed by ``_repair_skipped_migrations`` whenever a table or column they
    created is missing — even on a current database — so each must run twice
    without an error and without duplicating a row."""

    DATES = ("20261114", "20261116", "20261117")

    def test_a_fresh_install_can_query_every_new_model(self):
        init_database(self.uri)
        for model in (
            Playlist,
            PlaylistMember,
            PlaylistFolder,
            PlaylistPlacement,
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
        ):
            self.assertEqual(model.select().count(), 0)
        self.assertEqual(dbmod._missing_columns(), [])

    def _replay_sql(self, date):
        init_database(self.uri)
        with db.atomic():
            dbmod.execute_sql_resource_script(f"schema/migration/sqlite/{date}.sql")
        release_database()

    def _replay_py(self, date):
        import importlib

        init_database(self.uri)
        module = importlib.import_module(f"neezer.schema.migration.sqlite.{date}")
        module.apply({"database": self.path})
        release_database()

    def test_the_four_migrations_run_twice(self):
        init_database(self.uri)
        release_database()
        # Reproduce the pre-migration shape, then run each file twice: this is
        # what _repair_skipped_migrations does on a database that skipped one.
        self._sql(
            "DROP TABLE playlist_member",
            "DROP TABLE playlist_placement",
            "DROP TABLE playlist_folder",
            "DROP TABLE podcast_transcript_segment",
            "DROP TABLE podcast_transcript",
            "DROP TABLE podcast_chapter_scan",
            "DROP TABLE podcast_chapter",
            "ALTER TABLE playlist DROP COLUMN version",
            "ALTER TABLE playlist DROP COLUMN members_version",
            "ALTER TABLE playlist DROP COLUMN last_edited_by",
            "ALTER TABLE playlist DROP COLUMN last_edited_at",
        )
        for _ in range(2):
            for date in self.DATES:
                self._replay_sql(date)
            self._replay_py("20261115")
        init_database(self.uri)
        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(PlaylistMember.select().count(), 0)
        self.assertEqual(
            [c.name for c in db.get_columns("playlist")][-4:],
            ["version", "members_version", "last_edited_by", "last_edited_at"],
        )

    def test_the_four_tables_are_put_back_after_a_skipped_migration(self):
        """The shape is checked at boot: dropping what a migration created must
        not leave a model the next query 500s on."""
        init_database(self.uri)
        release_database()
        self._sql(
            "DROP TABLE playlist_placement",
            "DROP TABLE playlist_folder",
            "DROP TABLE playlist_member",
            "DROP TABLE podcast_transcript_segment",
            "DROP TABLE podcast_transcript",
            "DROP TABLE podcast_chapter_scan",
            "DROP TABLE podcast_chapter",
        )
        init_database(self.uri)
        self.assertEqual(dbmod._missing_columns(), [])
        for model in (
            PlaylistFolder,
            PlaylistPlacement,
            PlaylistMember,
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
        ):
            self.assertEqual(model.select().count(), 0)


class PlaylistFoldersUpgradeTestCase(_TempDatabaseTestCase):
    """20261114: two new tables and no backfill. An upgraded server's playlist
    list must be byte-identical, and a placement must follow its playlist or its
    folder when either is deleted."""

    def test_an_upgraded_server_keeps_its_playlists_and_gets_the_tables(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        start = datetime(2026, 1, 1)
        for n in range(60):
            Playlist.create(
                user=alice, name=f"List {n:02}", created=start + timedelta(minutes=n)
            )
        before = [
            (p.id, p.name, p.created)
            for p in Playlist.select().order_by(Playlist.created, Playlist.id)
        ]
        release_database()
        self._sql(
            "DROP TABLE playlist_placement",
            "DROP TABLE playlist_folder",
        )
        self._rewind("20261113")

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(PlaylistFolder.select().count(), 0)
        self.assertEqual(PlaylistPlacement.select().count(), 0)  # no backfill
        self.assertEqual(
            [
                (p.id, p.name, p.created)
                for p in Playlist.select().order_by(Playlist.created, Playlist.id)
            ],
            before,
        )
        self.assertEqual(
            dbmod.Meta["schema_version"].value, SCHEMA_VERSION
        )

    def test_a_placement_follows_its_playlist_and_its_folder(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=0)
        filed = Playlist.create(user=alice, name="Filed")
        unfiled = Playlist.create(user=alice, name="Unfiled")
        PlaylistPlacement.create(user=alice, playlist=filed, folder=folder, position=0)
        PlaylistPlacement.create(user=alice, playlist=unfiled, position=0)

        folder.delete_instance()
        # The folder's placements go; the playlists themselves never do.
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        self.assertEqual(Playlist.select().count(), 2)

        PlaylistPlacement.create(user=alice, playlist=filed, position=0)
        filed.delete_instance()
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        self.assertEqual(Playlist.select().count(), 1)


class PlaylistCollaborationUpgradeTestCase(_TempDatabaseTestCase):
    """20261115: a version pair, a last-edited snapshot and one owner row per
    playlist. The upgrade must not create any non-owner membership, and replay
    (the repair path) must neither duplicate nor remove a row."""

    def _tracks(self, count):
        from neezer.db import Album, Artist, Folder, Track

        root = Folder.create(root=True, name="root", path="/music")
        sub = Folder.create(root=False, name="sub", path="/music/sub", parent=root)
        artist = Artist.create(name="Artist")
        album = Album.create(artist=artist, name="Album")
        return [
            Track.create(
                title=f"Track {n}",
                album=album,
                artist=artist,
                disc=1,
                number=n + 1,
                duration=180,
                bitrate=320,
                path=f"/music/sub/{n}.flac",
                last_modification=0,
                root_folder=root,
                folder=sub,
            )
            for n in range(count)
        ]

    def test_an_upgraded_server_gets_one_owner_row_per_playlist(self):
        from neezer.db import PlaylistTrack, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0b", admin=False)
        alice = User.get(User.name == "alice")
        bob = User.get(User.name == "bob")
        tracks = self._tracks(3)
        alice_playlist = Playlist.create(user=alice, name="Alice mix")
        deezer_playlist = Playlist.create(
            user=bob, name="Bob mix", deezer_id="1234567"
        )
        for track in tracks:
            alice_playlist.add(track)
        playlists_before = [
            (p.id, p.user_id, p.name, p.created, p.deezer_id)
            for p in Playlist.select().order_by(Playlist.created, Playlist.id)
        ]
        tracks_before = [
            (t.playlist_id, t.track_id, t.index)
            for t in PlaylistTrack.select().order_by(
                PlaylistTrack.playlist, PlaylistTrack.index
            )
        ]
        release_database()
        self._sql(
            "DROP TABLE playlist_member",
            "ALTER TABLE playlist DROP COLUMN version",
            "ALTER TABLE playlist DROP COLUMN members_version",
            "ALTER TABLE playlist DROP COLUMN last_edited_by",
            "ALTER TABLE playlist DROP COLUMN last_edited_at",
        )
        self._rewind("20261113")

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        rows = list(PlaylistMember.select().order_by(PlaylistMember.added))
        self.assertEqual(len(rows), 2)  # one per playlist, no non-owner row
        for row in rows:
            self.assertEqual(row.role, "owner")
            self.assertEqual(row.user_id, row.playlist.user_id)
            self.assertEqual(row.added, row.playlist.created)
        self.assertTrue(
            all(
                p.version == 1 and p.members_version == 1
                for p in Playlist.select()
            )
        )
        self.assertEqual(
            [
                (p.id, p.user_id, p.name, p.created, p.deezer_id)
                for p in Playlist.select().order_by(Playlist.created, Playlist.id)
            ],
            playlists_before,
        )
        self.assertEqual(
            [
                (t.playlist_id, t.track_id, t.index)
                for t in PlaylistTrack.select().order_by(
                    PlaylistTrack.playlist, PlaylistTrack.index
                )
            ],
            tracks_before,
        )
        self.assertEqual(
            [p.id for p in Playlist.select().where(Playlist.user == bob)],
            [deezer_playlist.id],
        )

        # A second boot replays nothing and duplicates nothing.
        release_database()
        init_database(self.uri)
        self.assertEqual(PlaylistMember.select().count(), 2)

    def test_the_repair_path_recreates_the_missing_owner_rows(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        playlist = Playlist.create(user=alice, name="Mix")
        release_database()
        # The version says current, but 20261115 never ran: the exact state a
        # database moved past by a build from another branch is left in.
        self._sql("DROP TABLE playlist_member")

        init_database(self.uri)

        rows = list(PlaylistMember.select())
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0].playlist_id, rows[0].user_id), (playlist.id, alice.id))
        self.assertEqual(rows[0].role, "owner")
        self.assertEqual(dbmod._missing_columns(), [])

    def test_the_replay_touches_no_non_owner_row(self):
        """The backfill inserts missing owner rows and NOTHING else: a replay
        triggered by an unrelated missing table must not remove an editor, nor
        duplicate the owner."""
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0b", admin=False)
        alice = User.get(User.name == "alice")
        bob = User.get(User.name == "bob")
        playlist = Playlist.create(user=alice, name="Mix")
        release_database()
        self._sql("DROP TABLE playlist_folder")  # a shape hole triggers the replay
        init_database(self.uri)
        PlaylistMember.create(playlist=playlist, user=bob, role="editor")

        release_database()
        self._sql("DROP TABLE playlist_folder")
        init_database(self.uri)

        roles = {row.user_id: row.role for row in PlaylistMember.select()}
        self.assertEqual(roles, {alice.id: "owner", bob.id: "editor"})
        self.assertEqual(PlaylistMember.select().count(), 2)  # no duplicate owner


class PodcastDerivedDataUpgradeTestCase(_TempDatabaseTestCase):
    """20261116 and 20261117: chapters and transcripts are new tables, no ALTER
    on podcast_episode. The upgrade must leave archived episodes intact, and the
    derived rows must follow the episode (and, for a segment, its transcript)."""

    def _episode(self):
        from neezer.db import PodcastEpisode, User
        from neezer.managers.user import UserManager

        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        channel = PodcastChannel.create(user=alice, url="u", title="Show")
        episode = PodcastEpisode.create(channel=channel, title="Episode", duration=60)
        return channel, episode

    def test_an_upgraded_server_keeps_its_episodes_and_gets_the_tables(self):
        from neezer.db import PodcastEpisode

        init_database(self.uri)
        channel, episode = self._episode()
        release_database()
        self._sql(
            "DROP TABLE podcast_transcript_segment",
            "DROP TABLE podcast_transcript",
            "DROP TABLE podcast_chapter_scan",
            "DROP TABLE podcast_chapter",
        )
        self._rewind("20261115")

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        again = PodcastEpisode.get(PodcastEpisode.id == episode.id)
        self.assertEqual(again.title, "Episode")
        self.assertEqual(again.channel_id, channel.id)
        for model in (
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
        ):
            self.assertEqual(model.select().count(), 0)

    def test_every_derived_row_follows_its_episode(self):
        init_database(self.uri)
        _, episode = self._episode()
        for idx in range(2):
            PodcastChapter.create(
                episode=episode,
                idx=idx,
                start_ms=idx * 1000,
                end_ms=(idx + 1) * 1000,
                title=f"Chapter {idx}",
                source="id3",
            )
        PodcastChapterScan.create(
            episode=episode, scanned_at=datetime.now(), source="id3", n_chapters=2
        )
        transcript = PodcastTranscript.create(episode=episode, status="ready")
        for idx in range(2):
            PodcastTranscriptSegment.create(
                transcript=transcript,
                idx=idx,
                start_ms=idx * 1000,
                end_ms=(idx + 1) * 1000,
                text=f"Line {idx}",
                search_text=f"line {idx}",
            )

        # Two chapters cannot start at the same millisecond of one episode.
        with self.assertRaises(IntegrityError):
            PodcastChapter.create(
                episode=episode, idx=9, start_ms=1000, source="id3"
            )

        transcript.delete_instance()
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)

        again = PodcastTranscript.create(episode=episode, status="ready")
        PodcastTranscriptSegment.create(
            transcript=again,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Line",
            search_text="line",
        )
        episode.delete_instance()
        for model in (
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
        ):
            self.assertEqual(model.select().count(), 0)
        self.assertEqual(PodcastChannel.select().count(), 1)  # the show stays


class AccountOnboardingSchemaTestCase(unittest.TestCase):
    """20261118: the invitation table, the folded-name uniqueness and the
    account state columns. A fresh install never runs migrations, so the three
    base schemas must carry the same shape as the migration."""

    COLUMNS = ("name_key", "disabled", "last_login", "onboarding_step")
    INVITE_COLUMNS = (
        "id",
        "token_hash",
        "created_by",
        "created",
        "expires",
        "used_by",
        "used_at",
        "revoked_at",
        "note",
    )

    def _schema(self, provider):
        path = os.path.join(
            os.path.dirname(dbmod.__file__), "schema", f"{provider}.sql"
        )
        with open(path, encoding="utf-8") as fh:
            return fh.read()

    def _table_body(self, sql, table):
        m = re.search(
            r'CREATE TABLE(?: IF NOT EXISTS)? "?%s"? \((.*?)\n\)' % table, sql, re.S
        )
        self.assertIsNotNone(m, f"{table} missing")
        return m.group(1)

    def test_every_engine_has_the_user_columns(self):
        for provider in PROVIDERS:
            body = self._table_body(self._schema(provider), "user")
            for column in self.COLUMNS:
                self.assertRegex(
                    body,
                    r"\b%s\b" % column,
                    f"user.{column} missing from {provider}.sql",
                )
            self.assertRegex(
                self._schema(provider),
                r"index_user_name_key",
                f"the unique name index missing from {provider}.sql",
            )

    def test_every_engine_has_the_invite_table(self):
        for provider in PROVIDERS:
            body = self._table_body(self._schema(provider), "invite")
            for column in self.INVITE_COLUMNS:
                self.assertRegex(
                    body,
                    r"\b%s\b" % column,
                    f"invite.{column} missing from {provider}.sql",
                )
            self.assertNotRegex(body, r"\btoken\b", "the clear token is never stored")
            self.assertNotRegex(body, r"\bmac\b", "the token mac is never stored")

    def test_every_engine_ships_an_idempotent_migration(self):
        for provider in PROVIDERS:
            path = os.path.join(MIGRATION_ROOT, provider, "20261118.py")
            self.assertTrue(os.path.exists(path), path)
            with open(path, encoding="utf-8") as fh:
                text = fh.read()
            self.assertIn("invite", text)
            self.assertIn("name_key", text)

    def test_the_migration_fold_matches_the_application_fold(self):
        import importlib

        from neezer.managers.user import fold_name

        module = importlib.import_module("neezer.schema.migration.sqlite.20261118")
        for name in ("Élodie", "ALICE", "Jean-Luc", "  Bob ", "Álice"):
            self.assertEqual(module._fold(name), fold_name(name), name)


class AccountOnboardingUpgradeTestCase(_TempDatabaseTestCase):
    """20261118 on an existing database: accounts keep their passwords and
    names, get a folded key, and are all marked done (they never see the
    onboarding). Replaying the migration must not duplicate or lose a row."""

    def _strip_shape(self):
        self._sql(
            "DROP TABLE IF EXISTS invite",
            "DROP INDEX IF EXISTS index_user_name_key",
            "ALTER TABLE user DROP COLUMN name_key",
            "ALTER TABLE user DROP COLUMN disabled",
            "ALTER TABLE user DROP COLUMN last_login",
            "ALTER TABLE user DROP COLUMN onboarding_step",
        )

    def _apply(self):
        import importlib

        module = importlib.import_module("neezer.schema.migration.sqlite.20261118")
        module.apply({"database": self.path})

    def test_a_fresh_install_can_query_the_new_models(self):
        from neezer.db import Invite, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        self.assertEqual(Invite.select().count(), 0)
        self.assertEqual(dbmod._missing_columns(), [])
        user = UserManager.add("Élodie", "Alic3")
        self.assertEqual(user.name_key, "elodie")
        self.assertEqual(user.onboarding_step, "done")
        self.assertFalse(user.disabled)
        self.assertIsNone(user.last_login)
        self.assertEqual(User.select().count(), 1)

    def test_the_migration_runs_twice_without_duplicating(self):
        from neezer.db import Invite

        init_database(self.uri)
        release_database()
        self._strip_shape()
        for _ in range(2):
            self._apply()
        init_database(self.uri)
        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Invite.select().count(), 0)
        self.assertEqual(
            [c.name for c in db.get_columns("user")][-4:],
            ["name_key", "disabled", "last_login", "onboarding_step"],
        )

    def test_the_repair_path_puts_the_shape_back(self):
        from neezer.db import Invite, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        release_database()
        self._strip_shape()

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Invite.select().count(), 0)
        alice = User.get(User.name == "alice")
        self.assertEqual(alice.name_key, "alice")
        self.assertEqual(alice.onboarding_step, "done")
        self.assertFalse(alice.disabled)

    def test_a_missing_invite_table_alone_is_repaired(self):
        from neezer.db import Invite, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("Alice", "Alic3", admin=True)
        release_database()
        self._sql("DROP TABLE invite")

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Invite.select().count(), 0)
        alice = User.get(User.name == "Alice")
        self.assertEqual(alice.name_key, "alice")
        self.assertEqual(alice.onboarding_step, "done")

    def test_an_upgraded_database_keeps_its_accounts_and_marks_them_done(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("Alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb", mail="b@example.com")
        before = {
            u.name: (u.password, u.salt, u.password_clear, u.admin, u.mail)
            for u in User.select()
        }
        release_database()
        self._strip_shape()
        self._rewind("20261117")

        init_database(self.uri)

        users = {u.name: u for u in User.select()}
        self.assertEqual(set(users), set(before))
        for name, user in users.items():
            self.assertEqual(
                (user.password, user.salt, user.password_clear, user.admin, user.mail),
                before[name],
            )
            self.assertEqual(user.onboarding_step, "done")
            self.assertFalse(user.disabled)
            self.assertIsNone(user.last_login)
        self.assertEqual(users["Alice"].name_key, "alice")
        self.assertEqual(users["bob"].name_key, "bob")
        self.assertIsNotNone(UserManager.try_auth("Alice", "Alic3"))
        self.assertIsNotNone(UserManager.try_auth("bob", "B0bbb"))
        self.assertEqual(dbmod.Meta["schema_version"].value, SCHEMA_VERSION)

    def test_historical_case_duplicates_keep_one_key_and_their_logins(self):
        from neezer.db import User
        from neezer.managers.user import UserManager, encrypt_password

        init_database(self.uri)
        alice = UserManager.add("Alice", "Alic3", admin=True)
        crypt, salt = UserManager._hash_fields("Other1")
        User.create(
            name="alice",
            password=crypt,
            salt=salt,
            password_clear=encrypt_password("Other1"),
        )
        self.assertEqual(User.select().count(), 2)
        release_database()
        self._strip_shape()
        self._rewind("20261117")

        init_database(self.uri)

        users = {u.name: u for u in User.select()}
        self.assertEqual(set(users), {"Alice", "alice"})
        self.assertEqual(
            sorted(u.name_key for u in users.values() if u.name_key), ["alice"]
        )
        self.assertEqual(
            [u.name_key for u in users.values()].count(None), 1
        )
        self.assertEqual(users["Alice"].id, alice.id)
        self.assertIsNotNone(UserManager.try_auth("Alice", "Alic3"))
        self.assertIsNotNone(UserManager.try_auth("alice", "Other1"))

        with self.assertRaises(ValueError):
            UserManager.add("ALICE", "An0ther")
        with self.assertRaises(ValueError):
            UserManager.add("Álice", "An0ther")

        loser = next(u for u in users.values() if u.name_key is None)
        UserManager.rename(loser, "alice2")
        self.assertEqual(User.get(User.id == loser.id).name_key, "alice2")

    def test_name_key_is_unique(self):
        from neezer.db import User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3")
        with self.assertRaises(IntegrityError):
            User.create(name="other", name_key="alice", password="x", salt="xxxxxx")
        self.assertEqual(User.select().count(), 1)


class NewTablesMigrateDatabaseTestCase(unittest.TestCase):
    """``migrate_database`` walks ``_migration_order()``: a new table missing
    from it is silently left behind when an operator moves SQLite to Postgres.
    The two composite-key playlist tables and the transcript segment's generated
    id are the shapes that copy differently, so they are pinned here."""

    def setUp(self):
        self.src = tempfile.mkstemp(suffix=".db")
        self.dst = tempfile.mkstemp(suffix=".db")
        self.src_uri = "sqlite:///" + self.src[1]
        self.dst_uri = "sqlite:///" + self.dst[1]

    def tearDown(self):
        release_database()
        for fd, path in (self.src, self.dst):
            os.close(fd)
            try:
                os.remove(path)
            except OSError:
                pass

    def test_the_invite_table_is_copied(self):
        from neezer.db import Invite, User, migrate_database
        from neezer.managers.user import UserManager

        init_database(self.src_uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        Invite.create(
            id="AbC",
            token_hash="a" * 64,
            created_by=alice,
            created=datetime(2026, 1, 2, 3, 4, 5),
            expires=datetime(2026, 1, 9, 3, 4, 5),
            note="pour Léa",
        )
        release_database()

        copied = migrate_database(self.src_uri, self.dst_uri)

        self.assertEqual(copied["invite"], 1)
        moved = Invite.get()
        self.assertEqual((moved.id, moved.note), ("AbC", "pour Léa"))
        self.assertEqual(moved.token_hash, "a" * 64)
        self.assertEqual(moved.created_by.name, "alice")
        self.assertIsNone(moved.used_at)
        self.assertIsNone(moved.revoked_at)

    def test_every_new_table_is_copied(self):
        from neezer.db import PodcastEpisode, User, migrate_database
        from neezer.managers.user import UserManager

        init_database(self.src_uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        playlist = Playlist.create(
            user=alice,
            name="Shared",
            version=7,
            members_version=3,
            last_edited_by="alice",
            last_edited_at=datetime(2026, 1, 2, 3, 4, 5),
        )
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=2)
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        PlaylistPlacement.create(user=alice, playlist=playlist, folder=folder, position=4)
        # An unfiled placement: NULL folder_id must survive the copy too.
        unfiled = Playlist.create(user=alice, name="Unfiled")
        PlaylistPlacement.create(user=alice, playlist=unfiled, position=0)
        channel = PodcastChannel.create(user=alice, url="u", title="Show")
        episode = PodcastEpisode.create(channel=channel, title="Episode", duration=60)
        PodcastChapter.create(
            episode=episode, idx=0, start_ms=0, end_ms=1000, source="id3"
        )
        PodcastChapterScan.create(
            episode=episode, scanned_at=datetime(2026, 1, 2), source="id3", n_chapters=1
        )
        transcript = PodcastTranscript.create(
            episode=episode, status="ready", n_segments=1
        )
        worker = TranscribeWorker.create(
            name="Bureau", kind="client", token="encrypted", scopes="transcribe"
        )
        PodcastTranscriptLease.create(
            episode=transcript,
            worker=worker,
            lease_token_hash="c" * 64,
            acquired_at=datetime(2026, 1, 2),
            expires_at=datetime(2026, 1, 2, 0, 15),
            heartbeat_at=datetime(2026, 1, 2),
        )
        PodcastTranscriptSegment.create(
            transcript=PodcastTranscript.get(),
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Bonjour",
            search_text="bonjour",
        )
        release_database()

        copied = migrate_database(self.src_uri, self.dst_uri)

        self.assertEqual(copied["playlist_member"], 1)
        self.assertEqual(copied["playlist_placement"], 2)
        self.assertEqual(copied["podcast_transcript_segment"], 1)
        self.assertEqual(copied["transcribe_worker"], 1)
        self.assertEqual(copied["podcast_transcript_lease"], 1)
        self.assertEqual(TranscribeWorker.get().name, "Bureau")
        self.assertEqual(PodcastTranscriptLease.get().attempts, 0)
        moved = Playlist.get(Playlist.name == "Shared")
        self.assertEqual((moved.version, moved.members_version), (7, 3))
        self.assertEqual(moved.last_edited_by, "alice")
        self.assertEqual(
            moved.last_edited_at, datetime(2026, 1, 2, 3, 4, 5)
        )
        self.assertEqual(
            [(m.user_id, m.role) for m in PlaylistMember.select()],
            [(alice.id, "owner")],
        )
        self.assertEqual(
            sorted(
                ((p.folder_id, p.position) for p in PlaylistPlacement.select()),
                key=lambda row: row[1],
            ),
            [(None, 0), (folder.id, 4)],
        )
        self.assertEqual(PodcastTranscriptSegment.get().text, "Bonjour")
        # The generated ids still work after the copy with their explicit value.
        second = PodcastTranscriptSegment.create(
            transcript=PodcastTranscript.get(),
            idx=1,
            start_ms=1000,
            end_ms=2000,
            text="Encore",
            search_text="encore",
        )
        self.assertEqual(second.id, 2)


class TranscriptionSchemaTestCase(unittest.TestCase):
    """20261121: the transcript sidecar path, the per-show switches and the
    worker/lease tables. Additive on every engine, and a fresh install never
    runs migrations, so the base schemas must carry the same shape."""

    CHANNEL_COLUMNS = ("auto_transcribe", "transcribe_language")
    TRANSCRIPT_COLUMNS = ("source", "worker_name", "transcript_path")
    WORKER_COLUMNS = (
        "id",
        "name",
        "kind",
        "token",
        "scopes",
        "created",
        "last_seen",
        "enabled",
        "revoked",
    )
    LEASE_COLUMNS = (
        "episode_id",
        "worker_id",
        "lease_token_hash",
        "acquired_at",
        "expires_at",
        "heartbeat_at",
        "chunk_start_ms",
        "attempts",
        "last_error",
    )

    def _schema(self, provider):
        path = os.path.join(
            os.path.dirname(dbmod.__file__), "schema", f"{provider}.sql"
        )
        with open(path, encoding="utf-8") as fh:
            return fh.read()

    def _body(self, sql, table):
        m = re.search(
            r'CREATE TABLE(?: IF NOT EXISTS)? "?%s"? \((.*?)\n\)' % table, sql, re.S
        )
        self.assertIsNotNone(m, f"{table} missing")
        return m.group(1)

    def test_every_engine_has_the_columns_and_the_tables(self):
        for provider in PROVIDERS:
            sql = self._schema(provider)
            channel = self._body(sql, "podcast_channel")
            for column in self.CHANNEL_COLUMNS:
                self.assertRegex(channel, r"\b%s\b" % column, f"{provider} channel")
            transcript = self._body(sql, "podcast_transcript")
            for column in self.TRANSCRIPT_COLUMNS:
                self.assertRegex(transcript, r"\b%s\b" % column, f"{provider} transcript")
            worker = self._body(sql, "transcribe_worker")
            for column in self.WORKER_COLUMNS:
                self.assertRegex(worker, r"\b%s\b" % column, f"{provider} worker")
            lease = self._body(sql, "podcast_transcript_lease")
            for column in self.LEASE_COLUMNS:
                self.assertRegex(lease, r"\b%s\b" % column, f"{provider} lease")
            self.assertIn("index_transcribe_lease_expires", sql, provider)
            self.assertIn("index_transcribe_lease_worker", sql, provider)

    def test_every_engine_ships_the_migration(self):
        for provider in PROVIDERS:
            folder = os.path.join(MIGRATION_ROOT, provider)
            names = {os.path.splitext(n)[0] for n in os.listdir(folder)}
            self.assertIn("20261121", names, provider)

    def test_the_source_has_a_default_so_existing_rows_migrate(self):
        for provider in PROVIDERS:
            body = self._body(self._schema(provider), "podcast_transcript")
            self.assertRegex(
                body,
                r"source[^,\n]*NOT NULL DEFAULT 'server'",
                f"{provider} would leave old transcripts without a source",
            )
            channel = self._body(self._schema(provider), "podcast_channel")
            self.assertRegex(
                channel,
                r"auto_transcribe[^,\n]*NOT NULL DEFAULT (?:false|0)",
                f"{provider} would make the show switch undefined",
            )


class TranscriptionUpgradeTestCase(_TempDatabaseTestCase):
    """A 20261118 server keeps its shows, its archived episodes and its
    transcripts; the four columns and the two tables appear, nothing else moves.
    The migration is replayed twice (as the boot repair does) without loss."""

    def _seed(self):
        from neezer.db import PodcastEpisode, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        channel = PodcastChannel.create(
            user=alice, url="u", title="Show", auto_archive=True
        )
        episode = PodcastEpisode.create(
            channel=channel,
            title="Episode",
            duration=3600,
            path=os.path.join(self.path + "-archive", "Podcasts", "Show", "ep.mp3"),
            status="completed",
        )
        transcript = PodcastTranscript.create(
            episode=episode,
            status="ready",
            language="fr",
            engine="whisper.cpp",
            model="ggml-small.bin",
            duration_ms=3_600_000,
            done_ms=3_600_000,
            n_segments=2,
        )
        for idx, text in enumerate(("Bonjour", "Au revoir")):
            PodcastTranscriptSegment.create(
                transcript=transcript,
                idx=idx,
                start_ms=idx * 1000,
                end_ms=(idx + 1) * 1000,
                text=text,
                search_text=text.lower(),
            )
        return channel, episode, transcript

    def _strip_shape(self):
        self._sql(
            "DROP TABLE IF EXISTS podcast_transcript_lease",
            "DROP TABLE IF EXISTS transcribe_worker",
            "ALTER TABLE podcast_transcript DROP COLUMN source",
            "ALTER TABLE podcast_transcript DROP COLUMN worker_name",
            "ALTER TABLE podcast_transcript DROP COLUMN transcript_path",
            "ALTER TABLE podcast_channel DROP COLUMN auto_transcribe",
            "ALTER TABLE podcast_channel DROP COLUMN transcribe_language",
        )

    def test_an_upgraded_server_keeps_its_data_and_gets_the_shape(self):
        channel, episode, transcript = self._seed()
        release_database()
        self._strip_shape()
        self._rewind("20261118")
        cols = {r[1] for r in self._sql("PRAGMA table_info(podcast_transcript)")[0]}
        self.assertNotIn("transcript_path", cols)

        init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(dbmod.Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(TranscribeWorker.select().count(), 0)
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        again_channel = PodcastChannel.get(PodcastChannel.id == channel.id)
        self.assertFalse(again_channel.auto_transcribe)
        self.assertIsNone(again_channel.transcribe_language)
        again = PodcastTranscript.get(PodcastTranscript.episode == episode.id)
        self.assertEqual(again.status, "ready")
        self.assertEqual(again.language, "fr")
        self.assertEqual(again.source, "server")
        self.assertIsNone(again.worker_name)
        self.assertIsNone(again.transcript_path)
        self.assertEqual(again.n_segments, 2)
        self.assertEqual(
            [s.text for s in again.segments.order_by(PodcastTranscriptSegment.idx)],
            ["Bonjour", "Au revoir"],
        )
        self.assertEqual(again.episode.title, "Episode")

    def test_the_migration_replays_without_duplicating(self):
        self._seed()
        release_database()
        self._strip_shape()
        import importlib

        module = importlib.import_module("neezer.schema.migration.sqlite.20261121")
        for _ in range(2):
            module.apply({"database": self.path})
        init_database(self.uri)
        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(PodcastTranscriptSegment.select().count(), 2)

    def test_the_repair_path_puts_the_shape_back_on_a_current_database(self):
        _channel, episode, _transcript = self._seed()
        release_database()
        self._sql(
            "DROP TABLE podcast_transcript_lease",
            "DROP TABLE transcribe_worker",
            "ALTER TABLE podcast_transcript DROP COLUMN transcript_path",
            "ALTER TABLE podcast_channel DROP COLUMN auto_transcribe",
        )
        init_database(self.uri)
        self.assertEqual(dbmod._missing_columns(), [])
        row = PodcastTranscript.get(PodcastTranscript.episode == episode.id)
        self.assertEqual(row.transcript_path, None)
        self.assertEqual(PodcastChannel.get().auto_transcribe, False)
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)


class TranscriptionLeaseSchemaTestCase(_TempDatabaseTestCase):
    """The lease follows its transcript and its worker; the worker's token is
    not a secret the API may read back, but the row must hold every field the
    protocol uses."""

    def test_a_lease_follows_its_transcript_and_its_worker(self):
        from neezer.db import PodcastEpisode, User
        from neezer.managers.user import UserManager

        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        alice = User.get(User.name == "alice")
        channel = PodcastChannel.create(user=alice, url="u", title="Show")
        episode = PodcastEpisode.create(channel=channel, title="Episode", duration=60)
        transcript = PodcastTranscript.create(episode=episode, status="queued")
        worker = TranscribeWorker.create(
            name="Bureau", kind="client", token="encrypted", scopes="transcribe"
        )
        lease = PodcastTranscriptLease.create(
            episode=transcript,
            worker=worker,
            lease_token_hash="a" * 64,
            expires_at=datetime.now(),
            heartbeat_at=datetime.now(),
            attempts=1,
        )
        self.assertEqual(lease.episode.episode_id, transcript.episode_id)
        transcript.delete_instance()
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)

        transcript = PodcastTranscript.create(episode=episode, status="queued")
        PodcastTranscriptLease.create(
            episode=transcript,
            worker=worker,
            lease_token_hash="b" * 64,
            expires_at=datetime.now(),
            heartbeat_at=datetime.now(),
        )
        worker.delete_instance()
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)


class PlaylistShareForeignKeyTestCase(unittest.TestCase):
    """20261122: the cascade `playlist_share` always assumed.

    20261113 wrote ``REFERENCES playlist(id) ON DELETE CASCADE`` inside the
    column list. MySQL parses that and keeps NOTHING (``SHOW CREATE TABLE`` had
    no FOREIGN KEY at all), so deleting a playlist or an account left its public
    links behind for ever — and the same schema version then behaved
    differently on SQLite/PostgreSQL and on MySQL.
    """

    def _schema(self, provider):
        path = os.path.join(
            os.path.dirname(dbmod.__file__), "schema", f"{provider}.sql"
        )
        with open(path, encoding="utf-8") as fh:
            return fh.read()

    def _table_body(self, sql):
        m = re.search(
            r"CREATE TABLE IF NOT EXISTS playlist_share \((.*?)\n\)", sql, re.S
        )
        self.assertIsNotNone(m, "playlist_share missing")
        return m.group(1)

    def test_mysql_declares_both_foreign_keys(self):
        body = self._table_body(self._schema("mysql"))
        self.assertIn("CONSTRAINT playlist_share_playlist_fk FOREIGN KEY", body)
        self.assertIn("CONSTRAINT playlist_share_user_fk FOREIGN KEY", body)
        # Both cascade: a deleted playlist or account takes its links with it.
        self.assertEqual(body.count("ON DELETE CASCADE"), 2)
        # The inline form MySQL ignores must be gone.
        self.assertNotRegex(body, r"playlist_id CHAR\(32\) NOT NULL REFERENCES")

    def test_every_engine_ships_the_migration(self):
        for provider in PROVIDERS:
            folder = os.path.join(MIGRATION_ROOT, provider)
            self.assertTrue(
                os.path.exists(os.path.join(folder, "20261122.py"))
                or os.path.exists(os.path.join(folder, "20261122.sql")),
                provider,
            )
        with open(
            os.path.join(MIGRATION_ROOT, "mysql", "20261122.py"), encoding="utf-8"
        ) as fh:
            text = fh.read()
        # Idempotent BEFORE the ALTER: the orphans it removes are exactly what
        # would make ADD CONSTRAINT fail with error 1452.
        self.assertLess(
            text.index("DELETE FROM playlist_share"),
            text.index("ALTER TABLE playlist_share ADD CONSTRAINT"),
        )
        self.assertIn("information_schema.table_constraints", text)
        self.assertIn("ON DELETE CASCADE", text)

    def test_the_orphan_sweep_removes_a_link_without_its_playlist(self):
        """The 20261122 statement, twice (it is replayed on a skipped boot)."""
        fd, path = tempfile.mkstemp()
        uri = "sqlite:///" + path
        try:
            init_database(uri)
            from neezer.db import PlaylistShare, User
            from neezer.managers.user import UserManager

            alice = UserManager.add("alice", "Alic3", admin=True)
            playlist = Playlist.create(user=alice, name="Partagée")
            # An orphan can only be born with the check off (that is what a
            # MySQL server without the constraint did every day).
            raw = dbmod.db.obj
            raw.execute_sql("PRAGMA foreign_keys = OFF")
            PlaylistShare.create(
                id="orphan",
                playlist=playlist,
                user=alice,
                created=datetime(2026, 1, 1),
            )
            Playlist.delete().where(Playlist.id == playlist.id).execute()
            raw.execute_sql("PRAGMA foreign_keys = ON")
            self.assertEqual(PlaylistShare.select().count(), 1)
            for _ in range(2):
                dbmod.execute_sql_resource_script(
                    "schema/migration/sqlite/20261122.sql"
                )
            self.assertEqual(PlaylistShare.select().count(), 0)
            self.assertEqual(
                User.select().where(User.name == "alice").count(), 1
            )
        finally:
            release_database()
            os.close(fd)
            os.remove(path)


if __name__ == "__main__":
    unittest.main()
