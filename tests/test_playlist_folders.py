# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Playlist folders (neezer/webui/playlist_folders.py, schema 20261114).

A folder is the person's OWN organisation of their playlists: private, one
level, never a provider write. What is pinned here is what must never get
wrong — a folder of one account is unreachable from another (404, never an
oracle), the order PUT is additive (an omitted playlist is appended, never
lost), deleting a folder returns its playlists to the root instead of deleting
them, Subsonic does not move, and the list stays a handful of statements at
the sizes real libraries reach.

Every number in an assertion was measured, not guessed: 6 statements for
``/api/me/playlists`` at 61 playlists / 6 folders, 8 at 500 / 20, and 6 for a
500-id container reorder (see the comments beside them).
"""

import json
import unittest
import uuid
from datetime import datetime, timedelta

from neezer.db import (
    Playlist,
    PlaylistFolder,
    PlaylistPlacement,
    User,
)
from neezer.managers.user import UserManager
from neezer.webui import remote as R

from .detach_helpers import assert_no_deezer_writes
from .test_deezer import count_statements
from .test_detach_a import _AppCase

SUBSONIC = {"u": "alice", "p": "Alic3", "c": "t", "v": "1.12.0", "f": "json"}


class _FolderCase(_AppCase):
    """One server, alice (admin, the SPA's account) and bob (a guest)."""

    def setUp(self):
        super().setUp()
        UserManager.add("bob", "B0bb")
        self.alice = User.get(User.name == "alice")
        self.bob = User.get(User.name == "bob")
        self.bob_client = self._client("bob", "B0bb")

    # -- fixtures ---------------------------------------------------------------

    def _client(self, name, password):
        c = self.app.test_client()
        rv = c.post("/api/login", json={"username": name, "password": password})
        self.assertEqual(rv.status_code, 200, rv.get_json())
        return c

    def _folder(self, title="Travail", fid=None, client=None):
        body = {"title": title}
        if fid is not None:
            body["id"] = str(fid)
        rv = (client or self.client).post("/api/playlist-folders", json=body)
        self.assertIn(rv.status_code, (200, 201), rv.get_json())
        return rv.get_json()

    def _playlist(self, name="P", user=None, created=None):
        with self.app.app_context():
            return Playlist.create(
                user=user or self.alice,
                name=name,
                **({"created": created} if created is not None else {}),
            )

    def _place(self, playlist, folder=None, position=0):
        with self.app.app_context():
            return PlaylistPlacement.create(
                user=self.alice, playlist=playlist, folder=folder, position=position
            )

    def _placements(self):
        """``{(playlist id, folder id or None): position}``, the raw table."""
        with self.app.app_context():
            return {
                (str(pid), str(fid) if fid is not None else None): position
                for pid, fid, position in (
                    PlaylistPlacement.select(
                        PlaylistPlacement.playlist,
                        PlaylistPlacement.folder,
                        PlaylistPlacement.position,
                    )
                    .order_by(PlaylistPlacement.position)
                    .tuples()
                )
            }

    def _order(self, client=None):
        answer = (client or self.client).get("/api/me/playlists").get_json()
        return [p["title"] for p in answer["playlists"]]

    def _bulk(self, playlists, folders):
        """``folders`` folders and ``playlists`` playlists: two thirds filed."""
        with self.app.app_context():
            made = [
                PlaylistFolder.create(user=self.alice, name=f"F{n:02d}", position=n)
                for n in range(folders)
            ]
            rows, playlist_rows = [], []
            for n in range(playlists):
                pl = Playlist.create(user=self.alice, name=f"P{n:03d}")
                playlist_rows.append(pl)
                if n % 3:
                    rows.append((self.alice.id, pl.id, made[n % folders].id, n))
            PlaylistPlacement.insert_many(
                rows,
                fields=(
                    PlaylistPlacement.user,
                    PlaylistPlacement.playlist,
                    PlaylistPlacement.folder,
                    PlaylistPlacement.position,
                ),
            ).execute()
            return made, playlist_rows


# -- CRUD ---------------------------------------------------------------------


class FolderCrudTestCase(_FolderCase):
    def test_a_folder_is_created_renamed_and_deleted(self):
        fid = uuid.uuid4()
        created = self.client.post(
            "/api/playlist-folders", json={"id": str(fid), "title": "  Travail  "}
        )
        self.assertEqual(created.status_code, 201)
        self.assertEqual(
            created.get_json(),
            {"ok": True, "id": str(fid), "title": "Travail", "position": 0},
        )
        # A second folder is APPENDED (MAX + 1): creating never rewrites the
        # order the person already chose.
        self.assertEqual(self._folder("Jazz")["position"], 1)
        self.assertEqual(
            [(f["title"], f["position"]) for f in self.client.get("/api/me/playlists").get_json()["folders"]],
            [("Travail", 0), ("Jazz", 1)],
        )
        renamed = self.client.patch(
            "/api/playlist-folders/" + str(fid), json={"title": "Boulot"}
        )
        self.assertEqual(renamed.status_code, 200)
        self.assertEqual(renamed.get_json(), {"ok": True, "id": str(fid), "title": "Boulot"})
        deleted = self.client.delete("/api/playlist-folders/" + str(fid))
        self.assertEqual(deleted.get_json(), {"ok": True, "playlists": []})
        self.assertEqual(
            [f["title"] for f in self.client.get("/api/me/playlists").get_json()["folders"]],
            ["Jazz"],
        )
        self.assertEqual(PlaylistFolder.select().count(), 1)

    def test_a_retried_create_returns_the_same_folder(self):
        """The id is the client's, so a POST retried after a timeout is not a
        second folder — the one thing an optimistic UI cannot reconcile."""
        fid = uuid.uuid4()
        body = {"id": str(fid), "title": "Travail"}
        first = self.client.post("/api/playlist-folders", json=body)
        again = self.client.post("/api/playlist-folders", json=body)
        self.assertEqual((first.status_code, again.status_code), (201, 200))
        self.assertEqual(again.get_json(), first.get_json())
        self.assertEqual(PlaylistFolder.select().count(), 1)

    def test_a_title_is_one_to_eighty_printable_characters(self):
        cases = [
            ({}, "missing title"),  # the /api coverage walk POSTs {}
            ({"title": ""}, "missing title"),
            ({"title": "   "}, "missing title"),
            ({"title": 5}, "missing title"),
            ({"title": "x" * 81}, "title too long"),
            # A newline reflows the sidebar row; a NUL is refused by MySQL.
            ({"title": "Travail\nJazz"}, "invalid title"),
            ({"title": "Travail\x07"}, "invalid title"),
        ]
        for body, code in cases:
            rv = self.client.post("/api/playlist-folders", json=body)
            self.assertEqual(rv.status_code, 400, body)
            self.assertEqual(rv.get_json(), {"error": code}, body)
        self.assertEqual(PlaylistFolder.select().count(), 0)
        # Exactly 80 is accepted, and the stored name is stripped.
        rv = self.client.post("/api/playlist-folders", json={"title": " " + "x" * 80 + " "})
        self.assertEqual(rv.status_code, 201)
        self.assertEqual(len(rv.get_json()["title"]), 80)

    def test_a_rename_needs_a_title_too(self):
        fid = self._folder()["id"]
        for body in ({}, {"title": ""}, {"title": "x" * 81}, {"title": "a\x00b"}):
            rv = self.client.patch("/api/playlist-folders/" + fid, json=body)
            self.assertEqual(rv.status_code, 400, body)
        self.assertEqual(PlaylistFolder.get().name, "Travail")

    def test_a_malformed_id_is_400_on_every_route(self):
        self.assertEqual(
            self.client.post(
                "/api/playlist-folders", json={"id": "not-a-uuid", "title": "X"}
            ).status_code,
            400,
        )
        for method, path, body in (
            ("PATCH", "/api/playlist-folders/not-a-uuid", {"title": "X"}),
            ("DELETE", "/api/playlist-folders/not-a-uuid", None),
            ("PUT", "/api/playlist-folders/not-a-uuid/order", {"playlists": []}),
        ):
            kwargs = {"json": body} if body is not None else {}
            rv = self.client.open(path, method=method, **kwargs)
            self.assertEqual(rv.status_code, 400, (method, path))
            self.assertEqual(rv.get_json(), {"error": "invalid id"}, (method, path))
        self.assertEqual(PlaylistFolder.select().count(), 0)

    def test_a_hostile_body_is_a_400_never_a_500(self):
        """An empty body, a non-JSON body and a JSON array all answer 400: the
        /api coverage walk POSTs every route with no body at all, and a JSON
        array must not reach ``data.get``."""
        fid = self._folder()["id"]
        routes = (
            ("POST", "/api/playlist-folders"),
            ("PATCH", "/api/playlist-folders/" + fid),
            ("PUT", "/api/playlist-folders/order"),
            ("PUT", "/api/playlist-folders/root/order"),
            ("PUT", "/api/playlist-folders/%s/order" % fid),
        )
        for method, path in routes:
            for payload in ([1, 2], "nope", 7):
                rv = self.client.open(path, method=method, json=payload)
                self.assertEqual(rv.status_code, 400, (method, path, payload))
                self.assertEqual(rv.get_json(), {"error": "invalid body"}, (path, payload))
            for raw, ctype in ((b"", "application/json"), (b"not json", "application/json"), (b"x", "text/plain")):
                rv = self.client.open(path, method=method, data=raw, content_type=ctype)
                self.assertEqual(rv.status_code, 400, (method, path, raw))
                self.assertEqual(rv.get_json(), {"error": "invalid body"}, (path, raw))
        self.assertEqual(PlaylistFolder.select().count(), 1)


# -- ownership ----------------------------------------------------------------


class FolderOwnershipTestCase(_FolderCase):
    def test_a_folder_is_private_to_one_person(self):
        mine = self._folder("À moi")
        theirs = self._folder("À lui", client=self.bob_client)
        # Renaming somebody else's folder is a 404, never a 403: the answer
        # must not confirm that the id exists.
        rv = self.bob_client.patch("/api/playlist-folders/" + mine["id"], json={"title": "X"})
        self.assertEqual(rv.status_code, 404)
        self.assertEqual(rv.get_json(), {"error": "folder not found"})
        self.assertEqual(self.client.patch("/api/playlist-folders/" + theirs["id"], json={"title": "X"}).status_code, 404)
        # Deleting is idempotent and equally silent, and never touches it.
        self.assertEqual(self.bob_client.delete("/api/playlist-folders/" + mine["id"]).get_json(),
                         {"ok": True, "playlists": []})
        with self.app.app_context():
            self.assertEqual(
                PlaylistFolder.select().where(PlaylistFolder.id == uuid.UUID(mine["id"])).count(), 1
            )
        self.assertEqual(self.client.patch("/api/playlist-folders/" + theirs["id"], json={"title": "X"}).status_code, 404)
        # Neither account's list ever shows the other's folders. Every account
        # has its own list now (collaborative lot): bob's has HIS folder and no
        # playlist, never alice's folder.
        bob_answer = self.bob_client.get("/api/me/playlists").get_json()
        self.assertEqual([f["title"] for f in bob_answer["folders"]], ["À lui"])
        self.assertEqual(bob_answer["playlists"], [])
        self.assertEqual(
            [f["title"] for f in self.client.get("/api/me/playlists").get_json()["folders"]],
            ["À moi"],
        )

    def test_creating_over_somebody_elses_id_is_refused_without_a_leak(self):
        fid = uuid.uuid4()
        with self.app.app_context():
            PlaylistFolder.create(id=fid, user=self.bob, name="À lui", position=0)
        rv = self.client.post("/api/playlist-folders", json={"id": str(fid), "title": "Volé"})
        # The very 400 a malformed id gets: the answer confirms nothing.
        self.assertEqual(rv.status_code, 400)
        self.assertEqual(rv.get_json(), {"error": "invalid id"})
        with self.app.app_context():
            self.assertEqual(
                PlaylistFolder.select().where(PlaylistFolder.id == fid).get().name, "À lui"
            )

    def test_a_foreign_folder_cannot_receive_a_placement(self):
        theirs = self._folder("À lui", client=self.bob_client)
        mine = self._playlist("Mienne")
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % theirs["id"], json={"playlists": [str(mine.id)]}
        )
        self.assertEqual(rv.status_code, 404)
        self.assertEqual(rv.get_json(), {"error": "folder not found"})
        self.assertEqual(PlaylistPlacement.select().count(), 0)

    def test_a_playlist_the_caller_cannot_see_is_ignored(self):
        """The fileable set is exactly /api/me/playlists': the caller's own
        lists, or a Deezer import. Another person's private playlist is not an
        error, it is simply absent."""
        bobs = self._playlist("Privée de bob", user=self.bob)
        mine = self._playlist("Mienne")
        rv = self.client.put(
            "/api/playlist-folders/root/order",
            json={"playlists": [str(bobs.id), str(mine.id)]},
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["playlists"], [str(mine.id)])
        self.assertEqual(list(self._placements()), [(str(mine.id), None)])

    def test_an_imported_playlist_can_be_filed_like_any_other(self):
        imported = self._playlist("Depuis Deezer")
        with self.app.app_context():
            imported.deezer_id = "908622995"
            imported.save(only=(Playlist.deezer_id,))
        folder = self._folder("Jazz")
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"],
            json={"playlists": [str(imported.id)]},
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["playlists"], [str(imported.id)])
        self.assertEqual(self._placements(), {(str(imported.id), folder["id"]): 0})


