# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Config discovery — specifically, which file wins.

The container renders a config from the environment and points at it with
NEEZER_CONFIG. If that file did not override the one baked into the image,
every deployment would silently run on the image's placeholder database
credentials.
"""

import os
import tempfile
import unittest

from neezer.config import IniConfig


class ConfigOverrideTestCase(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()
        self.previous = os.environ.get("NEEZER_CONFIG")
        self.previous_legacy = os.environ.get("SUPYSONIC_CONFIG")

    def tearDown(self):
        for name, value in (
            ("NEEZER_CONFIG", self.previous),
            ("SUPYSONIC_CONFIG", self.previous_legacy),
        ):
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value

    def _write(self, name, body):
        path = os.path.join(self.dir, name)
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(body)
        return path

    def test_the_env_pointed_config_wins_over_the_discovered_ones(self):
        """This is what keeps the app and the daemon from fighting over one
        rendered file: each points at its own, and its own wins."""
        baked = self._write(
            "baked.conf", "[base]\ndatabase_uri = postgresql://u:placeholder@db/x\n"
        )
        mine = self._write(
            "mine.conf", "[base]\ndatabase_uri = postgresql://u:real@db/x\n"
        )
        os.environ["NEEZER_CONFIG"] = mine

        original = IniConfig.common_paths
        IniConfig.common_paths = [baked]
        try:
            config = IniConfig.from_common_locations()
        finally:
            IniConfig.common_paths = original
        self.assertEqual(config.BASE["database_uri"], "postgresql://u:real@db/x")

    def test_the_pre_rebrand_variable_is_still_honoured(self):
        """An upgraded container still exports SUPYSONIC_CONFIG."""
        baked = self._write(
            "baked.conf", "[base]\ndatabase_uri = postgresql://u:placeholder@db/x\n"
        )
        mine = self._write(
            "mine.conf", "[base]\ndatabase_uri = postgresql://u:real@db/x\n"
        )
        os.environ.pop("NEEZER_CONFIG", None)
        os.environ["SUPYSONIC_CONFIG"] = mine

        original = IniConfig.common_paths
        IniConfig.common_paths = [baked]
        try:
            config = IniConfig.from_common_locations()
        finally:
            IniConfig.common_paths = original
        self.assertEqual(config.BASE["database_uri"], "postgresql://u:real@db/x")

    def test_without_the_variable_the_discovered_config_still_applies(self):
        baked = self._write(
            "baked.conf", "[base]\ndatabase_uri = postgresql://u:placeholder@db/x\n"
        )
        os.environ.pop("NEEZER_CONFIG", None)

        original = IniConfig.common_paths
        IniConfig.common_paths = [baked]
        try:
            config = IniConfig.from_common_locations()
        finally:
            IniConfig.common_paths = original
        self.assertEqual(config.BASE["database_uri"], "postgresql://u:placeholder@db/x")

    def test_a_missing_override_is_not_an_error(self):
        """A path that isn't there yet must not stop the process booting."""
        os.environ["NEEZER_CONFIG"] = os.path.join(self.dir, "absent.conf")
        original = IniConfig.common_paths
        IniConfig.common_paths = []
        try:
            config = IniConfig.from_common_locations()
        finally:
            IniConfig.common_paths = original
        self.assertIn("database_uri", config.BASE)  # the built-in default

    def test_a_pre_rebrand_temp_database_is_still_the_default(self):
        """No config at all: the old /tmp/supysonic/supysonic.db wins over a
        fresh /tmp/neezer/neezer.db, which would look like data loss."""
        from unittest import mock

        from neezer import config as config_module

        with tempfile.TemporaryDirectory() as root:
            old = os.path.join(root, "supysonic")
            os.makedirs(old)
            open(os.path.join(old, "supysonic.db"), "wb").close()
            with mock.patch.object(config_module.tempfile, "gettempdir", return_value=root):
                directory, database, _socket = config_module._default_data_dir()
        self.assertEqual((directory, database), (old, "supysonic.db"))

    def test_the_new_temp_database_is_used_on_a_fresh_host(self):
        from unittest import mock

        from neezer import config as config_module

        with tempfile.TemporaryDirectory() as root:
            with mock.patch.object(config_module.tempfile, "gettempdir", return_value=root):
                directory, database, _socket = config_module._default_data_dir()
        self.assertEqual(
            (directory, database), (os.path.join(root, "neezer"), "neezer.db")
        )


if __name__ == "__main__":
    unittest.main()
