# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The upgrade path on the engines a server really runs.

``tests/test_schema.py`` proves the shapes on SQLite, and a hand-rebuilt SQLite
file cannot tell whether a PostgreSQL or MySQL migration parses, whether its
backfill keeps the rows, or whether the boot repair's information_schema check
sees the shape. This module runs the lot against a database named by
``NEEZER_TEST_DATABASE`` (PostgreSQL or MySQL), dropping every table between
cases. The database name must contain "test"; ``NS_REQUIRE_DATABASE=1`` turns a
missing URI into a failure instead of a skip.
"""

import importlib
import os
import re
import unittest

from datetime import datetime, timedelta
from urllib.parse import urlparse

from peewee import IntegrityError

from neezer import db as dbmod
from neezer.db import (
    Invite,
    LanDevice,
    LanDeviceKey,
    LanFriend,
    LanFriendPlaylist,
    Meta,
    Playlist,
    PlaylistFolder,
    PlaylistMember,
    PlaylistPlacement,
    PodcastChannel,
    PodcastChapter,
    PodcastChapterScan,
    PodcastEpisode,
    PodcastTranscript,
    PodcastTranscriptLease,
    PodcastTranscriptSegment,
    SCHEMA_VERSION,
    TranscribeWorker,
    User,
    db,
    init_database,
    release_database,
)
from neezer.managers.user import UserManager

URI = os.environ.get("NEEZER_TEST_DATABASE", "").strip()
REQUIRE_DATABASE = os.environ.get("NS_REQUIRE_DATABASE") == "1"
LOT = (
    "20261114",
    "20261115",
    "20261116",
    "20261117",
    "20261118",
    "20261121",
    "20261124",
)

EXPECTED_SHAPE = {
    "playlist": (
        "version",
        "members_version",
        "last_edited_by",
        "last_edited_at",
    ),
    "playlist_folder": ("id", "user_id", "name", "position", "created"),
    "playlist_placement": ("user_id", "playlist_id", "folder_id", "position"),
    "playlist_member": ("playlist_id", "user_id", "role", "added"),
    "invite": (
        "id",
        "token_hash",
        "created_by",
        "created",
        "expires",
        "used_by",
        "used_at",
        "revoked_at",
        "note",
    ),
    "user": (
        "name_key",
        "disabled",
        "last_login",
        "onboarding_step",
        "lan_serial",
    ),
    "lan_device": (
        "id",
        "user_id",
        "pubkey",
        "name",
        "platform",
        "created",
        "last_seen",
        "not_after",
        "revoked",
    ),
    "lan_device_key": ("pubkey", "device_id", "not_after"),
    "lan_friend": ("owner_id", "friend_id", "share_all", "created"),
    "lan_friend_playlist": ("owner_id", "friend_id", "playlist_id"),
    "podcast_chapter": (
        "id",
        "episode_id",
        "idx",
        "start_ms",
        "end_ms",
        "title",
        "source",
    ),
    "podcast_chapter_scan": (
        "episode_id",
        "scanned_at",
        "source",
        "n_chapters",
        "error",
    ),
    "podcast_transcript": (
        "episode_id",
        "status",
        "language",
        "engine",
        "model",
        "engine_version",
        "duration_ms",
        "done_ms",
        "n_segments",
        "error_code",
        "error_message",
        "cancel_requested",
        "source",
        "worker_name",
        "transcript_path",
        "user_id",
        "queued_at",
        "started_at",
        "finished_at",
        "updated",
    ),
    "podcast_transcript_segment": (
        "id",
        "transcript_id",
        "idx",
        "start_ms",
        "end_ms",
        "text",
        "search_text",
    ),
    "transcribe_worker": (
        "id",
        "name",
        "kind",
        "token",
        "scopes",
        "created",
        "last_seen",
        "enabled",
        "revoked",
    ),
    "podcast_transcript_lease": (
        "episode_id",
        "worker_id",
        "lease_token_hash",
        "acquired_at",
        "expires_at",
        "heartbeat_at",
        "chunk_start_ms",
        "attempts",
        "last_error",
    ),
}

_IDENTIFIER = re.compile(r"\A[a-z0-9_]+\Z")
_CONFIG = {}


def _describe(parsed):
    return (
        f"{parsed.scheme}://{parsed.hostname or 'localhost'}"
        f":{parsed.port or ''}/{parsed.path.lstrip('/')}"
    )


def _resolve():
    if not URI:
        if REQUIRE_DATABASE:
            raise AssertionError(
                "NS_REQUIRE_DATABASE=1 but NEEZER_TEST_DATABASE is not set"
            )
        raise unittest.SkipTest(
            "NEEZER_TEST_DATABASE is not set: the PostgreSQL/MySQL upgrade path "
            "runs in the database job of tests.yaml"
        )
    parsed = urlparse(URI)
    if parsed.scheme.startswith("postgres"):
        provider, driver = "postgres", "psycopg2"
    elif parsed.scheme.startswith("mysql"):
        provider, driver = "mysql", "pymysql"
    else:
        raise AssertionError(
            "NEEZER_TEST_DATABASE must point at PostgreSQL or MySQL, "
            f"not {parsed.scheme!r}"
        )
    database = parsed.path.lstrip("/")
    if "test" not in database.lower():
        raise AssertionError(
            f"refusing to wipe {database!r}: the database name must contain 'test'"
        )
    try:
        importlib.import_module(driver)
    except ImportError as exc:
        raise AssertionError(
            f"install the {provider} extra: pip install -e .[{provider}]"
        ) from exc
    try:
        probe, _, args = dbmod._database_from_uri(URI)
        probe.connect()
        probe.close()
    except Exception as exc:
        raise AssertionError(
            f"cannot reach {_describe(parsed)}: {type(exc).__name__}: {exc}"
        ) from exc
    _CONFIG.update(provider=provider, args=args)


def setUpModule():
    _resolve()


def _identifier(name):
    if not _IDENTIFIER.fullmatch(name):
        raise AssertionError(f"unexpected identifier from the catalog: {name!r}")
    return name


def _quote(name):
    return f'"{name}"' if _CONFIG["provider"] == "postgres" else f"`{name}`"


def _tables():
    where = (
        "current_schema()" if _CONFIG["provider"] == "postgres" else "DATABASE()"
    )
    rows = db.execute_sql(
        "SELECT table_name FROM information_schema.tables "
        f"WHERE table_schema = {where}"  # nosec B608 - fixed catalog query
    )
    return {row[0].lower() for row in rows}


def _columns(table):
    where = (
        "current_schema()" if _CONFIG["provider"] == "postgres" else "DATABASE()"
    )
    rows = db.execute_sql(
        "SELECT column_name FROM information_schema.columns "
        f"WHERE table_schema = {where} AND table_name = %s",  # nosec B608
        (table,),
    )
    return {row[0].lower() for row in rows}


def _wipe():
    database, provider, _ = dbmod._database_from_uri(URI)
    database.connect()
    try:
        if provider == "postgres":
            names = [
                row[0]
                for row in database.execute_sql(
                    "SELECT tablename FROM pg_tables"
                    " WHERE schemaname = current_schema()"
                )
            ]
            for name in names:
                database.execute_sql(
                    f'DROP TABLE IF EXISTS "{_identifier(name)}" CASCADE'  # nosec B608
                )
        else:
            names = [
                row[0]
                for row in database.execute_sql(
                    "SELECT table_name FROM information_schema.tables "
                    "WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE'"
                )
            ]
            database.execute_sql("SET FOREIGN_KEY_CHECKS = 0")
            try:
                for name in names:
                    database.execute_sql(
                        f"DROP TABLE IF EXISTS `{_identifier(name)}`"  # nosec B608
                    )
            finally:
                database.execute_sql("SET FOREIGN_KEY_CHECKS = 1")
    finally:
        database.close()


def _fresh():
    release_database()
    _wipe()
    init_database(URI)


def _drop_table(name):
    db.execute_sql(
        f"DROP TABLE IF EXISTS {_quote(_identifier(name))}"
        + (" CASCADE" if _CONFIG["provider"] == "postgres" else "")
    )


def _drop_column(table, column):
    if _CONFIG["provider"] == "postgres":
        db.execute_sql(
            f"ALTER TABLE {_quote(table)} DROP COLUMN IF EXISTS {_quote(column)}"
        )
    elif column in _columns(table):
        db.execute_sql(f"ALTER TABLE {_quote(table)} DROP COLUMN {_quote(column)}")


def _rewind(version):
    Meta.update(value=version).where(Meta.key == "schema_version").execute()


def _replay(date):
    provider = _CONFIG["provider"]
    name = next(
        n for n in dbmod.list_migrations(provider) if os.path.splitext(n)[0] == date
    )
    if name.endswith(".sql"):
        with db.atomic():
            dbmod.execute_sql_resource_script(f"schema/migration/{provider}/{name}")
    else:
        module = importlib.import_module(f"neezer.schema.migration.{provider}.{date}")
        module.apply(dict(_CONFIG["args"]))


def _seed_users():
    return (
        UserManager.add("Alice", "Alic3", admin=True),
        UserManager.add("bob", "B0bbb"),
    )


def _seed_show(user):
    channel = PodcastChannel.create(
        user=user, url="https://example.test/feed.xml", title="Show"
    )
    episode = PodcastEpisode.create(
        channel=channel,
        title="Episode",
        duration=3600,
        path="/srv/neezer/Podcasts/Show/episode.mp3",
        status="completed",
    )
    return channel, episode


def _assert_shape(test):
    tables = _tables()
    for table, columns in EXPECTED_SHAPE.items():
        test.assertIn(table, tables, f"{table} is missing")
        test.assertLessEqual(
            set(columns), _columns(table), f"{table} is missing columns"
        )


def _strip_to_20261113():
    for table in (
        "playlist_placement",
        "playlist_folder",
        "playlist_member",
        "podcast_transcript_lease",
        "transcribe_worker",
        "podcast_transcript_segment",
        "podcast_transcript",
        "podcast_chapter_scan",
        "podcast_chapter",
        "invite",
    ):
        _drop_table(table)
    for column in ("version", "members_version", "last_edited_by", "last_edited_at"):
        _drop_column("playlist", column)
    for column in ("name_key", "disabled", "last_login", "onboarding_step"):
        _drop_column("user", column)


def _strip_to_20261118():
    for table in ("podcast_transcript_lease", "transcribe_worker"):
        _drop_table(table)
    for column in ("source", "worker_name", "transcript_path"):
        _drop_column("podcast_transcript", column)
    for column in ("auto_transcribe", "transcribe_language"):
        _drop_column("podcast_channel", column)


def _strip_to_20261123():
    """The LAN lot (20261124) is the only thing a 20261123 server lacks."""
    for table in (
        "lan_friend_playlist",
        "lan_friend",
        "lan_device_key",
        "lan_device",
    ):
        _drop_table(table)
    _drop_column("user", "lan_serial")


def _seed_lan_device(alice):
    """One enrolled device with a rotated-away key."""
    device = LanDevice.create(
        id="0123456789abcdef01234567",
        user=alice,
        pubkey="a" * 64,
        name="PC du salon",
        platform="desktop",
        created=datetime(2026, 1, 1),
        last_seen=datetime(2026, 1, 2),
        not_after=datetime(2026, 2, 1),
    )
    LanDeviceKey.create(
        pubkey="b" * 64, device=device, not_after=datetime(2026, 1, 20)
    )
    LanDeviceKey.create(
        pubkey="a" * 64, device=device, not_after=datetime(2026, 2, 1)
    )
    return device


def _seed_lan(alice, bob, playlist):
    """One enrolled device, and one friendship with a playlist scope — the rows
    20261124 creates, so a replay has something to leave alone."""
    device = _seed_lan_device(alice)
    LanFriend.create(
        owner=alice, friend=bob, share_all=False, created=datetime(2026, 1, 1)
    )
    LanFriendPlaylist.create(owner=alice, friend=bob, playlist=playlist)
    return device


class _ExternalDatabaseTestCase(unittest.TestCase):
    def setUp(self):
        self.addCleanup(release_database)
        _fresh()


class FreshSchemaTestCase(_ExternalDatabaseTestCase):
    """A brand-new PostgreSQL or MySQL database is created from the base schema
    alone, with no migration, so every model column must be there."""

    def setUp(self):
        self.addCleanup(release_database)
        release_database()
        _wipe()

    def test_a_fresh_install_has_every_model_column(self):
        with self.assertNoLogs("neezer.db", level="WARNING"):
            init_database(URI)
        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(dbmod._missing_columns(), [])
        _assert_shape(self)
        for model in (
            Invite,
            Playlist,
            PlaylistFolder,
            PlaylistMember,
            PlaylistPlacement,
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
            TranscribeWorker,
            PodcastTranscriptLease,
            LanDevice,
            LanDeviceKey,
            LanFriend,
            LanFriendPlaylist,
        ):
            self.assertEqual(model.select().count(), 0)


class MigrationReplayTestCase(_ExternalDatabaseTestCase):
    """_repair_skipped_migrations replays every migration since 20260808 on a
    database that skipped one, so the 20261114..20261121 lot must run twice on
    populated tables without duplicating or losing a row."""

    def test_the_lot_replays_twice_without_losing_a_row(self):
        alice, bob = _seed_users()
        playlist = Playlist.create(user=alice, name="Mix")
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=0)
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        PlaylistMember.create(playlist=playlist, user=bob, role="editor")
        PlaylistPlacement.create(
            user=alice, playlist=playlist, folder=folder, position=0
        )
        channel, episode = _seed_show(alice)
        PodcastChapter.create(
            episode=episode, idx=0, start_ms=0, end_ms=1000, source="id3"
        )
        PodcastChapterScan.create(
            episode=episode,
            scanned_at=datetime(2026, 1, 1),
            source="id3",
            n_chapters=1,
        )
        transcript = PodcastTranscript.create(
            episode=episode, status="ready", n_segments=2
        )
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Bonjour",
            search_text="bonjour",
        )
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=1,
            start_ms=1000,
            end_ms=2000,
            text="Au revoir",
            search_text="au revoir",
        )
        lan_device = _seed_lan(alice, bob, playlist)

        for _ in range(2):
            for date in LOT:
                _replay(date)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(Playlist.select().count(), 1)
        self.assertEqual(
            sorted((m.user_id, m.role) for m in PlaylistMember.select()),
            sorted(((alice.id, "owner"), (bob.id, "editor"))),
        )
        self.assertEqual(
            PlaylistMember.select().where(PlaylistMember.role == "owner").count(), 1
        )
        self.assertEqual(PlaylistFolder.select().count(), 1)
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        self.assertEqual(PodcastChapter.select().count(), 1)
        self.assertEqual(PodcastChapterScan.select().count(), 1)
        self.assertEqual(PodcastTranscript.select().count(), 1)
        self.assertEqual(
            [
                segment.text
                for segment in PodcastTranscriptSegment.select().order_by(
                    PodcastTranscriptSegment.idx
                )
            ],
            ["Bonjour", "Au revoir"],
        )
        self.assertEqual(PodcastTranscript.get().source, "server")
        show = PodcastChannel.get(PodcastChannel.id == channel.id)
        self.assertFalse(show.auto_transcribe)
        self.assertEqual(LanDevice.select().count(), 1)
        self.assertEqual(LanDeviceKey.select().count(), 2)
        self.assertEqual(
            LanDevice.get(LanDevice.id == lan_device.id).name, "PC du salon"
        )
        self.assertEqual(LanFriend.select().count(), 1)
        self.assertEqual(LanFriendPlaylist.select().count(), 1)


class UpgradeFrom20261113TestCase(_ExternalDatabaseTestCase):
    """A 20261113 server has playlists and shows but none of the folder,
    collaboration, account or derived-data shape. Upgrading must keep every row
    and add exactly the owner memberships the backfill intends."""

    def test_an_upgraded_server_keeps_its_rows_and_gains_the_shape(self):
        alice, bob = _seed_users()
        start = datetime(2026, 1, 1, 12, 0, 0)
        playlists = [
            Playlist.create(user=alice, name="Alice one", created=start),
            Playlist.create(
                user=bob,
                name="Bob one",
                created=start + timedelta(minutes=1),
                deezer_id="1234567",
            ),
            Playlist.create(
                user=alice, name="Alice two", created=start + timedelta(minutes=2)
            ),
        ]
        before = [
            (p.id, p.user_id, p.name, p.created, p.deezer_id)
            for p in Playlist.select().order_by(Playlist.created, Playlist.id)
        ]
        users_before = {
            u.name: (u.password, u.salt, u.admin, u.mail) for u in User.select()
        }
        channel, episode = _seed_show(alice)

        _strip_to_20261113()
        _rewind("20261113")
        release_database()
        init_database(URI)

        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(dbmod._missing_columns(), [])
        _assert_shape(self)
        self.assertEqual(
            [
                (p.id, p.user_id, p.name, p.created, p.deezer_id)
                for p in Playlist.select().order_by(Playlist.created, Playlist.id)
            ],
            before,
        )
        self.assertEqual(PlaylistFolder.select().count(), 0)
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        members = list(PlaylistMember.select().order_by(PlaylistMember.added))
        self.assertEqual(len(members), len(playlists))
        for member in members:
            self.assertEqual(member.role, "owner")
            self.assertEqual(member.user_id, member.playlist.user_id)
            self.assertEqual(member.added, member.playlist.created)
        self.assertTrue(
            all(p.version == 1 and p.members_version == 1 for p in Playlist.select())
        )
        again = PodcastEpisode.get(PodcastEpisode.id == episode.id)
        self.assertEqual(
            (again.title, again.path, again.status, again.channel_id),
            (
                "Episode",
                "/srv/neezer/Podcasts/Show/episode.mp3",
                "completed",
                channel.id,
            ),
        )
        for model in (
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
            TranscribeWorker,
            PodcastTranscriptLease,
            Invite,
        ):
            self.assertEqual(model.select().count(), 0)
        users = {u.name: u for u in User.select()}
        self.assertEqual(set(users), set(users_before))
        for user in users.values():
            self.assertEqual(
                (user.password, user.salt, user.admin, user.mail),
                users_before[user.name],
            )
            self.assertEqual(user.onboarding_step, "done")
            self.assertFalse(user.disabled)
            self.assertIsNone(user.last_login)
        self.assertEqual(users["Alice"].name_key, "alice")
        self.assertEqual(users["bob"].name_key, "bob")
        self.assertIsNotNone(UserManager.try_auth("Alice", "Alic3"))
        self.assertIsNotNone(UserManager.try_auth("bob", "B0bbb"))


class UpgradeFrom20261118TestCase(_ExternalDatabaseTestCase):
    """A 20261118 server already has folders, members, chapters and transcripts;
    20261121 is additive only and must leave every one of them untouched."""

    def test_an_upgraded_server_keeps_its_derived_rows(self):
        alice, bob = _seed_users()
        playlist = Playlist.create(
            user=alice,
            name="Shared",
            version=7,
            members_version=3,
            last_edited_by="alice",
            last_edited_at=datetime(2026, 1, 2, 3, 4, 5),
        )
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        PlaylistMember.create(playlist=playlist, user=bob, role="editor")
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=2)
        PlaylistPlacement.create(
            user=alice, playlist=playlist, folder=folder, position=4
        )
        channel, episode = _seed_show(alice)
        chapter = PodcastChapter.create(
            episode=episode,
            idx=0,
            start_ms=0,
            end_ms=1000,
            title="Intro",
            source="id3",
        )
        PodcastChapterScan.create(
            episode=episode,
            scanned_at=datetime(2026, 1, 2),
            source="id3",
            n_chapters=1,
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
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Bonjour",
            search_text="bonjour",
        )
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=1,
            start_ms=1000,
            end_ms=2000,
            text="Au revoir",
            search_text="au revoir",
        )

        _strip_to_20261118()
        _rewind("20261118")
        release_database()
        init_database(URI)

        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(dbmod._missing_columns(), [])
        _assert_shape(self)
        again = Playlist.get(Playlist.id == playlist.id)
        self.assertEqual((again.version, again.members_version), (7, 3))
        self.assertEqual(again.last_edited_by, "alice")
        self.assertEqual(again.last_edited_at, datetime(2026, 1, 2, 3, 4, 5))
        self.assertEqual(
            {m.user_id: m.role for m in PlaylistMember.select()},
            {alice.id: "owner", bob.id: "editor"},
        )
        placement = PlaylistPlacement.get()
        self.assertEqual((placement.folder_id, placement.position), (folder.id, 4))
        stored_chapter = PodcastChapter.get(PodcastChapter.id == chapter.id)
        self.assertEqual((stored_chapter.title, stored_chapter.end_ms), ("Intro", 1000))
        stored_scan = PodcastChapterScan.get(
            PodcastChapterScan.episode == episode.id
        )
        self.assertEqual((stored_scan.n_chapters, stored_scan.source), (1, "id3"))
        moved = PodcastTranscript.get(PodcastTranscript.episode == episode.id)
        self.assertEqual(
            (
                moved.status,
                moved.language,
                moved.engine,
                moved.model,
                moved.duration_ms,
                moved.done_ms,
                moved.n_segments,
            ),
            ("ready", "fr", "whisper.cpp", "ggml-small.bin", 3_600_000, 3_600_000, 2),
        )
        self.assertEqual(moved.source, "server")
        self.assertIsNone(moved.worker_name)
        self.assertIsNone(moved.transcript_path)
        self.assertEqual(
            [
                segment.text
                for segment in PodcastTranscriptSegment.select().order_by(
                    PodcastTranscriptSegment.idx
                )
            ],
            ["Bonjour", "Au revoir"],
        )
        show = PodcastChannel.get(PodcastChannel.id == channel.id)
        self.assertFalse(show.auto_transcribe)
        self.assertIsNone(show.transcribe_language)
        self.assertEqual(TranscribeWorker.select().count(), 0)
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)


class UpgradeFrom20261123TestCase(_ExternalDatabaseTestCase):
    """A 20261123 server has everything but the LAN lot: no enrolled device, no
    key registry, no friendship and no roster counter. The migration is additive
    and idempotent, and it must keep every existing row and account intact."""

    def test_an_upgraded_server_gains_the_lan_shape_and_keeps_its_rows(self):
        alice, bob = _seed_users()
        playlist = Playlist.create(user=alice, name="Mix")
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        channel, episode = _seed_show(alice)
        users_before = {
            u.name: (u.password, u.salt, u.admin, u.mail, u.name_key)
            for u in User.select()
        }

        _strip_to_20261123()
        _rewind("20261123")
        release_database()
        init_database(URI)

        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(dbmod._missing_columns(), [])
        _assert_shape(self)
        self.assertEqual(Playlist.select().count(), 1)
        self.assertEqual(PlaylistMember.select().count(), 1)
        self.assertEqual(PodcastEpisode.select().count(), 1)
        self.assertEqual(
            {
                u.name: (u.password, u.salt, u.admin, u.mail, u.name_key)
                for u in User.select()
            },
            users_before,
        )
        # The roster counter starts at 0 for every account an upgrade carried.
        self.assertEqual([u.lan_serial for u in User.select()], [0, 0])
        for model in (LanDevice, LanDeviceKey, LanFriend, LanFriendPlaylist):
            self.assertEqual(model.select().count(), 0)
        # Replaying the lot on a database that already has it changes nothing.
        for _ in range(2):
            _replay("20261124")
        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(Playlist.select().count(), 1)
        self.assertEqual(User.select().count(), 2)


class UniqueKeysTestCase(_ExternalDatabaseTestCase):
    """The folded name and the invitation's token hash are the two unique keys
    the account lot relies on, and each must be enforced by the engine itself."""

    def test_name_key_and_token_hash_are_unique(self):
        UserManager.add("Alice", "Alic3")
        with self.assertRaises(IntegrityError):
            User.create(name="Other", name_key="alice", password="x", salt="xxxxxx")
        Invite.create(
            id="invite-one", token_hash="a" * 64, expires=datetime(2026, 1, 1)
        )
        with self.assertRaises(IntegrityError):
            Invite.create(
                id="invite-two", token_hash="a" * 64, expires=datetime(2026, 1, 2)
            )
        self.assertEqual(User.select().count(), 1)
        self.assertEqual(Invite.select().count(), 1)


class CascadeTestCase(_ExternalDatabaseTestCase):
    """The rows the lot derives from a playlist, an episode and a transcript
    are deleted by the engine's own foreign keys, not by Python."""

    def test_derived_rows_follow_their_parents(self):
        alice, bob = _seed_users()
        playlist = Playlist.create(user=alice, name="Mix")
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=0)
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        PlaylistMember.create(playlist=playlist, user=bob, role="viewer")
        PlaylistPlacement.create(
            user=alice, playlist=playlist, folder=folder, position=0
        )

        playlist.delete_instance()
        self.assertEqual(PlaylistMember.select().count(), 0)
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        self.assertEqual(PlaylistFolder.select().count(), 1)
        self.assertEqual(User.select().count(), 2)

        kept = Playlist.create(user=alice, name="Kept")
        PlaylistPlacement.create(user=alice, playlist=kept, folder=folder, position=0)
        folder.delete_instance()
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        self.assertEqual(Playlist.select().count(), 1)

        channel, episode = _seed_show(alice)
        PodcastChapter.create(
            episode=episode, idx=0, start_ms=0, end_ms=1000, source="id3"
        )
        PodcastChapterScan.create(
            episode=episode,
            scanned_at=datetime(2026, 1, 2),
            source="id3",
            n_chapters=1,
        )
        transcript = PodcastTranscript.create(episode=episode, status="ready")
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Ligne",
            search_text="ligne",
        )
        transcript.delete_instance()
        self.assertEqual(PodcastTranscriptSegment.select().count(), 0)
        self.assertEqual(PodcastChapter.select().count(), 1)

        transcript = PodcastTranscript.create(episode=episode, status="ready")
        PodcastTranscriptSegment.create(
            transcript=transcript,
            idx=0,
            start_ms=0,
            end_ms=1000,
            text="Encore",
            search_text="encore",
        )
        worker = TranscribeWorker.create(
            name="Bureau", kind="client", token="encrypted", scopes="transcribe"
        )
        PodcastTranscriptLease.create(
            episode=transcript,
            worker=worker,
            lease_token_hash="b" * 64,
            expires_at=datetime(2026, 1, 2),
        )
        worker.delete_instance()
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)

        episode.delete_instance()
        for model in (
            PodcastChapter,
            PodcastChapterScan,
            PodcastTranscript,
            PodcastTranscriptSegment,
        ):
            self.assertEqual(model.select().count(), 0)
        self.assertEqual(PodcastChannel.select().count(), 1)

    def test_lan_rows_follow_their_account_device_and_playlist(self):
        """The four LAN tables are held by the engine's own foreign keys: a
        deleted playlist drops the grant, a deleted device drops its keys, and a
        deleted account drops its friendships."""
        alice, bob = _seed_users()
        shared = Playlist.create(user=alice, name="Shared")
        other = Playlist.create(user=alice, name="Kept")
        LanFriend.create(owner=alice, friend=bob, share_all=False)
        LanFriendPlaylist.create(owner=alice, friend=bob, playlist=shared)
        device = _seed_lan_device(alice)
        self.assertEqual(LanDeviceKey.select().count(), 2)

        shared.delete_instance()
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        self.assertEqual(LanFriend.select().count(), 1)

        device.delete_instance()
        self.assertEqual(LanDeviceKey.select().count(), 0)

        LanFriendPlaylist.create(owner=alice, friend=bob, playlist=other)
        bob.delete_instance()
        self.assertEqual(LanFriend.select().count(), 0)
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        self.assertEqual(Playlist.select().count(), 1)