# -- the same-origin guard ----------------------------------------------------


class FolderCrossSiteTestCase(_FolderCase):
    def test_every_mutation_refuses_a_cross_site_request(self):
        """The guard is installed on the NEW blueprint: ``webapi``'s
        before_request hooks are not inherited, so a missing copy here would be
        a hole the rest of /api does not have."""
        folder = self._folder()
        playlist = self._playlist("P")
        calls = (
            ("POST", "/api/playlist-folders", {"title": "X"}),
            ("PATCH", "/api/playlist-folders/" + folder["id"], {"title": "X"}),
            ("DELETE", "/api/playlist-folders/" + folder["id"], None),
            ("PUT", "/api/playlist-folders/order", {"folders": [folder["id"]]}),
            ("PUT", "/api/playlist-folders/root/order", {"playlists": [str(playlist.id)]}),
            ("PUT", "/api/playlist-folders/%s/order" % folder["id"], {"playlists": [str(playlist.id)]}),
        )
        for method, path, body in calls:
            for headers in ({"Sec-Fetch-Site": "cross-site"}, {"Origin": "https://evil.example"}):
                kwargs = {"json": body} if body is not None else {}
                rv = self.client.open(path, method=method, headers=headers, **kwargs)
                self.assertEqual(rv.status_code, 403, (method, path, headers))
                self.assertEqual(rv.get_json(), {"error": "cross-site request rejected"})
        # Nothing went through: same folder, same title, no placement at all.
        self.assertEqual(PlaylistFolder.select().count(), 1)
        self.assertEqual(PlaylistFolder.get().name, "Travail")
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        # The SPA's own call is accepted, so the guard is a guard and not a wall.
        rv = self.client.open(
            "/api/playlist-folders",
            method="POST",
            json={"title": "Accepté"},
            headers={"Sec-Fetch-Site": "same-origin"},
        )
        self.assertEqual(rv.status_code, 201, rv.get_json())
        self.assertEqual(PlaylistFolder.select().count(), 2)


