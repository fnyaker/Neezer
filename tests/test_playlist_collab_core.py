# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Collaborative playlists, CORE slice (webui routes + playlist_access).

The full matrix belongs to ``tests/test_playlist_collab.py`` (qa). This file
pins what the core lot introduces, with the numbers that justified each rule:

* roles owner/editor/viewer + admin override, non-member 404 vs member 403;
* the two counters: ``version`` (tracks/metadata) and ``members_version``
  (who is on it), compare-and-swap, 409 + counters, legacy version-less writes;
* ``/api/me/playlists`` = own + memberships (never an admin firehose), role and
  folder fields, visible counts, 6 statements at 61 playlists (measured);
* membership routes: invite by exact name, role, remove, leave, caps;
* private uploads filtered from list/count/cover/seeds and preserved (anchored)
  on an editor's reorder and removal.
"""

import json
import threading
import unittest
import uuid

from neezer.db import (
    Album,
    Playlist,
    PlaylistMember,
    PlaylistTrack,
    Track,
    User,
)
from neezer.managers.user import UserManager

from .test_deezer import count_statements
from .test_detach_a import _AppCase


class _CollabCase(_AppCase):
    """One server: alice (admin, the owner), bob, chloe, dave (guests)."""

    def setUp(self):
        super().setUp()
        from neezer.webui import _reset_playlist_limits
        from neezer.webui.playlist_members import _reset_for_tests

        _reset_playlist_limits()
        _reset_for_tests()
        UserManager.add("bob", "B0bb")
        UserManager.add("chloe", "Chl03")
        UserManager.add("dave", "Dav3")
        self.alice = User.get(User.name == "alice")
        self.bob = User.get(User.name == "bob")
        self.chloe = User.get(User.name == "chloe")
        self.dave = User.get(User.name == "dave")
        self.bob_client = self._client("bob", "B0bb")
        self.chloe_client = self._client("chloe", "Chl03")
        self.dave_client = self._client("dave", "Dav3")

    # -- fixtures -----------------------------------------------------------

    def _client(self, name, password):
        client = self.app.test_client()
        rv = client.post("/api/login", json={"username": name, "password": password})
        self.assertEqual(rv.status_code, 200, rv.get_json())
        return client

    def _playlist(self, owner=None, name="Route 66", **kwargs):
        """A playlist created directly (no member row), like older servers."""
        with self.app.app_context():
            return Playlist.create(user=owner or self.alice, name=name, **kwargs)

    def _track(self, sng_id, album="600", title=None):
        """A Deezer-backed Track row with its own album (one cover each)."""
        from neezer.deezer import library

        root = library.get_root_folder(self.archive)
        return library.upsert_track(
            {
                "SNG_ID": str(sng_id),
                "SNG_TITLE": title or f"Song {sng_id}",
                "ART_NAME": f"Artist {album}",
                "ART_ID": f"9{album}",
                "ALB_TITLE": f"Album {album}",
                "ALB_ID": str(album),
                "DURATION": "200",
                "TRACK_NUMBER": "1",
                "DISK_NUMBER": "1",
            },
            root,
        )

    def _fill(self, playlist, tracks):
        with self.app.app_context():
            for index, track in enumerate(tracks):
                PlaylistTrack.create(playlist=playlist, track=track, index=index)
        return playlist

    def _join(self, playlist, user, role):
        with self.app.app_context():
            return PlaylistMember.create(
                playlist=playlist, user=user, role=role
            )

    def _titles(self, client, playlist, as_json=False):
        data = client.get(f"/api/playlist/{playlist.id}").get_json()
        if as_json:
            return data
        return [t["title"] for t in data["tracks"]]

    def _private(self, track, owner):
        track.owner = owner
        track.save()
        return track


class RoleMatrixTestCase(_CollabCase):
    def setUp(self):
        super().setUp()
        self.pl = self._playlist()
        self.t1 = self._track("1", album="600", title="One")
        self.t2 = self._track("2", album="601", title="Two")
        self._fill(self.pl, [self.t1, self.t2])
        self._join(self.pl, self.bob, "editor")
        self._join(self.pl, self.chloe, "viewer")

    def test_the_owner_reads_everything_and_may_manage(self):
        data = self._titles(self.client, self.pl, as_json=True)["playlist"]
        self.assertEqual(data["role"], "owner")
        self.assertTrue(data["editable"])
        self.assertTrue(data["can_manage"])
        self.assertTrue(data["admin"])  # the instance flag, not a role
        # The owner row is synthesized from Playlist.user_id: this playlist was
        # created directly, with no playlist_member row at all.
        self.assertEqual(
            [(m["name"], m["role"]) for m in data["members"]],
            [("alice", "owner"), ("bob", "editor"), ("chloe", "viewer")],
        )
        self.assertEqual((data["version"], data["members_version"]), (1, 1))
        self.assertIsNone(data["last_edited_by"])
        self.assertEqual(data["private_tracks"], 0)

    def test_an_editor_edits_the_tracklist_but_not_the_playlist(self):
        data = self._titles(self.bob_client, self.pl, as_json=True)["playlist"]
        self.assertEqual(data["role"], "editor")
        self.assertTrue(data["editable"])
        self.assertFalse(data["can_manage"])
        # An editor does not get the owner's private-upload count.
        self.assertNotIn("private_tracks", data)

        rv = self.bob_client.patch(f"/api/playlist/{self.pl.id}", json={"title": "Nope"})
        self.assertEqual(rv.status_code, 403)
        self.assertEqual(rv.get_json(), {"error": "forbidden"})
        self.assertEqual(
            self.bob_client.delete(f"/api/playlist/{self.pl.id}").status_code, 403
        )
        self.assertEqual(
            self.bob_client.post(
                f"/api/playlist/{self.pl.id}/members",
                json={"username": "dave", "role": "viewer"},
            ).status_code,
            403,
        )
        # ...but the tracklist is theirs.
        rv = self.bob_client.post(
            f"/api/playlist/{self.pl.id}/tracks", json={"tracks": ["3"]}
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["version"], 2)

    def test_a_viewer_only_reads(self):
        data = self._titles(self.chloe_client, self.pl, as_json=True)["playlist"]
        self.assertEqual(data["role"], "viewer")
        self.assertFalse(data["editable"])
        self.assertFalse(data["can_manage"])
        rv = self.chloe_client.post(
            f"/api/playlist/{self.pl.id}/tracks", json={"tracks": ["3"]}
        )
        self.assertEqual(rv.status_code, 403)
        with self.app.app_context():
            self.assertEqual(PlaylistTrack.select().count(), 2)

    def test_a_non_member_gets_the_same_404_as_an_unknown_id(self):
        foreign = self.dave_client.get(f"/api/playlist/{self.pl.id}")
        unknown = self.dave_client.get(f"/api/playlist/{uuid.uuid4()}")
        self.assertEqual(foreign.status_code, 404)
        self.assertEqual(unknown.status_code, 404)
        self.assertEqual(foreign.get_json(), unknown.get_json())
        # ...and every mutation and read route answers the same way.
        for method, path in (
            ("get", f"/api/playlist/{self.pl.id}/members"),
            ("get", f"/api/playlist/{self.pl.id}/version"),
            ("post", f"/api/playlist/{self.pl.id}/tracks"),
            ("put", f"/api/playlist/{self.pl.id}/order"),
            ("delete", f"/api/playlist/{self.pl.id}/tracks"),
            ("post", f"/api/playlist/{self.pl.id}/leave"),
            ("post", f"/api/playlist/{self.pl.id}/members"),
        ):
            rv = getattr(self.dave_client, method)(path, json={"tracks": ["1"]})
            self.assertEqual(rv.status_code, 404, (method, path, rv.get_json()))

    def test_an_admin_who_is_not_a_member_gets_the_override_not_a_member_row(self):
        # A second playlist owned by bob, read by the admin.
        theirs = self._playlist(owner=self.bob, name="Bob's")
        self._fill(theirs, [self.t1])
        data = self._titles(self.client, theirs, as_json=True)["playlist"]
        self.assertEqual(data["role"], "admin")
        self.assertTrue(data["can_manage"])
        self.assertEqual(
            [m["name"] for m in data["members"]], ["bob"]
        )  # the admin is not listed
        self.assertEqual(
            self.client.patch(f"/api/playlist/{theirs.id}", json={"title": "Fixed"}).status_code,
            200,
        )

    def test_an_admin_who_is_a_member_keeps_that_role_plus_the_override(self):
        theirs = self._playlist(owner=self.bob, name="Bob's")
        self._join(theirs, self.alice, "viewer")
        data = self._titles(self.client, theirs, as_json=True)["playlist"]
        self.assertEqual(data["role"], "viewer")
        self.assertTrue(data["editable"])
        self.assertTrue(data["can_manage"])

    def test_a_public_playlist_is_readable_without_member_names(self):
        with self.app.app_context():
            self.pl.public = True
            self.pl.save(only=(Playlist.public,))
        data = self._titles(self.dave_client, self.pl, as_json=True)["playlist"]
        self.assertEqual(data["role"], "public")
        self.assertFalse(data["editable"])
        self.assertNotIn("members", data)
        self.assertNotIn("last_edited_by", data)
        self.assertNotIn("last_edited_at", data)
        self.assertEqual(self.dave_client.get(f"/api/playlist/{self.pl.id}/version").status_code, 200)
        self.assertEqual(
            self.dave_client.get(f"/api/playlist/{self.pl.id}/members").status_code, 404
        )
        self.assertEqual(
            self.dave_client.post(
                f"/api/playlist/{self.pl.id}/tracks", json={"tracks": ["1"]}
            ).status_code,
            403,
        )

    def test_members_and_version_are_small_and_readable_by_a_viewer(self):
        rv = self.chloe_client.get(f"/api/playlist/{self.pl.id}/members")
        self.assertEqual(rv.status_code, 200)
        payload = rv.get_json()
        self.assertEqual(
            set(payload),
            {"members", "members_version", "role", "can_edit", "can_manage", "admin"},
        )
        self.assertEqual(
            (payload["members_version"], payload["role"], payload["can_edit"], payload["can_manage"], payload["admin"]),
            (1, "viewer", False, False, False),
        )
        self.assertEqual([m["role"] for m in payload["members"]], ["owner", "editor", "viewer"])
        version = self.chloe_client.get(f"/api/playlist/{self.pl.id}/version").get_json()
        self.assertEqual(version["version"], 1)
        self.assertEqual(version["members_version"], 1)
        self.assertEqual(version["nb_tracks"], 2)

    def test_recommendations_are_readable_but_not_for_a_non_member(self):
        self.assertEqual(
            self.chloe_client.get(f"/api/playlist/{self.pl.id}/recommendations").status_code,
            200,
        )
        self.assertEqual(
            self.dave_client.get(f"/api/playlist/{self.pl.id}/recommendations").status_code,
            404,
        )


class MembershipTestCase(_CollabCase):
    def setUp(self):
        super().setUp()
        self.pl = self._playlist()
        self._fill(self.pl, [self._track("1")])

    def _members(self, client=None):
        return (client or self.client).get(
            f"/api/playlist/{self.pl.id}/members"
        ).get_json()

    def test_invite_role_and_remove(self):
        rv = self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "bob", "role": "editor"},
        )
        self.assertEqual(rv.status_code, 200, rv.get_json())
        self.assertEqual(rv.get_json()["members_version"], 2)
        bob_row = [m for m in rv.get_json()["members"] if m["name"] == "bob"][0]
        self.assertEqual(bob_row["role"], "editor")

        # Re-invite = upsert, and `added` is preserved.
        rv = self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "bob", "role": "viewer"},
        )
        second = [m for m in rv.get_json()["members"] if m["name"] == "bob"][0]
        self.assertEqual(second["role"], "viewer")
        self.assertEqual(second["added"], bob_row["added"])

        bob_id = self.bob.id
        rv = self.client.patch(
            f"/api/playlist/{self.pl.id}/members/{bob_id}",
            json={"role": "editor"},
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(
            [m for m in rv.get_json()["members"] if m["name"] == "bob"][0]["role"],
            "editor",
        )

        rv = self.client.delete(f"/api/playlist/{self.pl.id}/members/{bob_id}")
        self.assertEqual(rv.status_code, 200)
        self.assertNotIn("bob", [m["name"] for m in rv.get_json()["members"]])
        # Idempotent: removing a non-member is a 200 with the current list.
        again = self.client.delete(f"/api/playlist/{self.pl.id}/members/{bob_id}")
        self.assertEqual(again.status_code, 200)
        self.assertEqual(again.get_json(), rv.get_json())

    def test_the_statuses_of_a_bad_invite(self):
        self.assertEqual(
            self.client.post(
                f"/api/playlist/{self.pl.id}/members",
                json={"username": "nobody", "role": "editor"},
            ).status_code,
            404,
        )
        self.assertEqual(
            self.client.post(
                f"/api/playlist/{self.pl.id}/members",
                json={"username": "alice", "role": "editor"},
            ).status_code,
            400,
        )
        self.assertEqual(
            self.client.post(
                f"/api/playlist/{self.pl.id}/members",
                json={"username": "bob", "role": "owner"},
            ).status_code,
            400,
        )
        # The owner is never assignable, nor removable.
        self.assertEqual(
            self.client.patch(
                f"/api/playlist/{self.pl.id}/members/{self.alice.id}",
                json={"role": "viewer"},
            ).status_code,
            409,
        )
        self.assertEqual(
            self.client.delete(
                f"/api/playlist/{self.pl.id}/members/{self.alice.id}"
            ).status_code,
            409,
        )
        # Malformed member id: a 404, never a 500 or an oracle.
        self.assertEqual(
            self.client.patch(
                f"/api/playlist/{self.pl.id}/members/not-a-uuid",
                json={"role": "viewer"},
            ).status_code,
            404,
        )

    def test_an_editor_cannot_manage_but_a_viewer_may_leave(self):
        self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "bob", "role": "editor"},
        )
        self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "chloe", "role": "viewer"},
        )
        self.assertEqual(
            self.bob_client.post(
                f"/api/playlist/{self.pl.id}/members",
                json={"username": "dave", "role": "viewer"},
            ).status_code,
            403,
        )
        # The owner cannot leave: there is no transfer in v1.
        self.assertEqual(
            self.client.post(f"/api/playlist/{self.pl.id}/leave", json={}).status_code,
            409,
        )
        # An editor and a viewer both can, and lose access on the next request.
        self.assertEqual(
            self.chloe_client.post(f"/api/playlist/{self.pl.id}/leave", json={}).status_code,
            200,
        )
        self.assertEqual(
            self.chloe_client.get(f"/api/playlist/{self.pl.id}").status_code, 404
        )
        self.assertEqual(
            self.bob_client.post(f"/api/playlist/{self.pl.id}/leave", json={}).status_code,
            200,
        )
        self.assertEqual(
            self.bob_client.get(f"/api/playlist/{self.pl.id}").status_code, 404
        )
        # An admin who is not a member has no membership to leave.
        self.assertEqual(
            self.dave_client.post(f"/api/playlist/{self.pl.id}/leave", json={}).status_code,
            404,
        )

    def test_the_member_cap_is_a_409(self):
        with self.app.app_context():
            # 100 rows already (the owner plus 99 guests).
            PlaylistMember.create(playlist=self.pl, user=self.alice, role="owner")
            for n in range(99):
                u = UserManager.add(f"guest{n}", "Gu3st")
                PlaylistMember.create(playlist=self.pl, user=u, role="viewer")
        rv = self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "dave", "role": "viewer"},
        )
        self.assertEqual(rv.status_code, 409)
        self.assertEqual(rv.get_json(), {"error": "member limit"})

    def test_membership_mutations_bump_members_version_only(self):
        rv = self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={"username": "bob", "role": "editor"},
        )
        self.assertEqual(rv.get_json()["members_version"], 2)
        version = self.client.get(f"/api/playlist/{self.pl.id}/version").get_json()
        self.assertEqual(version["version"], 1, "an invite must not invalidate a reorder")
        # A content mutation after the invite still succeeds with the OLD
        # content version: the two counters are independent.
        self.assertEqual(
            self.client.patch(
                f"/api/playlist/{self.pl.id}", json={"title": "Renamed", "version": 1}
            ).status_code,
            200,
        )
        # A stale members_version is a 409 with the members scope.
        stale = self.client.post(
            f"/api/playlist/{self.pl.id}/members",
            json={
                "username": "chloe",
                "role": "viewer",
                "members_version": 1,
            },
        )
        self.assertEqual(stale.status_code, 409)
        body = stale.get_json()
        self.assertEqual(body["error"], "conflict")
        self.assertEqual(body["scope"], "members")
        self.assertEqual(body["version"], 2)
        self.assertEqual(body["members_version"], 2)


class VersionTestCase(_CollabCase):
    def setUp(self):
        super().setUp()
        self.pl = self._playlist()
        self._fill(self.pl, [self._track("1", title="One"), self._track("2", title="Two")])

    def test_a_stale_version_is_a_409_with_the_counters_only(self):
        self.assertEqual(
            self.client.patch(
                f"/api/playlist/{self.pl.id}", json={"title": "A", "version": 1}
            ).status_code,
            200,
        )
        stale = self.client.patch(
            f"/api/playlist/{self.pl.id}", json={"title": "B", "version": 1}
        )
        self.assertEqual(stale.status_code, 409)
        body = stale.get_json()
        self.assertEqual(
            set(body),
            {
                "error",
                "scope",
                "version",
                "members_version",
                "last_edited_by",
                "last_edited_at",
            },
        )
        self.assertEqual((body["error"], body["scope"]), ("conflict", "content"))
        self.assertEqual((body["version"], body["members_version"]), (2, 1))
        self.assertEqual(body["last_edited_by"], "alice")
        self.assertIsNotNone(body["last_edited_at"])
        self.assertLess(len(stale.data), 1024)

    def test_a_legacy_client_omitting_the_version_still_works(self):
        rv = self.client.patch(f"/api/playlist/{self.pl.id}", json={"title": "Old SPA"})
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["version"], 2)
        # ...and the response's version can be used by a new client.
        rv = self.client.patch(
            f"/api/playlist/{self.pl.id}", json={"title": "New", "version": 2}
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["version"], 3)

    def test_two_appends_without_a_version_both_land(self):
        first = self.client.post(
            f"/api/playlist/{self.pl.id}/tracks", json={"tracks": ["3"]}
        )
        second = self.client.post(
            f"/api/playlist/{self.pl.id}/tracks", json={"tracks": ["4"]}
        )
        self.assertEqual((first.status_code, second.status_code), (200, 200))
        self.assertEqual(first.get_json()["version"], 2)
        self.assertEqual(second.get_json()["version"], 3)
        with self.app.app_context():
            self.assertEqual(PlaylistTrack.select().count(), 4)
            indexes = sorted(t.index for t in PlaylistTrack.select())
            self.assertEqual(indexes, [0, 1, 2, 3])

    def test_delete_is_terminal_and_needs_no_version(self):
        self._join(self.pl, self.bob, "editor")
        self.assertEqual(
            self.client.delete(f"/api/playlist/{self.pl.id}").status_code, 200
        )
        with self.app.app_context():
            self.assertFalse(Playlist.select().where(Playlist.id == self.pl.id).exists())
            # Member rows and track rows go with it (schema ON DELETE CASCADE /
            # the recursive delete); nothing is left pointing at a dead id.
            self.assertEqual(PlaylistMember.select().count(), 0)
            self.assertEqual(PlaylistTrack.select().count(), 0)
        # The editor's next visit is the ordinary 404.
        self.assertEqual(self.bob_client.get(f"/api/playlist/{self.pl.id}").status_code, 404)

    def test_a_bad_version_is_a_400(self):
        for bad in (True, False, 0, -3, "2", 1.0):
            rv = self.client.patch(
                f"/api/playlist/{self.pl.id}", json={"title": "x", "version": bad}
            )
            self.assertEqual(rv.status_code, 400, bad)
            rv = self.client.post(
                f"/api/playlist/{self.pl.id}/tracks",
                json={"tracks": ["3"], "version": bad},
            )
            self.assertEqual(rv.status_code, 400, bad)

    def test_two_concurrent_reorders_one_wins_one_conflicts(self):
        """The lost-update test: exactly one 200 and one 409, never a merge."""
        barrier = threading.Barrier(2)
        results = {}

        def reorder(name, client, order):
            barrier.wait()
            rv = client.put(
                f"/api/playlist/{self.pl.id}/order",
                json={"tracks": order, "version": 1},
            )
            results[name] = (rv.status_code, rv.get_json())

        self._join(self.pl, self.bob, "editor")
        clients = {"alice": self.app.test_client(), "bob": self.app.test_client()}
        clients["alice"].post("/api/login", json={"username": "alice", "password": "Alic3"})
        clients["bob"].post("/api/login", json={"username": "bob", "password": "B0bb"})
        attendees = [
            threading.Thread(target=reorder, args=("alice", clients["alice"], ["2", "1"])),
            threading.Thread(target=reorder, args=("bob", clients["bob"], ["1", "2"])),
        ]
        for thread in attendees:
            thread.start()
        for thread in attendees:
            thread.join(timeout=15)
        statuses = sorted(code for code, _body in results.values())
        self.assertEqual(statuses, [200, 409], results)
        winner = [name for name, (code, _b) in results.items() if code == 200][0]
        with self.app.app_context():
            self.assertEqual(Playlist[self.pl.id].version, 2)
            order = [
                str(t.deezer_id)
                for t in Track.select(Track.deezer_id)
                .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
                .where(PlaylistTrack.playlist == self.pl.id)
                .order_by(PlaylistTrack.index)
            ]
        # The winner's whole order landed; the loser's was never merged.
        self.assertEqual(order, ["2", "1"] if winner == "alice" else ["1", "2"])

    def test_a_reorder_and_a_removal_without_a_version_still_work(self):
        self.assertEqual(
            self.client.put(
                f"/api/playlist/{self.pl.id}/order", json={"tracks": ["2", "1"]}
            ).status_code,
            200,
        )
        self.assertEqual(
            self.client.delete(
                f"/api/playlist/{self.pl.id}/tracks", json={"indexes": [0]}
            ).status_code,
            200,
        )
        self.assertEqual(self._titles(self.client, self.pl), ["One"])


class PrivateUploadTestCase(_CollabCase):
    """The leak rule: absent from list, count, cover and seeds; anchored on a
    reorder and preserved by a removal."""

    def setUp(self):
        super().setUp()
        self.pl = self._playlist()
        # Chloe's private uploads, not Bob's: Bob is the EDITOR whose view must
        # hide them (an uploader always sees their own, which would prove
        # nothing). One is the first row (cover/count), one is interleaved
        # (anchoring), and both must survive an editor's removal.
        self.hidden_front = self._private(
            self._track("2", album="601", title="Chloe front"), self.chloe
        )
        self.vis1 = self._track("1", album="600", title="Visible One")
        self.hidden_mid = self._private(
            self._track("5", album="605", title="Chloe mid"), self.chloe
        )
        self.vis2 = self._track("3", album="602", title="Visible Two")
        self.vis3 = self._track("4", album="603", title="Visible Three")
        self._fill(
            self.pl,
            [self.hidden_front, self.vis1, self.hidden_mid, self.vis2, self.vis3],
        )
        self._join(self.pl, self.bob, "editor")
        self._join(self.pl, self.dave, "viewer")

    def test_each_person_sees_their_own_visible_rows_only(self):
        owner = self._titles(self.client, self.pl)
        bob = self._titles(self.bob_client, self.pl)
        dave = self._titles(self.dave_client, self.pl)
        self.assertEqual(len(owner), 5)
        self.assertEqual(bob, ["Visible One", "Visible Two", "Visible Three"])
        self.assertEqual(dave, ["Visible One", "Visible Two", "Visible Three"])
        owner_data = self.client.get(f"/api/playlist/{self.pl.id}").get_json()["playlist"]
        dave_data = self.dave_client.get(f"/api/playlist/{self.pl.id}").get_json()["playlist"]
        bob_data = self.bob_client.get(f"/api/playlist/{self.pl.id}").get_json()["playlist"]
        self.assertEqual((owner_data["nb_tracks"], dave_data["nb_tracks"]), (5, 3))
        # The cover is the first VISIBLE track's album, not the hidden first row.
        from neezer.webui.refs import cover_url_for

        with self.app.app_context():
            first_album = Album.get(Album.deezer_id == "600")
            hidden_album = Album.get(Album.deezer_id == "601")
        self.assertEqual(dave_data["cover"], cover_url_for(first_album.id))
        self.assertEqual(bob_data["cover"], cover_url_for(first_album.id))
        self.assertEqual(owner_data["cover"], cover_url_for(hidden_album.id))
        self.assertNotEqual(dave_data["cover"], cover_url_for(hidden_album.id))

    def test_me_playlists_counts_only_visible_rows(self):
        dave = self.dave_client.get("/api/me/playlists").get_json()["playlists"][0]
        bob = self.bob_client.get("/api/me/playlists").get_json()["playlists"][0]
        self.assertEqual(dave["nb_tracks"], 3)
        self.assertEqual(bob["nb_tracks"], 3)
        self.assertEqual((dave["role"], dave["can_manage"]), ("viewer", False))
        self.assertEqual((bob["role"], bob["editable"]), ("editor", True))
        self.assertEqual(dave["owner"], "alice")

    def test_duplicate_occurrences_keep_their_own_hidden_neighbour(self):
        """A playlist may hold the same title twice; each occurrence anchors the
        hidden row that followed it, so a reorder never renumbers or merges
        them (the old set-based reorder silently deleted duplicates)."""
        with self.app.app_context():
            other = Playlist.create(user=self.alice, name="Duplicates")
            dup = self._track("9", album="609", title="Dup")
            hidden_a = self._private(self._track("10", album="610", title="Mid A"), self.chloe)
            hidden_b = self._private(self._track("11", album="611", title="Mid B"), self.chloe)
            for index, track in enumerate([dup, hidden_a, dup, hidden_b]):
                PlaylistTrack.create(playlist=other, track=track, index=index)
            PlaylistMember.create(playlist=other, user=self.bob, role="editor")
        rv = self.bob_client.put(
            f"/api/playlist/{other.id}/order", json={"tracks": ["9", "9"]}
        )
        self.assertEqual(rv.status_code, 200, rv.get_json())
        self.assertEqual(
            self._titles(self.client, other),
            ["Dup", "Mid A", "Dup", "Mid B"],
        )

    def test_an_editor_cannot_add_somebody_elses_private_upload(self):
        rv = self.bob_client.post(
            f"/api/playlist/{self.pl.id}/tracks",
            json={"tracks": [str(self.hidden_front.id)]},
        )
        self.assertEqual(rv.status_code, 400)
        self.assertEqual(rv.get_json(), {"error": "no tracks"})

    def test_an_editor_reorder_anchors_the_hidden_row(self):
        # Visible order [One, Two, Three] -> [Three, One, Two].
        rv = self.dave_client.put(
            f"/api/playlist/{self.pl.id}/order",
            json={"tracks": ["4", "1", "3"]},
        )
        self.assertEqual(rv.status_code, 403, "a viewer cannot reorder")
        rv = self.bob_client.put(
            f"/api/playlist/{self.pl.id}/order",
            json={"tracks": ["4", "1", "3"], "version": 1},
        )
        self.assertEqual(rv.status_code, 200, rv.get_json())
        # The front hidden row stays at the front (no preceding visible row);
        # the middle one followed Visible One, which is now second: it stays
        # immediately after it, never at the end.
        self.assertEqual(
            self._titles(self.client, self.pl),
            [
                "Chloe front",
                "Visible Three",
                "Visible One",
                "Chloe mid",
                "Visible Two",
            ],
        )
        self.assertEqual(
            self._titles(self.dave_client, self.pl),
            ["Visible Three", "Visible One", "Visible Two"],
        )

    def test_an_editor_removal_keeps_the_hidden_row(self):
        # Dave removes visible index 0 (Visible One).
        rv = self.dave_client.delete(
            f"/api/playlist/{self.pl.id}/tracks", json={"indexes": [0]}
        )
        self.assertEqual(rv.status_code, 403)
        rv = self.bob_client.delete(
            f"/api/playlist/{self.pl.id}/tracks",
            json={"indexes": [1], "version": 1},  # visible index 1 = Visible Two
        )
        self.assertEqual(rv.status_code, 200, rv.get_json())
        self.assertEqual(
            self._titles(self.client, self.pl),
            ["Chloe front", "Visible One", "Chloe mid", "Visible Three"],
        )

    def test_recommendation_seeds_do_not_include_the_hidden_row(self):
        rv = self.dave_client.get(
            f"/api/playlist/{self.pl.id}/recommendations?limit=20"
        )
        self.assertEqual(rv.status_code, 200)
        ids = {t["id"] for t in rv.get_json()["tracks"]}
        deezer_ids = {t.get("deezer_id") for t in rv.get_json()["tracks"]}
        self.assertNotIn(str(self.hidden_front.id), ids)
        self.assertNotIn(str(self.hidden_mid.id), ids)
        self.assertFalse({"2", "5"} & deezer_ids)


class ListTestCase(_CollabCase):
    def test_me_playlists_is_own_plus_memberships_never_a_firehose(self):
        self._playlist(name="Alice")
        self._playlist(owner=self.bob, name="Bob")
        shared = self._playlist(name="Shared with bob")
        self._join(shared, self.bob, "editor")
        # A public playlist of SOMEBODY ELSE is readable by URL and listed by
        # Subsonic, but never pushed into this person's web list.
        self._playlist(owner=self.bob, name="Public", public=True)

        alice_titles = [p["title"] for p in self.client.get("/api/me/playlists").get_json()["playlists"]]
        bob_titles = [p["title"] for p in self.bob_client.get("/api/me/playlists").get_json()["playlists"]]
        self.assertIn("Alice", alice_titles)
        self.assertIn("Shared with bob", alice_titles)
        self.assertNotIn("Bob", alice_titles, "an admin never gets every playlist")
        self.assertNotIn("Public", alice_titles)
        self.assertEqual(sorted(bob_titles), ["Bob", "Public", "Shared with bob"])
        shared_row = [
            p for p in self.bob_client.get("/api/me/playlists").get_json()["playlists"]
            if p["title"] == "Shared with bob"
        ][0]
        self.assertEqual(
            (shared_row["role"], shared_row["editable"], shared_row["can_manage"]),
            ("editor", True, False),
        )
        self.assertEqual(shared_row["owner"], "alice")
        # The folders contract is preserved.
        self.assertIn("folders", self.client.get("/api/me/playlists").get_json())
        self.assertIn("folder_id", shared_row)
        self.assertIn("position", shared_row)

    def test_sixty_one_playlists_cost_six_statements(self):
        with self.app.app_context():
            for n in range(61):
                Playlist.create(user=self.alice, name=f"P{n:03d}")
            shared = Playlist.create(user=self.bob, name="Shared")
            PlaylistMember.create(playlist=shared, user=self.bob, role="owner")
            PlaylistMember.create(playlist=shared, user=self.alice, role="editor")
        with count_statements() as counts:
            answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual(len(answer["playlists"]), 62)
        # Session, the list+role join, counts, covers, folders, folder counts.
        # Measured: 6 at 61 owned playlists (tests/test_playlist_folders.py) and
        # still 6 with a membership: the role rides the same SELECT.
        self.assertEqual(sum(counts.values()), 6, counts)

    def test_creating_a_playlist_is_open_and_inserts_the_owner_row(self):
        rv = self.bob_client.post("/api/playlists", json={"title": "Mine"})
        self.assertEqual(rv.status_code, 200, rv.get_json())
        body = rv.get_json()
        self.assertEqual((body["version"], body["members_version"], body["role"]), (1, 1, "owner"))
        with self.app.app_context():
            row = PlaylistMember.get(
                (PlaylistMember.playlist == Playlist[uuid.UUID(body["id"])])
                & (PlaylistMember.user == self.bob)
            )
            self.assertEqual(row.role, "owner")
        data = self.bob_client.get(f"/api/playlist/{body['id']}").get_json()["playlist"]
        self.assertEqual((data["role"], data["editable"], data["can_manage"]), ("owner", True, True))

    def test_a_direct_created_playlist_still_belongs_to_its_owner(self):
        pl = self._playlist(owner=self.bob, name="Old")
        data = self.bob_client.get(f"/api/playlist/{pl.id}").get_json()["playlist"]
        self.assertEqual(data["role"], "owner")
        self.assertEqual([m["name"] for m in data["members"]], ["bob"])
        self.assertEqual(
            self.bob_client.patch(f"/api/playlist/{pl.id}", json={"title": "Edited"}).status_code,
            200,
        )

    def test_input_caps_and_validation(self):
        pl = self._playlist()
        self.assertEqual(
            self.client.post("/api/playlists", json={"title": "x" * 257}).status_code,
            400,
        )
        self.assertEqual(
            self.client.patch(
                f"/api/playlist/{pl.id}", json={"description": "x" * 257}
            ).status_code,
            400,
        )
        self.assertEqual(
            self.client.patch(f"/api/playlist/{pl.id}", json={"title": 123}).status_code,
            400,
        )
        # Body caps: 16 KiB for metadata, 256 KiB for track refs.
        big = json.dumps({"title": "x" * (17 * 1024)})
        rv = self.client.patch(
            f"/api/playlist/{pl.id}", data=big, content_type="application/json"
        )
        self.assertEqual(rv.status_code, 413)
        huge = json.dumps({"tracks": ["x" * 40] * 7000})
        rv = self.client.post(
            f"/api/playlist/{pl.id}/tracks",
            data=huge,
            content_type="application/json",
        )
        self.assertEqual(rv.status_code, 413)

    def test_a_cross_site_mutation_is_refused(self):
        pl = self._playlist()
        rv = self.client.post(
            f"/api/playlist/{pl.id}/tracks",
            json={"tracks": ["1"]},
            headers={"Sec-Fetch-Site": "cross-site"},
        )
        self.assertEqual(rv.status_code, 403)


class RemotePolicyTestCase(unittest.TestCase):
    def test_the_membership_routes_are_classified(self):
        from neezer.webui import remote as R

        self.assertEqual(R.POLICY["playlist_version"], "read")
        self.assertEqual(R.POLICY["playlist_members"], "read")
        for endpoint in (
            "playlist_invite",
            "playlist_member_role",
            "playlist_member_remove",
            "playlist_leave",
        ):
            self.assertIs(R.POLICY[endpoint], R.NEVER, endpoint)