class RepairPathTestCase(_ExternalDatabaseTestCase):
    """A database marked current but missing a table or a column is repaired at
    boot: this is the shape check that reads information_schema, and the replays
    must not duplicate an owner membership nor remove an editor."""

    def test_a_missing_shape_is_repaired_without_touching_rows(self):
        alice, bob = _seed_users()
        playlist = Playlist.create(user=alice, name="Mix")
        folder = PlaylistFolder.create(user=alice, name="Jazz", position=0)
        PlaylistMember.create(playlist=playlist, user=alice, role="owner")
        PlaylistMember.create(playlist=playlist, user=bob, role="editor")
        PlaylistPlacement.create(
            user=alice, playlist=playlist, folder=folder, position=1
        )
        _drop_table("invite")
        _drop_table("podcast_transcript_lease")
        _drop_table("lan_device_key")
        _drop_table("lan_friend_playlist")
        _drop_column("podcast_transcript", "worker_name")
        _drop_column("user", "name_key")
        _drop_column("user", "lan_serial")

        release_database()
        init_database(URI)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertEqual(Invite.select().count(), 0)
        self.assertEqual(PodcastTranscriptLease.select().count(), 0)
        self.assertEqual(LanDeviceKey.select().count(), 0)
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        self.assertIn("worker_name", _columns("podcast_transcript"))
        self.assertIn("lan_serial", _columns("user"))
        self.assertEqual(
            {m.user_id: m.role for m in PlaylistMember.select()},
            {alice.id: "owner", bob.id: "editor"},
        )
        self.assertEqual(PlaylistMember.select().count(), 2)
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        self.assertEqual(User.get(User.name == "Alice").name_key, "alice")
        self.assertIsNotNone(UserManager.try_auth("Alice", "Alic3"))


if __name__ == "__main__":
    unittest.main()