# -- placement ----------------------------------------------------------------


class PlacementTestCase(_FolderCase):
    def test_an_order_files_appends_and_renormalises(self):
        a, b, c = (self._playlist(n) for n in "ABC")
        folder = self._folder()
        # A complete root order first, so all three are RESIDENTS of it.
        rv = self.client.put(
            "/api/playlist-folders/root/order",
            json={"playlists": [str(a.id), str(b.id), str(c.id)]},
        )
        self.assertEqual(rv.get_json()["playlists"], [str(a.id), str(b.id), str(c.id)])
        self.assertEqual(
            self._placements(),
            {(str(a.id), None): 0, (str(b.id), None): 1, (str(c.id), None): 2},
        )
        # B, A; C is omitted and must be APPENDED, never dropped.
        rv = self.client.put(
            "/api/playlist-folders/root/order",
            json={"playlists": [str(b.id), str(a.id)]},
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(
            rv.get_json(),
            {"ok": True, "folder": None, "playlists": [str(b.id), str(a.id), str(c.id)]},
        )
        self.assertEqual(
            self._placements(),
            {(str(b.id), None): 0, (str(a.id), None): 1, (str(c.id), None): 2},
        )
        self.assertEqual(self._order(), ["B", "A", "C"])
        # A move into the folder is the DESTINATION's order, and the root
        # renormalises around the hole it leaves (A stays at 0, C at 1).
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"],
            json={"playlists": [str(c.id)]},
        )
        self.assertEqual(rv.get_json(), {"ok": True, "folder": folder["id"], "playlists": [str(c.id)]})
        self.assertEqual(
            self._placements(),
            {
                (str(c.id), folder["id"]): 0,
                (str(b.id), None): 0,
                (str(a.id), None): 1,
            },
        )
        # ...and moving it back to the root is the same call.
        rv = self.client.put(
            "/api/playlist-folders/root/order",
            json={"playlists": [str(c.id), str(b.id)]},
        )
        self.assertEqual(
            rv.get_json()["playlists"], [str(c.id), str(b.id), str(a.id)]
        )
        self.assertEqual(
            self._placements(),
            {(str(c.id), None): 0, (str(b.id), None): 1, (str(a.id), None): 2},
        )

    def test_a_playlist_with_no_placement_row_is_never_ordered(self):
        """It has no row at all — not a resident — so an order PUT does not have
        to know about it: it stays unplaced and sorts after the placed ones by
        creation date. Nothing is lost either way."""
        ordered = self._playlist("Ordonnée")
        loose = self._playlist("Jamais ordonnée")
        rv = self.client.put(
            "/api/playlist-folders/root/order", json={"playlists": [str(ordered.id)]}
        )
        self.assertEqual(rv.get_json()["playlists"], [str(ordered.id)])
        self.assertEqual(self._placements(), {(str(ordered.id), None): 0})
        self.assertEqual(set(self._order()), {"Ordonnée", "Jamais ordonnée"})
        answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual(
            [(p["title"], p["position"]) for p in answer["playlists"]],
            [("Ordonnée", 0), ("Jamais ordonnée", None)],
        )
        self.assertEqual(Playlist.select().count(), 2)

    def test_unknown_ids_are_ignored_and_duplicates_collapse(self):
        a, b = self._playlist("A"), self._playlist("B")
        unknown = str(uuid.uuid4())
        rv = self.client.put(
            "/api/playlist-folders/root/order",
            json={"playlists": [unknown, str(a.id), str(a.id), str(b.id), "1", 5, None]},
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["playlists"], [str(a.id), str(b.id)])
        self.assertEqual(self._placements(), {(str(a.id), None): 0, (str(b.id), None): 1})

    def test_the_same_put_twice_writes_the_same_rows(self):
        a, b, c = (self._playlist(n) for n in "ABC")
        folder = self._folder()
        order = [str(b.id), str(a.id), str(c.id)]
        self.client.put("/api/playlist-folders/%s/order" % folder["id"], json={"playlists": order})
        first = self._placements()
        self.client.put("/api/playlist-folders/%s/order" % folder["id"], json={"playlists": order})
        self.assertEqual(self._placements(), first)
        self.assertEqual(
            first,
            {(str(b.id), folder["id"]): 0, (str(a.id), folder["id"]): 1, (str(c.id), folder["id"]): 2},
        )

    def test_a_partial_order_never_loses_a_playlist(self):
        a, b, c = (self._playlist(n) for n in "ABC")
        folder = self._folder()
        self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"],
            json={"playlists": [str(a.id), str(b.id), str(c.id)]},
        )
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": [str(c.id)]}
        )
        self.assertEqual(rv.get_json()["playlists"], [str(c.id), str(a.id), str(b.id)])
        self.assertEqual(Playlist.select().count(), 3)

    def test_a_container_order_needs_ids_and_refuses_more_than_2000(self):
        empty = self.client.put("/api/playlist-folders/root/order", json={"playlists": []})
        self.assertEqual((empty.status_code, empty.get_json()), (400, {"error": "no playlists"}))
        huge = [str(uuid.uuid4()) for _ in range(2001)]
        rv = self.client.put("/api/playlist-folders/root/order", json={"playlists": huge})
        self.assertEqual((rv.status_code, rv.get_json()), (400, {"error": "too many playlists"}))
        self.assertEqual(PlaylistPlacement.select().count(), 0)

    def test_a_folder_order_needs_ids_and_refuses_more_than_500(self):
        empty = self.client.put("/api/playlist-folders/order", json={"folders": []})
        self.assertEqual((empty.status_code, empty.get_json()), (400, {"error": "no order"}))
        huge = {"folders": [str(uuid.uuid4()) for _ in range(501)]}
        rv = self.client.put("/api/playlist-folders/order", json=huge)
        self.assertEqual((rv.status_code, rv.get_json()), (400, {"error": "too many folders"}))
        # A body that is not a list of ids is refused, not silently accepted.
        for body in ("abc", [1, 2], [None]):
            rv = self.client.put("/api/playlist-folders/order", json={"folders": body})
            self.assertEqual(rv.status_code, 400, body)
            self.assertEqual(rv.get_json(), {"error": "invalid order"}, body)

    def test_folders_reorder_and_omit_appends(self):
        one, two, three = (self._folder(n) for n in ("Un", "Deux", "Trois"))
        rv = self.client.put(
            "/api/playlist-folders/order", json={"folders": [three["id"], one["id"]]}
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(
            rv.get_json(),
            {"ok": True, "folders": [three["id"], one["id"], two["id"]]},
        )
        self.assertEqual(
            [(f["title"], f["position"]) for f in self.client.get("/api/me/playlists").get_json()["folders"]],
            [("Trois", 0), ("Un", 1), ("Deux", 2)],
        )
        # Unknown/duplicate ids: ignored and collapsed, the rest is appended.
        rv = self.client.put(
            "/api/playlist-folders/order",
            json={"folders": [two["id"], str(uuid.uuid4()), two["id"]]},
        )
        self.assertEqual(rv.get_json()["folders"], [two["id"], three["id"], one["id"]])
        with self.app.app_context():
            self.assertEqual(
                [(f.name, f.position) for f in PlaylistFolder.select().order_by(PlaylistFolder.position)],
                [("Deux", 0), ("Trois", 1), ("Un", 2)],
            )

    def test_a_500_id_reorder_is_six_statements(self):
        """500 playlists in one folder, reordered in ONE request: the session,
        the container read, one DELETE, and three 240-row INSERTs (a placement
        carries four columns, so 240 rows is what stays under the 999 bound
        parameters of an older SQLite). Measured: 6."""
        folder = self._folder()
        with self.app.app_context():
            rows = []
            playlists = []
            for n in range(500):
                pl = Playlist.create(user=self.alice, name=f"P{n:03d}")
                playlists.append(pl)
                rows.append((self.alice.id, pl.id, uuid.UUID(folder["id"]), n))
            PlaylistPlacement.insert_many(
                rows,
                fields=(
                    PlaylistPlacement.user,
                    PlaylistPlacement.playlist,
                    PlaylistPlacement.folder,
                    PlaylistPlacement.position,
                ),
            ).execute()
        ids = [str(p.id) for p in playlists]
        with count_statements() as counts:
            rv = self.client.put(
                "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": list(reversed(ids))}
            )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(rv.get_json()["playlists"], list(reversed(ids)))
        self.assertLessEqual(sum(counts.values()), 6, counts)
        # No statement per id: one DELETE and chunked INSERTs.
        self.assertEqual(counts["DELETE"], 1, counts)
        self.assertLessEqual(counts["INSERT"], 3, counts)


