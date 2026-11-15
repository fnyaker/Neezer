# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2018 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import os
import unittest
import shutil
import tempfile
from unittest import mock

from neezer import utils
from neezer.db import init_database, release_database
from neezer.web import create_application

from ..testbase import TestConfig


class SecretTestCase(unittest.TestCase):
    def setUp(self):
        self.__dir = tempfile.mkdtemp()
        self.config = TestConfig(False, False)
        self.config.BASE["database_uri"] = "sqlite://"
        self.config.WEBAPP["cache_dir"] = self.__dir

        init_database(self.config.BASE["database_uri"])
        release_database()

    def tearDown(self):
        shutil.rmtree(self.__dir)

    def test_key(self):
        app1 = create_application(self.config)
        release_database()

        app2 = create_application(self.config)
        release_database()

        self.assertEqual(app1.secret_key, app2.secret_key)

    def test_secrets_env_only_refuses_to_boot_without_the_invite_secret(self):
        env = {
            "NEEZER_SECRETS_ENV_ONLY": "1",
            "NEEZER_SECRET_COOKIES_SECRET": "cookies-for-the-test",
            "NEEZER_SECRET_PASSWORD_SECRET": "passwords-for-the-test",
            "NEEZER_SECRET_DAEMON_KEY": "daemon-for-the-test",
        }
        with mock.patch.dict(os.environ, env, clear=False):
            os.environ.pop("NEEZER_SECRET_INVITE_SECRET", None)
            os.environ.pop("SUPYSONIC_SECRET_INVITE_SECRET", None)
            utils.__dict__["__key_cache"].clear()
            try:
                with self.assertRaises(RuntimeError) as raised:
                    create_application(self.config)
            finally:
                utils.__dict__["__key_cache"].clear()
                release_database()
        self.assertIn("NEEZER_SECRET_INVITE_SECRET", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
