# This file is part of Neezer.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The Content-Type of a transcode must not depend on the host's mime tables.

A slim image ships no /etc/mime.types (the Dockerfile installs the codec tools,
not `media-types`), and CPython's built-in table has no `.ogg` on its own: an
Opus transcode used to be answered as `application/octet-stream` there, which
players that trust the header refuse. The targets are a closed set, so
`neezer.media_types` answers them without asking the host.
"""

import mimetypes
import unittest

from neezer.db import (
    Album,
    Artist,
    ClientPrefs,
    Folder,
    PodcastChannel,
    PodcastEpisode,
    Track,
    User,
)
from neezer.managers.folder import FolderManager
from neezer.scanner import Scanner

from .apitestbase import ApiTestBase

#: Targets the transcoders write; a host without /etc/mime.types knows none.
_TARGET_EXTENSIONS = (".ogg", ".opus", ".oga", ".mp3", ".m4a", ".mp4", ".flac", ".aac")


class _NoHostMimeTable:
    """Hide the host's entries for the audio targets, then put them back."""

    def __enter__(self):
        # Fill the interpreter's table first (it is populated lazily, on first
        # use), so popping really simulates an image without /etc/mime.types.
        mimetypes.init()
        self.saved = []
        for table in (mimetypes.types_map, mimetypes.common_types):
            for ext in _TARGET_EXTENSIONS:
                if ext in table:
                    self.saved.append((table, ext, table.pop(ext)))
        return self

    def __exit__(self, *exc):
        for table, ext, value in self.saved:
            table[ext] = value
        return False


class MimeTableTestCase(unittest.TestCase):
    def test_known_transcode_targets_answer_without_the_host(self):
        from neezer.media_types import transcode_mimetype

        with _NoHostMimeTable():
            self.assertEqual(transcode_mimetype("ogg"), "audio/ogg")
            self.assertEqual(transcode_mimetype(".opus"), "audio/ogg")
            self.assertEqual(transcode_mimetype("mp3"), "audio/mpeg")
            self.assertEqual(transcode_mimetype("m4a"), "audio/mp4")
            self.assertEqual(transcode_mimetype("flac"), "audio/flac")

    def test_an_unknown_target_falls_back_instead_of_crashing(self):
        from neezer.media_types import transcode_mimetype

        self.assertEqual(transcode_mimetype("zzz"), "application/octet-stream")
        self.assertEqual(transcode_mimetype(None), "application/octet-stream")


class TranscodedContentTypeTestCase(ApiTestBase):
    """`as_subsonic_child`'s `transcodedContentType`, with no host mime table.

    `neezer/api/media.py` answers a stream's Content-Type from
    `neezer.media_types` (PR #280); `neezer/db.py` announces the same value
    about a track or an episode, so it must not read the host's tables either.
    """

    def setUp(self):
        super().setUp()
        Folder.create(name="Root", root=True, path="tests")
        Folder.create(
            name="Folder",
            root=False,
            path="tests/assets",
            parent=Folder.get(name="Root"),
        )
        artist = Artist.create(name="Artist")
        album = Album.create(name="Album", artist=artist)
        self.track = Track.create(
            title="Track",
            album=album,
            artist=artist,
            disc=1,
            number=1,
            path="tests/assets/empty",
            folder=Folder.get(name="Folder"),
            root_folder=Folder.get(name="Root"),
            duration=2,
            bitrate=320,
            last_modification=0,
        )
        self.user = User.get(name="alice")
        channel = PodcastChannel.create(
            user=self.user, url="https://example.test/feed"
        )
        self.episode = PodcastEpisode.create(
            channel=channel,
            title="Episode",
            path="episode.mp3",
            duration=60,
            status="completed",
        )
        # What a client that asked for ogg carries (`request.client`).
        self.prefs = ClientPrefs.create(
            user=self.user, client_name="tests", format="ogg"
        )

    def test_a_song_announces_audio_ogg_without_a_host_mime_table(self):
        with _NoHostMimeTable():
            info = self.track.as_subsonic_child(self.user, self.prefs)
        self.assertEqual(info["transcodedSuffix"], "ogg")
        self.assertEqual(info["transcodedContentType"], "audio/ogg")

    def test_a_podcast_episode_announces_audio_ogg_without_a_host_mime_table(self):
        with _NoHostMimeTable():
            info = self.episode.as_subsonic_episode(self.user, self.prefs)
        self.assertEqual(info["transcodedSuffix"], "ogg")
        self.assertEqual(info["transcodedContentType"], "audio/ogg")


class TranscodeContentTypeTestCase(ApiTestBase):
    """GET /rest/stream?format=ogg, with no `.ogg` anywhere on the host."""

    def setUp(self):
        super().setUp()
        FolderManager.add("Folder", "tests/assets/folder")
        scanner = Scanner()
        scanner.queue_folder("Folder")
        scanner.run()
        self.trackid = Track.get().id
        with self.app_context():
            from flask import current_app

            # `cat` is enough: the assertion is about the announced type.
            current_app.config["TRANSCODING"]["transcoder_mp3_ogg"] = "cat %srcpath"

    def _stream(self, fmt):
        return self.client.get(
            "/rest/stream.view",
            query_string={
                "u": "alice",
                "p": "Alic3",
                "c": "tests",
                "v": "1.9.0",
                "id": self.trackid,
                "format": fmt,
            },
        )

    def test_an_ogg_transcode_is_audio_ogg_without_a_host_mime_table(self):
        with _NoHostMimeTable():
            rv = self._stream("ogg")
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.mimetype, "audio/ogg")
        self.assertTrue(rv.get_data(), "the transcoder output is served")


if __name__ == "__main__":
    unittest.main()