# -- deleting a folder --------------------------------------------------------


class DeleteFolderTestCase(_FolderCase):
    def test_its_playlists_return_to_the_root_never_deleted(self):
        folder = self._folder("Jazz")
        one, two = self._playlist("Un"), self._playlist("Deux")
        self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"],
            json={"playlists": [str(two.id), str(one.id)]},
        )
        deleted = self.client.delete("/api/playlist-folders/" + folder["id"])
        self.assertEqual(deleted.status_code, 200)
        # The previous order, which is exactly what undo re-applies.
        self.assertEqual(deleted.get_json(), {"ok": True, "playlists": [str(two.id), str(one.id)]})
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        self.assertEqual(Playlist.select().count(), 2)
        answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual(answer["folders"], [])
        self.assertEqual(
            {p["id"]: (p["folder_id"], p["position"]) for p in answer["playlists"]},
            {str(one.id): (None, None), str(two.id): (None, None)},
        )

    def test_undo_restores_the_folder_and_its_order_in_two_requests(self):
        folder = self._folder("Jazz")
        one, two = self._playlist("Un"), self._playlist("Deux")
        self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"],
            json={"playlists": [str(two.id), str(one.id)]},
        )
        previous = self.client.delete("/api/playlist-folders/" + folder["id"]).get_json()["playlists"]
        # The toast's Annuler: re-create with the SAME id, then one order PUT.
        self.assertEqual(
            self.client.post("/api/playlist-folders", json={"id": folder["id"], "title": "Jazz"}).status_code,
            201,
        )
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": previous}
        )
        self.assertEqual(rv.get_json()["playlists"], previous)
        self.assertEqual(self._order(), ["Deux", "Un"])


