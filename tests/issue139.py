# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2019-2022 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import shutil
import tempfile
import unittest

from neezer.db import init_database, release_database
from neezer.managers.folder import FolderManager
from neezer.scanner import Scanner


class Issue139TestCase(unittest.TestCase):
    def setUp(self):
        self.__dir = tempfile.mkdtemp()
        init_database("sqlite:")
        FolderManager.add("folder", self.__dir)

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir)

    def do_scan(self):
        scanner = Scanner()
        scanner.queue_folder("folder")
        scanner.run()

    def test_null_genre(self):
        shutil.copy("tests/assets/issue139.mp3", self.__dir)
        self.do_scan()

    def test_float_bitrate(self):
        shutil.copy("tests/assets/issue139.aac", self.__dir)
        self.do_scan()


if __name__ == "__main__":
    unittest.main()
