# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The LAN lot as an UPGRADE (rule 7), on SQLite.

Three ways a schema change breaks a running server, each pinned here: a server
whose database is a version behind never gets the tables; a migration that is
replayed (the boot repair does replay it, and a database a branch moved past it
gets it late) must be a no-op; and a database marked current but missing a table
must be repaired at boot instead of answering 500 on every LAN request.

The same lot runs on a REAL PostgreSQL and MySQL in `tests.yaml`'s database job
(`tests/test_schema_external.py`, which carries the 20261124 and 20261125 lots in
its replay list and the four tables in its expected shape).
"""

import importlib
import os
import shutil
import tempfile
import unittest
from datetime import datetime, timedelta

from neezer import db as dbmod
from neezer import lan
from neezer.db import (
    LanDevice,
    LanDeviceKey,
    LanFriend,
    LanFriendPlaylist,
    Meta,
    Playlist,
    SCHEMA_VERSION,
    User,
    init_database,
    release_database,
)
from neezer.managers.user import UserManager

ROOT = os.path.join(os.path.expanduser("~"), ".cache", "neezer-lan-tests")
LAN_TABLES = ("lan_friend_playlist", "lan_friend", "lan_device_key", "lan_device")


class LanUpgradeCase(unittest.TestCase):
    def setUp(self):
        os.makedirs(ROOT, exist_ok=True)
        self.directory = tempfile.mkdtemp(prefix="upgrade-", dir=ROOT)
        self.path = os.path.join(self.directory, "neezer.db")
        self.uri = "sqlite:///" + self.path
        vars(__import__("neezer.utils", fromlist=["utils"]))["__key_cache"].clear()
        lan._reset_for_tests()

    def tearDown(self):
        release_database()
        lan._reset_for_tests()
        shutil.rmtree(self.directory, ignore_errors=True)

    def _sql(self, *statements):
        import sqlite3

        connection = sqlite3.connect(self.path)
        try:
            for statement in statements:
                connection.execute(statement)
            connection.commit()
        finally:
            connection.close()

    def _tables(self):
        import sqlite3

        connection = sqlite3.connect(self.path)
        try:
            return {
                row[0]
                for row in connection.execute(
                    "SELECT name FROM sqlite_master WHERE type = 'table'"
                )
            }
        finally:
            connection.close()

    def _columns(self, table):
        import sqlite3

        connection = sqlite3.connect(self.path)
        try:
            return {row[1] for row in connection.execute(f"PRAGMA table_info({table})")}
        finally:
            connection.close()

    def _seed(self):
        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb")
        alice = User.get(User.name == "alice")
        bob = User.get(User.name == "bob")
        playlist = Playlist.create(user=alice, name="Route")
        return alice, bob, playlist


class UpgradeFromThePreviousVersionTestCase(LanUpgradeCase):
    def test_a_server_one_date_behind_gains_the_shape_and_keeps_its_rows(self):
        alice, bob, playlist = self._seed()
        users_before = {
            user.name: (user.password, user.salt, user.admin, user.name_key)
            for user in User.select()
        }
        release_database()

        # The state a 20261123 server is in: no LAN table, no roster counter.
        self._sql(*[f"DROP TABLE IF EXISTS {table}" for table in LAN_TABLES])
        self._sql("ALTER TABLE user DROP COLUMN lan_serial")
        self._sql("UPDATE meta SET value = '20261123' WHERE key = 'schema_version'")
        self.assertNotIn("lan_device", self._tables())
        self.assertNotIn("lan_serial", self._columns("user"))

        init_database(self.uri)

        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        self.assertEqual(SCHEMA_VERSION, "20261125")
        self.assertEqual(dbmod._missing_columns(), [])
        for table in LAN_TABLES:
            self.assertIn(table, self._tables())
        self.assertIn("lan_serial", self._columns("user"))
        self.assertEqual(
            {
                user.name: (user.password, user.salt, user.admin, user.name_key)
                for user in User.select()
            },
            users_before,
        )
        self.assertEqual(Playlist.select().count(), 1)
        # The counter starts at 0 for every account an upgrade carried, and the
        # four tables are empty: an upgrade invents no device and no friendship.
        self.assertEqual([user.lan_serial for user in User.select()], [0, 0])
        for model in (LanDevice, LanDeviceKey, LanFriend, LanFriendPlaylist):
            self.assertEqual(model.select().count(), 0)
        # The feature works on the upgraded database.
        self.assertEqual(len(lan.subject_bytes(alice)), 16)
        self.assertTrue(lan.enabled())
        self.assertEqual(lan.credential_days(), 30)


class ReplayTestCase(LanUpgradeCase):
    def test_replaying_the_migration_leaves_every_row_alone(self):
        alice, bob, playlist = self._seed()
        release_database()
        module = importlib.import_module("neezer.schema.migration.sqlite.20261124")
        # A populated LAN shape, so the replay has something to preserve.
        init_database(self.uri)
        device = LanDevice.create(
            id="0123456789abcdef01234567",
            user=User.get(User.name == "alice"),
            pubkey="a" * 64,
            name="PC",
            platform="desktop",
            not_after=datetime.now() + timedelta(days=30),
        )
        LanDeviceKey.create(pubkey="a" * 64, device=device, not_after=device.not_after)
        LanFriend.create(
            owner=User.get(User.name == "alice"),
            friend=User.get(User.name == "bob"),
            share_all=True,
        )
        LanFriendPlaylist.create(
            owner=User.get(User.name == "alice"),
            friend=User.get(User.name == "bob"),
            playlist=playlist,
        )
        for _ in range(3):
            module.apply({"database": self.path})
        self.assertEqual(LanDevice.select().count(), 1)
        self.assertEqual(LanDeviceKey.select().count(), 1)
        self.assertEqual(LanFriend.select().count(), 1)
        self.assertEqual(LanFriendPlaylist.select().count(), 1)
        self.assertEqual(dbmod._missing_columns(), [])


class BootRepairTestCase(LanUpgradeCase):
    def test_a_current_database_missing_the_lan_shape_is_repaired_at_boot(self):
        self._seed()
        init_database(self.uri)
        LanDevice.create(
            id="0123456789abcdef01234567",
            user=User.get(User.name == "alice"),
            pubkey="a" * 64,
            name="PC",
            platform="desktop",
            not_after=datetime.now() + timedelta(days=30),
        )
        release_database()
        # The broken state: the version says current, the tables are gone.
        self._sql("DROP TABLE lan_device_key", "DROP TABLE lan_friend_playlist")
        self._sql("ALTER TABLE user DROP COLUMN lan_serial")
        self.assertNotIn("lan_device_key", self._tables())

        with self.assertLogs("neezer.db", level="WARNING"):
            init_database(self.uri)

        self.assertEqual(dbmod._missing_columns(), [])
        self.assertIn("lan_device_key", self._tables())
        self.assertIn("lan_friend_playlist", self._tables())
        self.assertIn("lan_serial", self._columns("user"))
        # The device row survived the repair.
        self.assertEqual(LanDevice.select().count(), 1)
        self.assertEqual(
            User.get(User.name == "alice").lan_serial, 0
        )


class FreshInstallTestCase(LanUpgradeCase):
    def test_a_fresh_database_has_the_tables_without_any_migration(self):
        init_database(self.uri)
        self.assertEqual(Meta["schema_version"].value, SCHEMA_VERSION)
        for table in LAN_TABLES:
            self.assertIn(table, self._tables())
        self.assertIn("lan_serial", self._columns("user"))
        self.assertEqual(dbmod._missing_columns(), [])
        # The unique key the enrolment relies on is the engine's own.
        from peewee import IntegrityError

        alice = UserManager.add("alice", "Alic3", admin=True)
        LanDevice.create(
            id="0123456789abcdef01234567",
            user=alice,
            pubkey="a" * 64,
            name="PC",
            platform="desktop",
            not_after=datetime.now() + timedelta(days=30),
        )
        with self.assertRaises(IntegrityError):
            LanDevice.create(
                id="abcdefabcdefabcdefabcdef",
                user=alice,
                pubkey="a" * 64,
                name="Autre",
                platform="desktop",
                not_after=datetime.now() + timedelta(days=30),
            )


if __name__ == "__main__":
    unittest.main()
