# This file is part of Neezer.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Regressions for the code-scanning work in the Subsonic API.

The cover-art test fails on the code it replaces: a folder row whose
``cover_art`` column held a relative path served a file outside the folder.
The log test pins the mechanism behind the false-positive CodeQL log-injection
alerts: the failed-login line goes through the package logger, where
``neezer/logsafe.py`` escapes line breaks (a ``print`` or a hand-written file
would not be covered).
"""

import logging
import os
import os.path
import shutil
import tempfile
import unittest

from PIL import Image

from neezer.db import Folder

from .apitestbase import ApiTestBase
from ..testbase import TestBase


class AuthFailureLogTestCase(TestBase):
    """The failed-login line goes through the package logger.

    ``neezer/logsafe.py`` (installed by ``web.create_application``) escapes line
    breaks in the arguments of every record this package logs, so a user name
    cannot start a line of its own. This pins the call site: an f-string in the
    message, a ``print`` or a hand-written file would bypass the factory.
    """

    __with_api__ = True

    def test_user_name_cannot_forge_a_log_line(self):
        from neezer.api import _record_auth_failure

        records = []

        class Collector(logging.Handler):
            def emit(self, record):
                records.append(record)

        collector = Collector()
        log = logging.getLogger("neezer.api")
        log.addHandler(collector)
        try:
            with self.request_context(
                "/rest/ping.view", environ_base={"REMOTE_ADDR": "203.0.113.9"}
            ):
                _record_auth_failure(
                    "eve\nERROR admin logged in from 10.0.0.1\rFAILED"
                )
        finally:
            log.removeHandler(collector)

        self.assertEqual(len(records), 1)
        line = records[0].getMessage()
        # Exactly one line, and the attempt is still readable as evidence.
        self.assertNotIn("\n", line)
        self.assertNotIn("\r", line)
        self.assertIn("eve\\nERROR admin logged in from 10.0.0.1\\rFAILED", line)


class CoverArtPathTestCase(ApiTestBase):
    def setUp(self):
        super().setUp()
        # One unique parent per test: other sessions run this suite on the same
        # machine, so nothing may share a fixed name under /tmp.
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.music = os.path.join(self.root, "music")
        os.mkdir(self.music)
        self.outside = os.path.join(self.root, "outside.jpg")
        Image.new("RGB", (8, 8), (12, 34, 56)).save(self.outside, "JPEG")

    def _cover_query(self, folder):
        return {
            "u": "alice",
            "p": "Alic3",
            "c": "tests",
            "id": str(folder.id),
        }

    def test_folder_cover_cannot_escape_its_folder(self):
        folder = Folder.create(
            name="Root",
            path=self.music,
            root=True,
            cover_art="../outside.jpg",
        )
        # A relative cover_art used to be joined as-is: this answered with a
        # real image that lives outside the library.
        with self.client.get(
            "/rest/getCoverArt.view", query_string=self._cover_query(folder)
        ) as rv:
            self.assertNotEqual(
                (rv.status_code, rv.mimetype),
                (200, "image/jpeg"),
                "served a cover file from outside the folder",
            )
        self._make_request("getCoverArt", {"id": str(folder.id)}, error=70)

    def test_a_cover_inside_the_folder_is_still_served(self):
        Image.new("RGB", (9, 9), (1, 2, 3)).save(
            os.path.join(self.music, "cover.jpg"), "JPEG"
        )
        folder = Folder.create(
            name="Root", path=self.music, root=True, cover_art="cover.jpg"
        )
        with self.client.get(
            "/rest/getCoverArt.view", query_string=self._cover_query(folder)
        ) as rv:
            self.assertEqual(rv.status_code, 200)
            self.assertEqual(rv.mimetype, "image/jpeg")


if __name__ == "__main__":
    unittest.main()