class PlacementCascadeTestCase(_FolderCase):
    def test_deleting_a_playlist_from_the_spa_takes_its_placement(self):
        folder = self._folder()
        pl = self._playlist("P")
        self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": [str(pl.id)]}
        )
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        self.assertEqual(self.client.delete("/api/playlist/" + str(pl.id)).status_code, 200)
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        self.assertEqual(PlaylistFolder.select().count(), 1)

    def test_deleting_a_playlist_from_subsonic_takes_its_placement(self):
        track = self.make_track("1", "One")
        rv = self.client.get(
            "/rest/createPlaylist", query_string=dict(SUBSONIC, name="S", songId=str(track.id))
        )
        self.assertEqual(rv.get_json()["subsonic-response"]["status"], "ok")
        pl = Playlist.get(Playlist.name == "S")
        folder = self._folder()
        self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": [str(pl.id)]}
        )
        self.assertEqual(PlaylistPlacement.select().count(), 1)
        rv = self.client.get("/rest/deletePlaylist", query_string=dict(SUBSONIC, id=str(pl.id)))
        self.assertEqual(rv.get_json()["subsonic-response"]["status"], "ok")
        self.assertEqual(PlaylistPlacement.select().count(), 0)


# -- GET /api/me/playlists ----------------------------------------------------


