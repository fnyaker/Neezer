"""DC-03: four migrations shipped as standalone scripts.

``neezer.db.init_database`` imports every ``.py`` migration and calls
``apply(args.copy())``. These four used to run ``argparse.parse_args()`` at module
level, so importing them killed the upgrade with ``SystemExit(2)`` — and two of
them still called ``iteritems``. This pins the contract the engine expects and
replays the SQLite pair on the tables a 2017/2018 database has.
"""

import hashlib
import importlib
import inspect
import os
import sqlite3
import sys
import tempfile
import unittest

from unittest import mock
from uuid import UUID, uuid4

LEGACY = (
    ("sqlite", "20171230"),
    ("sqlite", "20180317"),
    ("mysql", "20171230"),
    ("postgres", "20180317"),
)

DRIVERS = {
    "mysql": ("MySQLdb", "pymysql"),
    "postgres": ("psycopg2",),
}

SCHEMA = """
CREATE TABLE folder (id TEXT PRIMARY KEY, parent_id TEXT, path TEXT);
CREATE TABLE artist (id TEXT PRIMARY KEY);
CREATE TABLE album (id TEXT PRIMARY KEY, artist_id TEXT);
CREATE TABLE track (
    id TEXT PRIMARY KEY,
    album_id TEXT,
    artist_id TEXT,
    root_folder_id TEXT,
    folder_id TEXT,
    path TEXT
);
CREATE TABLE "user" (id TEXT PRIMARY KEY, last_play_id TEXT);
CREATE TABLE client_prefs (user_id TEXT);
CREATE TABLE starred_folder (user_id TEXT, starred_id TEXT);
CREATE TABLE starred_artist (user_id TEXT, starred_id TEXT);
CREATE TABLE starred_album (user_id TEXT, starred_id TEXT);
CREATE TABLE starred_track (user_id TEXT, starred_id TEXT);
CREATE TABLE rating_folder (user_id TEXT, rated_id TEXT);
CREATE TABLE rating_track (user_id TEXT, rated_id TEXT);
CREATE TABLE chat_message (id TEXT, user_id TEXT);
CREATE TABLE playlist (id TEXT, user_id TEXT);
"""


def _driver_missing(provider):
    for name in DRIVERS.get(provider, ()):
        try:
            importlib.import_module(name)
            return False
        except ImportError:
            continue
    return True


def _legacy_module(provider, date):
    name = f"neezer.schema.migration.{provider}.{date}"
    sys.modules.pop(name, None)
    with mock.patch.object(sys, "argv", ["neezer-server"]):
        return importlib.import_module(name)


def _seed(path):
    ids = {
        key: uuid4().hex
        for key in (
            "root",
            "folder",
            "artist",
            "album",
            "track",
            "user",
            "playlist",
            "chat",
        )
    }
    conn = sqlite3.connect(path)
    conn.executescript(SCHEMA)
    c = conn.cursor()
    c.execute(
        "INSERT INTO folder (id, parent_id, path) VALUES (?, NULL, '/music')",
        (ids["root"],),
    )
    c.execute(
        "INSERT INTO folder (id, parent_id, path) VALUES (?, ?, '/music/sub')",
        (ids["folder"], ids["root"]),
    )
    c.execute("INSERT INTO artist (id) VALUES (?)", (ids["artist"],))
    c.execute(
        "INSERT INTO album (id, artist_id) VALUES (?, ?)",
        (ids["album"], ids["artist"]),
    )
    c.execute(
        "INSERT INTO track (id, album_id, artist_id, root_folder_id, folder_id, path) "
        "VALUES (?, ?, ?, ?, ?, '/music/sub/one.flac')",
        (
            ids["track"],
            ids["album"],
            ids["artist"],
            ids["root"],
            ids["folder"],
        ),
    )
    c.execute('INSERT INTO "user" (id, last_play_id) VALUES (?, ?)', (ids["user"], None))
    c.execute("INSERT INTO client_prefs (user_id) VALUES (?)", (ids["user"],))
    c.execute(
        "INSERT INTO starred_folder (user_id, starred_id) VALUES (?, ?)",
        (ids["user"], ids["folder"]),
    )
    c.execute(
        "INSERT INTO starred_artist (user_id, starred_id) VALUES (?, NULL)",
        (ids["user"],),
    )
    c.execute(
        "INSERT INTO starred_album (user_id, starred_id) VALUES (?, ?)",
        (ids["user"], ids["album"]),
    )
    c.execute(
        "INSERT INTO starred_track (user_id, starred_id) VALUES (?, ?)",
        (ids["user"], ids["track"]),
    )
    c.execute(
        "INSERT INTO rating_folder (user_id, rated_id) VALUES (?, ?)",
        (ids["user"], ids["folder"]),
    )
    c.execute(
        "INSERT INTO rating_track (user_id, rated_id) VALUES (?, ?)",
        (ids["user"], ids["track"]),
    )
    c.execute(
        "INSERT INTO chat_message (id, user_id) VALUES (?, ?)",
        (ids["chat"], ids["user"]),
    )
    c.execute(
        "INSERT INTO playlist (id, user_id) VALUES (?, ?)",
        (ids["playlist"], ids["user"]),
    )
    conn.commit()
    conn.close()
    return ids


