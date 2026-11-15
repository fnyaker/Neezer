# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2017-2022 Alban 'spl0k' Féron
#
# Distributed under terms of the GNU AGPLv3 license.

import collections
import contextlib
import unittest
import uuid

import peewee

from neezer.db import (
    Album,
    Artist,
    Folder,
    Playlist,
    PlaylistMember,
    PlaylistTrack,
    RatingTrack,
    StarredTrack,
    Track,
    User,
    db,
)

from .apitestbase import ApiTestBase


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql

    def execute_sql(self, sql, *args, **kwargs):
        counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


class PlaylistTestCase(ApiTestBase):
    def setUp(self):
        super().setUp()

        root = Folder.create(root=True, name="Root folder", path="tests/assets")
        artist = Artist.create(name="Artist")
        album = Album.create(name="Album", artist=artist)

        songs = {}
        for num, song in enumerate(["One", "Two", "Three", "Four"]):
            track = Track.create(
                disc=1,
                number=num,
                title=song,
                duration=2,
                album=album,
                artist=artist,
                bitrate=320,
                path="tests/assets/" + song,
                last_modification=0,
                root_folder=root,
                folder=root,
            )
            songs[song] = track

        users = {u.name: u for u in User.select()}

        playlist = Playlist.create(user=users["alice"], name="Alice's")
        playlist.add(songs["One"])
        playlist.add(songs["Three"])
        playlist.save()

        playlist = Playlist.create(
            user=users["alice"], public=True, name="Alice's public"
        )
        playlist.add(songs["One"])
        playlist.add(songs["Two"])
        playlist.save()

        playlist = Playlist.create(user=users["bob"], name="Bob's")
        playlist.add(songs["Two"])
        playlist.add(songs["Four"])
        playlist.save()

    def test_get_playlists(self):
        # get own playlists
        rv, child = self._make_request("getPlaylists", tag="playlists")
        self.assertEqual(len(child), 2)
        self.assertEqual(child[0].get("owner"), "alice")
        self.assertEqual(child[1].get("owner"), "alice")

        # get own and public
        rv, child = self._make_request(
            "getPlaylists", {"u": "bob", "p": "B0b"}, tag="playlists"
        )
        self.assertEqual(len(child), 2)
        self.assertTrue("alice" in (child[0].get("owner"), child[1].get("owner")))
        self.assertTrue("bob" in (child[0].get("owner"), child[1].get("owner")))
        self.assertIsNotNone(
            self._find(child, "./playlist[@owner='alice'][@public='true']")
        )

        # get other
        rv, child = self._make_request(
            "getPlaylists", {"username": "bob"}, tag="playlists"
        )
        self.assertEqual(len(child), 1)
        self.assertEqual(child[0].get("owner"), "bob")

        # get other when not admin
        self._make_request(
            "getPlaylists", {"u": "bob", "p": "B0b", "username": "alice"}, error=50
        )

        # get from unknown user
        self._make_request("getPlaylists", {"username": "johndoe"}, error=70)

    def test_get_playlist(self):
        # missing param
        self._make_request("getPlaylist", error=10)

        # invalid id
        self._make_request("getPlaylist", {"id": 1234}, error=0)

        # unknown
        self._make_request("getPlaylist", {"id": str(uuid.uuid4())}, error=70)

        # other's private from non admin
        playlist = (
            Playlist.select()
            .join(User)
            .where(~Playlist.public, User.name == "alice")
            .get()
        )
        self._make_request(
            "getPlaylist", {"u": "bob", "p": "B0b", "id": str(playlist.id)}, error=50
        )

        # standard
        rv, child = self._make_request("getPlaylists", tag="playlists")
        self._make_request("getPlaylist", {"id": child[0].get("id")}, tag="playlist")
        rv, child = self._make_request(
            "getPlaylist", {"id": child[1].get("id")}, tag="playlist"
        )
        self.assertEqual(child.get("songCount"), "2")
        self.assertEqual(
            self._xpath(child, "count(./entry)"), 2
        )  # don't count children, there may be 'allowedUser's (even though not supported by neezer)
        self.assertEqual(child.get("duration"), "4")
        self.assertEqual(child[0].get("title"), "One")
        self.assertTrue(
            child[1].get("title") in ("Two", "Three")
        )  # depending on 'getPlaylists' result ordering

    def test_adding_a_track_to_a_playlist_archives_it(self):
        """Putting a track in a playlist puts it in your library, whichever
        client did it — so it gets archived there and then, not on the day you
        happen to play it."""
        queued = []

        class Prefetch:
            def download_ids(self, ids):
                ids = list(ids)
                queued.extend(ids)
                return len(ids)

        self.app_context().app.deezer_prefetch = Prefetch()
        track = Track.get(title="One")
        track.deezer_id = "424242"
        track.path = "tests/assets/not-archived-yet"  # no file: needs fetching
        track.save()

        self._make_request(
            "createPlaylist",
            {"name": "Archiving", "songId": str(track.id)},
            skip_post=True,
        )
        self.assertEqual(queued, ["424242"])

        # …and so does adding it to an existing one.
        queued.clear()
        pid = str(Playlist.get(name="Archiving").id)
        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIdToAdd": str(track.id)},
            skip_post=True,
        )
        self.assertEqual(queued, ["424242"])

        # Removing one archives nothing (and never deletes the file).
        queued.clear()
        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": "0"},
            skip_post=True,
        )
        self.assertEqual(queued, [])

    def test_create_playlist(self):
        self._make_request("createPlaylist", error=10)
        self._make_request(
            "createPlaylist", {"name": "wrongId", "songId": "abc"}, error=0
        )
        self._make_request(
            "createPlaylist",
            {"name": "unknownId", "songId": str(uuid.uuid4())},
            error=70,
        )
        self._make_request("createPlaylist", {"playlistId": "abc"}, error=0)
        self._make_request(
            "createPlaylist", {"playlistId": str(uuid.uuid4())}, error=70
        )

        # create
        self._make_request("createPlaylist", {"name": "new playlist"}, skip_post=True)
        rv, child = self._make_request("getPlaylists", tag="playlists")
        self.assertEqual(len(child), 3)
        playlist = self._find(child, "./playlist[@name='new playlist']")
        self.assertEqual(len(playlist), 0)

        # "update" newly created
        self._make_request("createPlaylist", {"playlistId": playlist.get("id")})
        rv, child = self._make_request("getPlaylists", tag="playlists")
        self.assertEqual(len(child), 3)

        # renaming
        self._make_request(
            "createPlaylist", {"playlistId": playlist.get("id"), "name": "renamed"}
        )
        rv, child = self._make_request("getPlaylists", tag="playlists")
        self.assertEqual(len(child), 3)
        self.assertIsNone(self._find(child, "./playlist[@name='new playlist']"))
        playlist = self._find(child, "./playlist[@name='renamed']")
        self.assertIsNotNone(playlist)

        # update from other user
        self._make_request(
            "createPlaylist",
            {"u": "bob", "p": "B0b", "playlistId": playlist.get("id")},
            error=50,
        )

        # create more useful playlist
        songs = {s.title: str(s.id) for s in Track.select()}
        self._make_request(
            "createPlaylist",
            {
                "name": "songs",
                "songId": [songs[s] for s in ("Three", "One", "Two")],
            },
            skip_post=True,
        )
        playlist = Playlist.get(name="songs")
        self.assertIsNotNone(playlist)
        rv, child = self._make_request(
            "getPlaylist", {"id": str(playlist.id)}, tag="playlist"
        )
        self.assertEqual(child.get("songCount"), "3")
        self.assertEqual(self._xpath(child, "count(./entry)"), 3)
        self.assertEqual(child[0].get("title"), "Three")
        self.assertEqual(child[1].get("title"), "One")
        self.assertEqual(child[2].get("title"), "Two")

        # update
        self._make_request(
            "createPlaylist",
            {"playlistId": str(playlist.id), "songId": songs["Two"]},
            skip_post=True,
        )
        rv, child = self._make_request(
            "getPlaylist", {"id": str(playlist.id)}, tag="playlist"
        )
        self.assertEqual(child.get("songCount"), "1")
        self.assertEqual(self._xpath(child, "count(./entry)"), 1)
        self.assertEqual(child[0].get("title"), "Two")

    def assertPlaylistCountEqual(self, count):
        self.assertEqual(Playlist.select().count(), count)

    def test_delete_playlist(self):
        # check params
        self._make_request("deletePlaylist", error=10)
        self._make_request("deletePlaylist", {"id": "string"}, error=0)
        self._make_request("deletePlaylist", {"id": str(uuid.uuid4())}, error=70)

        # delete unowned when not admin
        playlist = Playlist.select().join(User).where(User.name == "alice").first()
        self._make_request(
            "deletePlaylist", {"u": "bob", "p": "B0b", "id": str(playlist.id)}, error=50
        )
        self.assertPlaylistCountEqual(3)

        # delete owned
        self._make_request("deletePlaylist", {"id": str(playlist.id)}, skip_post=True)
        self.assertPlaylistCountEqual(2)
        self._make_request("deletePlaylist", {"id": str(playlist.id)}, error=70)
        self.assertPlaylistCountEqual(2)

        # delete unowned when admin
        playlist = Playlist.select().join(User).where(User.name == "bob").get()
        self._make_request("deletePlaylist", {"id": str(playlist.id)}, skip_post=True)
        self.assertPlaylistCountEqual(1)

    def test_update_playlist(self):
        self._make_request("updatePlaylist", error=10)
        self._make_request("updatePlaylist", {"playlistId": 1234}, error=0)
        self._make_request(
            "updatePlaylist", {"playlistId": str(uuid.uuid4())}, error=70
        )

        playlist = (
            Playlist.select()
            .join(User)
            .where(User.name == "alice")
            .order_by(Playlist.created)
            .first()
        )
        pid = str(playlist.id)
        self._make_request(
            "updatePlaylist", {"playlistId": pid, "songIdToAdd": "string"}, error=0
        )
        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": "string"},
            error=0,
        )

        name = str(playlist.name)
        self._make_request(
            "updatePlaylist",
            {"u": "bob", "p": "B0b", "playlistId": pid, "name": "new name"},
            error=50,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertEqual(child.get("name"), name)
        self.assertEqual(self._xpath(child, "count(./entry)"), 2)

        self._make_request(
            "updatePlaylist", {"playlistId": pid, "name": "new name"}, skip_post=True
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertEqual(child.get("name"), "new name")
        self.assertEqual(self._xpath(child, "count(./entry)"), 2)

        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": [-1, 2]},
            skip_post=True,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertEqual(self._xpath(child, "count(./entry)"), 2)

        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": [0, 2]},
            skip_post=True,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertEqual(self._xpath(child, "count(./entry)"), 1)
        self.assertEqual(self._find(child, "./entry").get("title"), "Three")

        songs = {s.title: str(s.id) for s in Track.select()}

        self._make_request(
            "updatePlaylist",
            {
                "playlistId": pid,
                "songIdToAdd": [songs["One"], songs["Two"], songs["Two"]],
            },
            skip_post=True,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertSequenceEqual(
            self._xpath(child, "./entry/@title"), ["Three", "One", "Two", "Two"]
        )

        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": [2, 1]},
            skip_post=True,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertSequenceEqual(self._xpath(child, "./entry/@title"), ["Three", "Two"])

        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIdToAdd": songs["One"]},
            skip_post=True,
        )
        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIndexToRemove": [1, 1]},
            skip_post=True,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertSequenceEqual(self._xpath(child, "./entry/@title"), ["Three", "One"])

        self._make_request(
            "updatePlaylist",
            {"playlistId": pid, "songIdToAdd": str(uuid.uuid4())},
            error=70,
        )
        rv, child = self._make_request("getPlaylist", {"id": pid}, tag="playlist")
        self.assertEqual(self._xpath(child, "count(./entry)"), 2)

    def _bulk_tracks(self, count):
        """``count`` rows of one album, in one transaction (a big playlist)."""
        root = Folder.get(Folder.root == True)  # noqa: E712 - a column
        artist = Artist.get(Artist.name == "Artist")
        album = Album.get(Album.name == "Album")
        made = []
        with db.atomic():
            for i in range(count):
                made.append(
                    Track.create(
                        disc=1,
                        number=i + 1,
                        title="Bulk %04d" % i,
                        duration=60,
                        album=album,
                        artist=artist,
                        bitrate=320,
                        path="tests/assets/bulk-%d" % i,
                        last_modification=0,
                        root_folder=root,
                        folder=root,
                    )
                )
        return made

    def test_a_playlist_of_4000_titles_is_read_in_a_handful_of_statements(self):
        """One joined read, three primed lookups, then serialization.

        It used to be seven SELECTs per title — folder, album, artist, root
        folder, the caller's star, their rating and the average — so a
        4 000-title playlist answered 28 016 statements (2.65 s on SQLite;
        minutes of round-trips on PostgreSQL). Now 9 statements, 0.33 s, and
        the same JSON: this test pins both the budget and the fields the
        per-row reads used to produce.
        """
        user = User.get(User.name == "alice")
        tracks = self._bulk_tracks(4000)
        with db.atomic():
            playlist = Playlist.create(user=user, name="Quatre mille")
            PlaylistMember.create(playlist=playlist, user=user, role="owner")
            PlaylistTrack.insert_many(
                [
                    {"playlist": playlist, "track": t, "index": i}
                    for i, t in enumerate(tracks)
                ]
            ).execute()
        first, second = tracks[0], tracks[1]
        StarredTrack.create(user=user, starred=first.id, date=first.created)
        RatingTrack.create(user=user, rated=first.id, rating=4)
        RatingTrack.create(user=user, rated=second.id, rating=2)
        other = User.get(User.name == "bob")
        RatingTrack.create(user=other, rated=first.id, rating=2)

        args = {
            "u": "alice",
            "p": "Alic3",
            "v": self.apiVersion,
            "c": "tests",
            "f": "json",
            "id": str(playlist.id),
        }
        with count_statements() as counts:
            rv = self.client.get("/rest/getPlaylist.view", query_string=args)
        self.assertEqual(rv.status_code, 200)
        entries = rv.get_json()["subsonic-response"]["playlist"]["entry"]
        self.assertEqual(len(entries), 4000)
        # The session, the playlist, its owner, the joined rows, the credits
        # and the three primed lookups: no statement per title.
        self.assertLessEqual(sum(counts.values()), 12, counts)
        self.assertEqual([e["title"] for e in entries[:3]],
                         ["Bulk 0000", "Bulk 0001", "Bulk 0002"])
        self.assertEqual(entries[0]["starred"], first.created.isoformat())
        self.assertEqual(entries[0]["userRating"], 4)
        self.assertEqual(entries[0]["averageRating"], 3)
        self.assertNotIn("userRating", entries[2])
        self.assertEqual(entries[0]["artist"], "Artist")
        self.assertEqual(entries[0]["album"], "Album")
        # No file on disk for these rows: the size the old per-row read gave.
        self.assertEqual(entries[0]["size"], -1)
        self.assertEqual(entries[0]["path"], "bulk-0")

    def test_creating_and_filling_a_playlist_of_2000_songs_costs_19_statements(self):
        """Resolving the rows and writing them are both batched now.

        Adding songs read ``MAX(index)`` per title and inserted one row at a
        time: 2 000 titles paid 4 005 statements. One batched resolve (5 IN
        chunks) and seven 300-row INSERTs make it 19, measured on SQLite, and
        the stored order is still 0..n-1.
        """
        tracks = self._bulk_tracks(2000)
        ids = [str(t.id) for t in tracks]
        base = {
            "u": "alice",
            "p": "Alic3",
            "v": self.apiVersion,
            "c": "tests",
            "f": "json",
        }
        with count_statements() as counts:
            rv = self.client.post(
                "/rest/createPlaylist.view",
                data=dict(base, name="Deux mille", songId=ids),
            )
        self.assertEqual(rv.status_code, 200)
        self.assertLessEqual(sum(counts.values()), 25, counts)
        playlist = Playlist.get(name="Deux mille")
        stored = [
            row.index
            for row in PlaylistTrack.select(PlaylistTrack.index)
            .where(PlaylistTrack.playlist == playlist)
            .order_by(PlaylistTrack.index)
        ]
        self.assertEqual(stored, list(range(2000)))

        with count_statements() as counts:
            rv = self.client.post(
                "/rest/updatePlaylist.view",
                data=dict(base, playlistId=str(playlist.id), songIdToAdd=ids),
            )
        self.assertLessEqual(sum(counts.values()), 25, counts)
        stored = [
            row.index
            for row in PlaylistTrack.select(PlaylistTrack.index)
            .where(PlaylistTrack.playlist == playlist)
            .order_by(PlaylistTrack.index)
        ]
        self.assertEqual(stored, list(range(4000)))


if __name__ == "__main__":
    unittest.main()