class MyPlaylistsFoldersTestCase(_FolderCase):
    def test_the_payload_is_folders_then_the_root_in_container_order(self):
        start = datetime(2026, 1, 1)
        first = self._folder("Un")
        second = self._folder("Deux")
        # The person's own folder order: Deux before Un.
        self.client.put(
            "/api/playlist-folders/order", json={"folders": [second["id"], first["id"]]}
        )
        with self.app.app_context():
            un_late = self._playlist("Un tard", created=start + timedelta(minutes=9))
            un_early = self._playlist("Un tôt", created=start + timedelta(minutes=1))
            two = self._playlist("Deux seul", created=start + timedelta(minutes=5))
            placed = self._playlist("Racine placée", created=start + timedelta(minutes=8))
            free_new = self._playlist("Racine récente", created=start + timedelta(minutes=7))
            free_old = self._playlist("Racine ancienne", created=start)
        self._place(un_late, uuid.UUID(first["id"]), 1)
        self._place(un_early, uuid.UUID(first["id"]), 0)
        self._place(two, uuid.UUID(second["id"]), 0)
        self._place(placed, None, 0)

        answer = self.client.get("/api/me/playlists").get_json()
        # Folder blocks in folder order, each contiguous, then the unfiled root:
        # the placed row first, then the never-ordered ones newest first.
        self.assertEqual(
            [(p["title"], p["folder_id"], p["position"]) for p in answer["playlists"]],
            [
                ("Deux seul", second["id"], 0),
                ("Un tôt", first["id"], 0),
                ("Un tard", first["id"], 1),
                ("Racine placée", None, 0),
                ("Racine récente", None, None),
                ("Racine ancienne", None, None),
            ],
        )
        self.assertEqual(
            answer["folders"],
            [
                {"id": second["id"], "title": "Deux", "position": 0, "nb_playlists": 1},
                {"id": first["id"], "title": "Un", "position": 1, "nb_playlists": 2},
            ],
        )
        # Additive only: every key the older SPA and the car overlay read is
        # still there, with the same meaning. `role`/`can_manage`/`owner` belong
        # to the collaborative lot (shared playlists); an old client ignores
        # them.
        for playlist in answer["playlists"]:
            self.assertEqual(
                sorted(playlist),
                [
                    "can_manage",
                    "cover",
                    "deezer_id",
                    "editable",
                    "folder_id",
                    "id",
                    "nb_tracks",
                    "owner",
                    "position",
                    "role",
                    "title",
                ],
            )

    def test_a_guest_gets_two_empty_arrays(self):
        self._folder()
        self.assertEqual(
            self.bob_client.get("/api/me/playlists").get_json(),
            {"playlists": [], "folders": []},
        )

    def test_sixty_one_playlists_cost_six_statements(self):
        self._bulk(61, 6)
        with count_statements() as counts:
            answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual((len(answer["playlists"]), len(answer["folders"])), (61, 6))
        # The session, the playlists joined to the caller's placements, their
        # counts, their first tracks, the folder list, the grouped folder
        # counts. Measured: 6 (it was 4 before folders).
        self.assertEqual(sum(counts.values()), 6, counts)

    def test_five_hundred_playlists_and_twenty_folders_cost_at_most_eight(self):
        self._bulk(500, 20)
        with count_statements() as counts:
            answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual((len(answer["playlists"]), len(answer["folders"])), (500, 20))
        # Measured: 8 — the two chunked pairs (counts, first tracks) double at
        # _IN_CHUNK = 400; nothing is asked per playlist.
        self.assertLessEqual(sum(counts.values()), 8, counts)
        # 16 of the 25 playlists of the fixture land in F00 (n % 20 == 0, and
        # the fixture leaves every third playlist unfiled).
        self.assertEqual(answer["folders"][0]["nb_playlists"], 16)

    def test_an_unchanged_answer_still_revalidates(self):
        folder = self._folder("Jazz")
        rv = self.client.get("/api/me/playlists")
        tag = rv.headers["ETag"]
        again = self.client.get("/api/me/playlists", headers={"If-None-Match": tag})
        self.assertEqual(again.status_code, 304)
        # A rename changes the body, so the next visit gets a full answer.
        self.client.patch("/api/playlist-folders/" + folder["id"], json={"title": "Bop"})
        changed = self.client.get("/api/me/playlists", headers={"If-None-Match": tag})
        self.assertEqual(changed.status_code, 200)
        self.assertNotEqual(changed.headers["ETag"], tag)