class _TempLegacyDatabaseTestCase(unittest.TestCase):
    def setUp(self):
        fd, self.path = tempfile.mkstemp(suffix=".db")
        os.close(fd)
        self.ids = _seed(self.path)

    def tearDown(self):
        try:
            os.remove(self.path)
        except OSError:
            pass

    def _rows(self, sql, params=()):
        conn = sqlite3.connect(self.path)
        try:
            return conn.execute(sql, params).fetchall()
        finally:
            conn.close()


class LegacyMigrationImportTestCase(unittest.TestCase):
    def _assert_engine_contract(self, provider, date):
        module = _legacy_module(provider, date)
        self.assertTrue(callable(getattr(module, "apply", None)), f"{provider}/{date}")
        self.assertFalse(hasattr(module, "parser"), f"{provider}/{date}")
        self.assertNotIn("argparse", inspect.getsource(module), f"{provider}/{date}")

    def test_the_sqlite_scripts_import_without_parsing_argv(self):
        for date in ("20171230", "20180317"):
            with self.subTest(date=date):
                self._assert_engine_contract("sqlite", date)

    @unittest.skipIf(_driver_missing("mysql"), "no MySQL driver installed")
    def test_the_mysql_script_imports_without_parsing_argv(self):
        self._assert_engine_contract("mysql", "20171230")

    @unittest.skipIf(_driver_missing("postgres"), "no psycopg2 installed")
    def test_the_postgres_script_imports_without_parsing_argv(self):
        self._assert_engine_contract("postgres", "20180317")


class SQLiteIdConversionTestCase(_TempLegacyDatabaseTestCase):
    def test_hex_ids_become_blobs_and_null_ids_are_kept(self):
        before = self._rows("SELECT COUNT(*) FROM track")[0][0]
        module = _legacy_module("sqlite", "20171230")

        module.apply({"database": self.path})

        self.assertEqual(self._rows("SELECT COUNT(*) FROM track")[0][0], before)
        for table, column in (
            ("folder", "id"),
            ("artist", "id"),
            ("album", "id"),
            ("track", "id"),
            ('"user"', "id"),
            ("chat_message", "id"),
            ("playlist", "id"),
        ):
            texts = self._rows(
                f"SELECT COUNT(*) FROM {table} WHERE typeof({column}) != 'blob'"
            )[0][0]
            self.assertEqual(texts, 0, f"{table}.{column} left as text")

        root = self._rows("SELECT id, parent_id FROM folder WHERE path = '/music'")[0]
        self.assertEqual(root[0], UUID(self.ids["root"]).bytes)
        self.assertIsNone(root[1])

        child = self._rows("SELECT id, parent_id FROM folder WHERE path = '/music/sub'")[0]
        self.assertEqual(child[1], UUID(self.ids["root"]).bytes)

        track = self._rows(
            "SELECT id, album_id, artist_id, root_folder_id, folder_id, path FROM track"
        )[0]
        self.assertEqual(
            track[:5],
            (
                UUID(self.ids["track"]).bytes,
                UUID(self.ids["album"]).bytes,
                UUID(self.ids["artist"]).bytes,
                UUID(self.ids["root"]).bytes,
                UUID(self.ids["folder"]).bytes,
            ),
        )
        self.assertEqual(track[5], "/music/sub/one.flac")

        user = self._rows('SELECT id, last_play_id FROM "user"')[0]
        self.assertEqual(user[0], UUID(self.ids["user"]).bytes)
        self.assertIsNone(user[1])

        starred = self._rows(
            "SELECT starred_id FROM starred_artist WHERE user_id = ?",
            (UUID(self.ids["user"]).bytes,),
        )
        self.assertEqual(starred, [(None,)])

    def test_a_second_run_leaves_the_converted_rows_alone(self):
        module = _legacy_module("sqlite", "20171230")
        module.apply({"database": self.path})
        snapshot = self._rows("SELECT id, parent_id FROM folder ORDER BY path")

        module.apply({"database": self.path})

        self.assertEqual(
            self._rows("SELECT id, parent_id FROM folder ORDER BY path"), snapshot
        )


class SQLitePathHashTestCase(_TempLegacyDatabaseTestCase):
    def test_the_two_2018_scripts_run_in_order_and_hash_every_path(self):
        _legacy_module("sqlite", "20171230").apply({"database": self.path})
        _legacy_module("sqlite", "20180317").apply({"database": self.path})

        for table in ("folder", "track"):
            rows = self._rows(f"SELECT path, path_hash FROM {table}")
            self.assertEqual(len(rows), 2 if table == "folder" else 1)
            for path, path_hash in rows:
                self.assertEqual(
                    bytes(path_hash),
                    hashlib.sha1(path.encode("utf-8"), usedforsecurity=False).digest(),
                    f"{table}.{path}",
                )
            indexes = {
                row[1] for row in self._rows(f"PRAGMA index_list('{table}')")
            }
            self.assertIn(f"index_{table}_path", indexes)
