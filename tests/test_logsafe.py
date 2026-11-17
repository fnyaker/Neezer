# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""What a client's text may do once it reaches a log line: nothing.

``logging`` writes its arguments verbatim, so a value the client chose — a path
segment, a search id, the username of a failed login — carrying a newline
forges an entry of its choosing (CWE-117), and one carrying ESC drives the
terminal that reads the file: it can erase the screen or rewrite the very line
it sits on. ``neezer/logsafe`` escapes both, once, in the record factory.
"""

import io
import logging
import os
import shutil
import tempfile
import unittest

from neezer import logsafe
from neezer.config import DefaultConfig
from neezer.db import release_database
from neezer.managers.user import UserManager
from neezer.web import create_application


def _logged(name, *args):
    """The text a handler receives for one log call on ``name``."""
    logsafe.install()
    buf = io.StringIO()
    handler = logging.StreamHandler(buf)
    logger = logging.getLogger(name)
    logger.addHandler(handler)
    level = logger.level
    logger.setLevel(logging.INFO)
    try:
        logger.info(*args)
    finally:
        logger.removeHandler(handler)
        logger.setLevel(level)
    return buf.getvalue()


class LogSafeTestCase(unittest.TestCase):
    """Every control character, and only those, is rewritten."""

    def test_a_newline_cannot_start_a_forged_entry(self):
        out = _logged("neezer.webui", "login %s", "bob\nERROR forged")
        self.assertEqual("login bob\\nERROR forged\n", out)

    def test_an_escape_sequence_cannot_repaint_the_terminal(self):
        # ESC [ 2 J clears the screen, ESC [ 1 ; 31 m turns the rest red: a
        # reader would see neither the entry nor the one before it.
        out = _logged("neezer.webui", "login %s", "bob\x1b[2J\x1b[1;31mforged")
        self.assertNotIn("\x1b", out)
        self.assertIn("bob\\x1b[2J\\x1b[1;31mforged", out)

    def test_nul_del_and_c1_are_escaped(self):
        out = _logged("neezer.webui", "%s", "a\x00b\x7fc\x9bd\x85e")
        self.assertEqual("a\\x00b\\x7fc\\x9bd\\x85e\n", out)

    def test_unicode_line_separators_are_escaped(self):
        out = _logged("neezer.webui", "%s", "a\u2028b\u2029c")
        self.assertEqual("a\\u2028b\\u2029c\n", out)

    def test_ordinary_text_is_left_alone(self):
        # Accents, quotes, percent signs and tags are what a real title holds.
        for value in ("Élodie — L'été", "100% <b>rock</b>", "", "42"):
            self.assertEqual(value + "\n", _logged("neezer.webui", "%s", value))

    def test_an_exception_message_is_cleaned_too(self):
        out = _logged("neezer", "%s", ValueError("a\r\nb\x1b[2Jc"))
        self.assertEqual("a\\r\\nb\\x1b[2Jc\n", out)


class FailedLoginLogTestCase(unittest.TestCase):
    """The hostile value, end to end, through the route that logs it."""

    def setUp(self):
        self._db = tempfile.mkstemp()
        self._dir = tempfile.mkdtemp()
        db_path, cache = self._db[1], self._dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )

        self.app = create_application(Config())
        UserManager.add("alice", "Alic3", admin=True)
        self.client = self.app.test_client()

    def tearDown(self):
        release_database()
        shutil.rmtree(self._dir, ignore_errors=True)
        os.close(self._db[0])
        os.remove(self._db[1])

    def test_a_hostile_username_reaches_the_log_inert(self):
        forged = (
            "mallory\n"
            "2026-01-01 00:00:00,000 ERROR login succeeded for admin from 127.0.0.1"
            "\x1b[2J"
        )
        with self.assertLogs("neezer.webui", level="ERROR") as captured:
            response = self.client.post(
                "/api/login", json={"username": forged, "password": "nope"}
            )
        self.assertEqual(401, response.status_code)

        message = next(
            record.getMessage()
            for record in captured.records
            if "Failed web login" in record.getMessage()
        )
        # One line, and no escape the terminal would obey.
        self.assertNotIn("\n", message)
        self.assertNotIn("\x1b", message)
        self.assertIn("\\n2026-01-01 00:00:00,000 ERROR login succeeded", message)