# -- Subsonic does not move ---------------------------------------------------


class SubsonicUnchangedTestCase(_FolderCase):
    def test_filing_playlists_changes_nothing_on_the_subsonic_side(self):
        # `MockPrefetch` predates `getPlaylist`'s own queueing call; the folder
        # feature touches neither, so a no-op is enough to reach the answer.
        self.app.deezer_prefetch.enqueue_many = lambda *a, **k: None
        track = self.make_track("1", "One")
        ids = []
        for name in ("Alpha", "Beta"):
            rv = self.client.post(
                "/api/playlists", json={"title": name, "tracks": [str(track.id)]}
            )
            ids.append(rv.get_json()["id"])
        before_list = self.client.get("/rest/getPlaylists", query_string=SUBSONIC).get_json()
        before_one = self.client.get(
            "/rest/getPlaylist", query_string=dict(SUBSONIC, id=ids[0])
        ).get_json()
        folder = self._folder("Jazz")
        rv = self.client.put(
            "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": ids}
        )
        self.assertEqual(rv.status_code, 200)
        self.assertEqual(
            self.client.get("/rest/getPlaylists", query_string=SUBSONIC).get_json(),
            before_list,
        )
        self.assertEqual(
            self.client.get("/rest/getPlaylist", query_string=dict(SUBSONIC, id=ids[0])).get_json(),
            before_one,
        )
        self.assertNotIn("folder", json.dumps(before_list))
        self.assertNotIn("folder", json.dumps(before_one))


# -- the optional `folder` of POST /api/playlists -----------------------------


