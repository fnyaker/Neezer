# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2020-2022 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import os
import os.path
import shutil
import sys
import tempfile
import unittest

from neezer.db import init_database, release_database
from neezer.managers.folder import FolderManager
from neezer.scanner import Scanner


@unittest.skipIf(
    sys.platform == "win32", "Windows doesn't seem too allow badly encoded paths"
)
class Issue85TestCase(unittest.TestCase):
    def setUp(self):
        self.__dir = tempfile.mkdtemp()
        init_database("sqlite:")
        FolderManager.add("folder", self.__dir)

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir)

    def test_issue(self):
        os.mkdir(os.path.join(self.__dir.encode(), b"\xe6"))
        shutil.copyfile(
            "tests/assets/folder/silence.mp3",
            os.path.join(self.__dir.encode(), b"\xe6", b"silence.mp3"),
        )

        scanner = Scanner()
        scanner.queue_folder("folder")
        scanner.run()


if __name__ == "__main__":
    unittest.main()