class CreateInFolderTestCase(_FolderCase):
    def test_a_new_playlist_lands_at_the_top_of_its_folder(self):
        folder = self._folder()
        first = self.client.post(
            "/api/playlists", json={"title": "Un", "folder": folder["id"]}
        )
        self.assertEqual(first.status_code, 200, first.get_json())
        self.assertEqual(first.get_json()["folder_id"], folder["id"])
        second = self.client.post(
            "/api/playlists", json={"title": "Deux", "folder": folder["id"]}
        )
        self.assertEqual(second.get_json()["folder_id"], folder["id"])
        # MIN(position) - 1: the newest sits at the top and the container is
        # never rewritten.
        self.assertEqual(
            [(p["title"], p["position"]) for p in self.client.get("/api/me/playlists").get_json()["playlists"]],
            [("Deux", -2), ("Un", -1)],
        )

    def test_a_null_folder_puts_it_at_the_top_of_the_root(self):
        old = self.client.post("/api/playlists", json={"title": "Ancienne", "folder": None})
        self.assertIsNone(old.get_json()["folder_id"])
        new = self.client.post("/api/playlists", json={"title": "Neuve", "folder": None})
        self.assertIsNone(new.get_json()["folder_id"])
        self.assertEqual(
            [(p["title"], p["position"]) for p in self.client.get("/api/me/playlists").get_json()["playlists"]],
            [("Neuve", -2), ("Ancienne", -1)],
        )

    def test_a_foreign_or_malformed_folder_creates_nothing(self):
        theirs = self._folder("À lui", client=self.bob_client)
        with self.app.app_context():
            before = Playlist.select().count()
        rv = self.client.post("/api/playlists", json={"title": "X", "folder": theirs["id"]})
        self.assertEqual(rv.status_code, 404)
        self.assertEqual(rv.get_json(), {"error": "folder not found"})
        rv = self.client.post("/api/playlists", json={"title": "X", "folder": "nope"})
        self.assertEqual(rv.status_code, 400)
        self.assertEqual(rv.get_json(), {"error": "invalid id"})
        with self.app.app_context():
            self.assertEqual(Playlist.select().count(), before)
        self.assertEqual(PlaylistPlacement.select().count(), 0)

    def test_an_older_client_without_the_key_gets_no_placement_row(self):
        rv = self.client.post("/api/playlists", json={"title": "Ancienne"})
        self.assertEqual(rv.status_code, 200)
        self.assertIsNone(rv.get_json()["folder_id"])
        self.assertEqual(PlaylistPlacement.select().count(), 0)
        # It still shows up: unfiled, never ordered, at its creation-date place.
        answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual(
            [(p["title"], p["folder_id"], p["position"]) for p in answer["playlists"]],
            [("Ancienne", None, None)],
        )


# -- remote control -----------------------------------------------------------


class FolderRemoteTestCase(_FolderCase):
    def setUp(self):
        super().setUp()
        R._reset_for_tests()

    def tearDown(self):
        R._reset_for_tests()
        super().tearDown()

    def _controller(self, level):
        rv = self.client.post(
            "/api/remote/links",
            json={"level": level, "device": "phone-0001", "device_name": "Téléphone"},
        )
        self.assertEqual(rv.status_code, 200, rv.get_json())
        c = self.app.test_client()
        rv = c.post("/api/remote/claim", json={"token": rv.get_json()["link"]["token"]})
        self.assertEqual(rv.status_code, 200, rv.get_json())
        return c

    def test_read_reaches_the_list_but_not_a_mutation(self):
        c = self._controller("read")
        self.assertEqual(c.get("/api/me/playlists").status_code, 200)
        rv = c.post("/api/playlist-folders", json={"title": "X"})
        self.assertEqual(rv.status_code, 403)
        self.assertEqual(rv.get_json(), {"error": "not allowed", "level": "read"})
        self.assertEqual(PlaylistFolder.select().count(), 0)

    def test_full_reaches_all_five_mutations(self):
        c = self._controller("full")
        created = c.post("/api/playlist-folders", json={"title": "X"})
        self.assertEqual(created.status_code, 201, created.get_json())
        fid = created.get_json()["id"]
        self.assertEqual(c.patch("/api/playlist-folders/" + fid, json={"title": "Y"}).status_code, 200)
        self.assertEqual(c.put("/api/playlist-folders/order", json={"folders": [fid]}).status_code, 200)
        pl = self._playlist("P")
        self.assertEqual(
            c.put("/api/playlist-folders/root/order", json={"playlists": [str(pl.id)]}).status_code, 200
        )
        self.assertEqual(
            c.put("/api/playlist-folders/%s/order" % fid, json={"playlists": [str(pl.id)]}).status_code, 200
        )
        self.assertEqual(c.delete("/api/playlist-folders/" + fid).status_code, 200)


# -- no provider --------------------------------------------------------------


class FolderWithoutProviderTestCase(_FolderCase):
    def test_folders_need_no_provider_at_all(self):
        """Deezer is optional to the app running: folders are this server's own
        rows, so the whole feature works with the provider switched off."""
        self.app.deezer = None
        folder = self._folder("Jazz")
        pl = self._playlist("P")
        self.assertEqual(
            self.client.put(
                "/api/playlist-folders/%s/order" % folder["id"], json={"playlists": [str(pl.id)]}
            ).status_code,
            200,
        )
        self.assertEqual(
            self.client.put("/api/playlist-folders/order", json={"folders": [folder["id"]]}).status_code,
            200,
        )
        self.assertEqual(
            self.client.post("/api/playlists", json={"title": "Q", "folder": folder["id"]}).status_code,
            200,
        )
        answer = self.client.get("/api/me/playlists").get_json()
        self.assertEqual([p["title"] for p in answer["playlists"]], ["Q", "P"])
        self.assertEqual(self.client.delete("/api/playlist-folders/" + folder["id"]).status_code, 200)
        assert_no_deezer_writes(self, self.writes)


if __name__ == "__main__":
    unittest.main()
