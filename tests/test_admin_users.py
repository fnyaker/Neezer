# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The admin account dashboard (``webui/admin_users.py``) and the account
routes it guards (legacy ``/user``, ``/rest`` create/update/delete).

Pinned here: a non-admin learns nothing (403 for every id and every search), the
list is keyset with a total in two statements and never loads a password hash,
the last active admin cannot be demoted or deleted, nobody can disable or delete
themselves, deleting an account reassigns its private uploads to the acting
admin (the file stays, ``owner_id`` is never NULL, so nothing becomes public),
revokes its pending invitations, refuses the holder of the server's Deezer
account, and every path that revokes sessions bumps ``session_epoch``.
"""

import collections
import contextlib
import os
import shutil
import tempfile
import threading
import unittest
import uuid
from datetime import timedelta
from xml.etree import ElementTree

import peewee

from neezer.config import DefaultConfig
from neezer.db import (
    Album,
    Artist,
    Folder,
    Invite,
    Playlist,
    ProviderAccount,
    Track,
    User,
    now,
    release_database,
)
from neezer.managers.user import LastAdmin, UserManager
from neezer.web import create_application
from neezer.webui.admin_users import admin_users_api

SUB = "{http://subsonic.org/restapi}"


@contextlib.contextmanager
def count_statements():
    counts = collections.Counter()
    original = peewee.Database.execute_sql
    mine = threading.get_ident()

    def execute_sql(self, sql, *args, **kwargs):
        if threading.get_ident() == mine:
            counts[sql.split(None, 1)[0].upper()] += 1
        return original(self, sql, *args, **kwargs)

    peewee.Database.execute_sql = execute_sql
    try:
        yield counts
    finally:
        peewee.Database.execute_sql = original


class AdminUsersTestCase(unittest.TestCase):
    def setUp(self):
        self.__db = tempfile.mkstemp()
        self.__dir = tempfile.mkdtemp()
        self.archive = tempfile.mkdtemp()
        db_path = self.__db[1]
        cache = self.__dir

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )

        self.app = create_application(Config())
        if "admin_users_api" not in self.app.blueprints:
            self.app.register_blueprint(admin_users_api, url_prefix="/api")
        self.alice = UserManager.add("alice", "Alic3", admin=True)
        self.bob = UserManager.add("bob", "B0b")
        self.client = self.app.test_client()

    def tearDown(self):
        release_database()
        shutil.rmtree(self.__dir, ignore_errors=True)
        shutil.rmtree(self.archive, ignore_errors=True)
        os.close(self.__db[0])
        os.remove(self.__db[1])

    # -- fixtures -------------------------------------------------------------

    def _login(self, name="alice", password="Alic3", client=None):
        client = client or self.client
        response = client.post(
            "/api/login", json={"username": name, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        return client

    def _uid(self, name):
        return User.get(User.name == name).id

    def _add(self, name, admin=False):
        return UserManager.add(name, "Quiet9-Meadow", admin=admin)

    def _upload(self, owner):
        root = Folder.create(root=True, name="Music", path=self.archive)
        artist = Artist.create(name="Artist")
        album = Album.create(name="Album", artist=artist)
        path = os.path.join(self.archive, f"{owner.name}-{uuid.uuid4()}.flac")
        with open(path, "wb") as handle:
            handle.write(b"\0")
        track = Track.create(
            disc=1,
            number=1,
            title="Mine",
            duration=1,
            album=album,
            artist=artist,
            bitrate=0,
            path=path,
            last_modification=1,
            root_folder=root,
            folder=root,
            owner=owner,
        )
        return track, path

    def _rest(self, endpoint, client=None, **params):
        params.setdefault("u", "alice")
        params.setdefault("p", "Alic3")
        params.setdefault("c", "tests")
        params.setdefault("v", "1.12.0")
        response = (client or self.client).get(
            f"/rest/{endpoint}.view", query_string=params
        )
        self.assertEqual(response.status_code, 200, response.data)
        return ElementTree.fromstring(response.data)

    def _legacy(self, name="alice", password="Alic3"):
        client = self.app.test_client()
        with client.session_transaction() as session:
            session["_csrf_token"] = "test-csrf"
        response = client.post(
            "/user/login",
            data={"user": name, "password": password, "_csrf": "test-csrf"},
            follow_redirects=True,
        )
        self.assertIn(b"Logged in", response.data)
        with client.session_transaction() as session:
            session["_csrf_token"] = "test-csrf"
        return client

    # -- authorization --------------------------------------------------------

    def test_a_non_admin_gets_403_whatever_the_id_and_leaks_nothing(self):
        routes = [
            ("GET", "/api/admin/users"),
            ("GET", f"/api/admin/users/{self.alice.id}"),
            ("PATCH", f"/api/admin/users/{self.alice.id}"),
            ("DELETE", f"/api/admin/users/{self.alice.id}"),
            ("GET", f"/api/admin/users/{uuid.uuid4()}"),
        ]
        anonymous = self.app.test_client()
        for method, path in routes:
            response = anonymous.open(path, method=method, json={})
            self.assertEqual(response.status_code, 401, (method, path))
        self._login("bob", "B0b")
        for method, path in routes:
            response = self.client.open(path, method=method, json={})
            self.assertEqual(response.status_code, 403, (method, path))
            self.assertNotIn(b"alice", response.data)

    def test_a_cross_site_write_is_refused_before_anything_changes(self):
        self._login()
        for headers in (
            {"Origin": "https://evil.example"},
            {"Sec-Fetch-Site": "cross-site"},
        ):
            response = self.client.patch(
                f"/api/admin/users/{self.bob.id}",
                json={"admin": True},
                headers=headers,
            )
            self.assertEqual(response.status_code, 403, headers)
        self.assertFalse(User[self.bob.id].admin)

    # -- listing, search, cost ------------------------------------------------

    def test_list_pages_by_keyset_with_a_total_and_no_duplicate(self):
        self._add("carol")
        self._add("dave")
        self._add("Élodie")
        self._login()
        seen = []
        after = None
        pages = 0
        while True:
            query = "/api/admin/users?limit=2"
            if after:
                query += "&after=" + after
            payload = self.client.get(query).get_json()
            self.assertEqual(payload["total"], 5)
            seen.extend(user["id"] for user in payload["users"])
            pages += 1
            after = payload["next"]
            if after is None:
                break
        self.assertEqual(pages, 3)
        self.assertEqual(len(seen), 5)
        self.assertEqual(len(set(seen)), 5)
        self.assertEqual(set(seen), {str(u.id) for u in User.select()})

    def test_search_folds_the_name_and_drops_the_wildcard(self):
        self._add("carol")
        elodie = self._add("Élodie")
        bob = User.get(User.name == "bob")
        bob.mail = "Bob@Example.com"
        bob.save()
        self._login()

        for term in ("élodie", "ELODIE", "elodie"):
            payload = self.client.get("/api/admin/users?q=" + term).get_json()
            self.assertEqual([u["id"] for u in payload["users"]], [str(elodie.id)], term)
            self.assertEqual(payload["total"], 1)

        payload = self.client.get("/api/admin/users?q=BOB@example").get_json()
        self.assertEqual([u["name"] for u in payload["users"]], ["bob"])

        self.assertEqual(self.client.get("/api/admin/users?q=%25").status_code, 400)
        self.assertEqual(self.client.get("/api/admin/users?q=zzz").get_json()["users"], [])
        self.assertEqual(self.client.get("/api/admin/users?q=" + "x" * 81).status_code, 400)
        self.assertEqual(self.client.get("/api/admin/users?limit=0").status_code, 400)
        self.assertEqual(self.client.get("/api/admin/users?limit=201").status_code, 400)
        self.assertEqual(self.client.get("/api/admin/users?after=nope").status_code, 400)

    def test_the_list_and_detail_statement_budgets(self):
        self._add("carol")
        self._upload(self.bob)
        Playlist.create(user=self.bob, name="Mine")
        self._login()
        # 1 for the session's own user, 1 page, 1 total.
        with count_statements() as counts:
            payload = self.client.get("/api/admin/users?limit=50").get_json()
        self.assertEqual(sum(counts.values()), 3)
        self.assertEqual(payload["total"], 3)
        # 1 for the session, 1 target, 1 uploads, 1 playlists.
        with count_statements() as counts:
            detail = self.client.get(f"/api/admin/users/{self.bob.id}").get_json()
        self.assertEqual(sum(counts.values()), 4)
        self.assertEqual(detail["uploads"], 1)
        self.assertEqual(detail["playlists"], 1)

    def test_the_patch_statement_budgets(self):
        self._add("carol", admin=True)
        self._login()
        # 1 session, 1 target, 1 UPDATE for a promotion (no invariant to check).
        with count_statements() as counts:
            response = self.client.patch(
                f"/api/admin/users/{self.bob.id}", json={"admin": True}
            )
        self.assertEqual(response.status_code, 200)
        self.assertEqual(sum(counts.values()), 3)
        # A demotion pays the target read, the count before and the count after.
        with count_statements() as counts:
            response = self.client.patch(
                f"/api/admin/users/{self.bob.id}", json={"admin": False}
            )
        self.assertEqual(response.status_code, 200)
        self.assertEqual(sum(counts.values()), 5)

    def test_the_payload_never_carries_a_secret(self):
        self._login()
        bob = User.get(User.name == "bob")
        detail = self.client.get(f"/api/admin/users/{bob.id}").get_json()
        self.assertEqual(
            set(detail["user"]),
            {
                "id",
                "name",
                "mail",
                "admin",
                "jukebox",
                "disabled",
                "last_login",
                "onboarding",
            },
        )
        body = self.client.get("/api/admin/users").data.decode()
        self.assertNotIn("password", body)
        self.assertNotIn(bob.password, body)
        self.assertNotIn(bob.password_clear, body)
        self.assertNotIn("session_epoch", body)

    def test_the_detail_404s_an_unknown_or_malformed_id(self):
        self._login()
        self.assertEqual(
            self.client.get(f"/api/admin/users/{uuid.uuid4()}").status_code, 404
        )
        self.assertEqual(self.client.get("/api/admin/users/not-a-uuid").status_code, 404)

    # -- role, state, sessions ------------------------------------------------

    def test_the_last_active_admin_cannot_be_demoted(self):
        self._login()
        response = self.client.patch(
            f"/api/admin/users/{self.alice.id}", json={"admin": False}
        )
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "last_admin")
        self.assertTrue(User[self.alice.id].admin)

        member = self._login("bob", "B0b", client=self.app.test_client())
        response = member.patch(
            f"/api/admin/users/{self.bob.id}", json={"admin": False}
        )
        self.assertEqual(response.status_code, 403, "a member cannot demote anybody")

        other = self._add("carol", admin=True)
        response = self.client.patch(
            f"/api/admin/users/{self.alice.id}", json={"admin": False}
        )
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertFalse(User[self.alice.id].admin)
        self.assertTrue(User[other.id].admin)

    def test_a_stale_last_admin_read_is_rolled_back(self):
        from unittest import mock

        self._add("carol", admin=True)
        epoch = User[self.alice.id].session_epoch
        # Two admins demoting each other at once, on SQLite: the second one's
        # pre-check read is stale, so the post-write count must roll it back.
        with mock.patch.object(
            UserManager, "_active_admin_ids", side_effect=[[self.alice.id], []]
        ):
            with self.assertRaises(LastAdmin):
                UserManager.update_guarded(self.alice.id, self.alice, admin=False)
        self.assertTrue(User[self.alice.id].admin)
        self.assertEqual(User[self.alice.id].session_epoch, epoch)

    def test_nobody_disables_or_deletes_themselves(self):
        self._add("carol", admin=True)
        self._login()
        response = self.client.patch(
            f"/api/admin/users/{self.alice.id}", json={"disabled": True}
        )
        self.assertEqual(response.status_code, 403)
        self.assertEqual(response.get_json()["code"], "self_disable")
        self.assertFalse(User[self.alice.id].disabled)

        response = self.client.delete(f"/api/admin/users/{self.alice.id}")
        self.assertEqual(response.status_code, 403)
        self.assertEqual(response.get_json()["code"], "self_delete")
        self.assertIsNotNone(User.get_or_none(User.id == self.alice.id))

    def test_patch_refuses_a_non_boolean(self):
        self._login()
        for body in ({"admin": "yes"}, {"disabled": 1}, {"admin": None}, {}):
            response = self.client.patch(
                f"/api/admin/users/{self.bob.id}", json=body
            )
            self.assertEqual(response.status_code, 400, body)
        self.assertEqual(
            self.client.patch(f"/api/admin/users/{uuid.uuid4()}", json={"admin": True}).status_code,
            404,
        )

    def test_patch_revokes_sessions_on_demotion_disable_and_enable_only(self):
        bob = self._login("bob", "B0b", client=self.app.test_client())
        self.assertEqual(bob.get("/api/me").status_code, 200)

        self._login()
        epoch = User[self.bob.id].session_epoch
        payload = self.client.patch(
            f"/api/admin/users/{self.bob.id}", json={"disabled": True}
        ).get_json()
        self.assertTrue(payload["user"]["disabled"])
        self.assertEqual(User[self.bob.id].session_epoch, epoch + 1)
        self.assertEqual(bob.get("/api/me").status_code, 401)

        self.client.patch(f"/api/admin/users/{self.bob.id}", json={"disabled": False})
        self.assertEqual(User[self.bob.id].session_epoch, epoch + 2)
        self.assertEqual(bob.get("/api/me").status_code, 401, "the old cookie stays dead")
        bob = self._login("bob", "B0b", client=self.app.test_client())

        epoch = User[self.bob.id].session_epoch
        payload = self.client.patch(
            f"/api/admin/users/{self.bob.id}", json={"admin": True}
        ).get_json()
        self.assertTrue(payload["user"]["admin"])
        self.assertEqual(User[self.bob.id].session_epoch, epoch, "a promotion keeps it")
        me = bob.get("/api/me")
        self.assertEqual(me.status_code, 200)
        self.assertTrue(me.get_json()["user"]["admin"])

        payload = self.client.patch(
            f"/api/admin/users/{self.bob.id}", json={"admin": False}
        ).get_json()
        self.assertFalse(payload["user"]["admin"])
        self.assertEqual(User[self.bob.id].session_epoch, epoch + 1)
        self.assertEqual(bob.get("/api/me").status_code, 401)

    def test_patch_without_a_change_is_a_free_success(self):
        self._login()
        epoch = User[self.bob.id].session_epoch
        response = self.client.patch(
            f"/api/admin/users/{self.bob.id}", json={"admin": False, "disabled": False}
        )
        self.assertEqual(response.status_code, 200)
        self.assertEqual(User[self.bob.id].session_epoch, epoch)

    # -- deletion -------------------------------------------------------------

    def test_delete_reassigns_uploads_and_deletes_no_file(self):
        track, path = self._upload(self.bob)
        Playlist.create(user=self.bob, name="Bob's")
        self._login()
        response = self.client.delete(f"/api/admin/users/{self.bob.id}")
        self.assertEqual(response.status_code, 200, response.get_json())
        self.assertEqual(response.get_json(), {"deleted": True, "uploads_reassigned": 1})
        self.assertIsNone(User.get_or_none(User.id == self.bob.id))
        track = Track[track.id]
        self.assertEqual(track.owner_id, self.alice.id)
        self.assertTrue(os.path.isfile(path))
        self.assertEqual(Track.select().where(Track.owner.is_null(True)).count(), 0)
        self.assertEqual(Playlist.select().where(Playlist.user == self.bob.id).count(), 0)

    def test_delete_revokes_the_pending_invitations_of_the_target(self):
        stamp = now()
        pending = Invite.create(
            id="pending-invite-000000001",
            token_hash="1" * 64,
            created_by=self.bob,
            created=stamp,
            expires=stamp + timedelta(days=1),
        )
        used = Invite.create(
            id="used-invite-0000000001",
            token_hash="2" * 64,
            created_by=self.bob,
            created=stamp,
            expires=stamp + timedelta(days=1),
            used_at=stamp,
        )
        expired = Invite.create(
            id="expired-invite-00000001",
            token_hash="3" * 64,
            created_by=self.bob,
            created=stamp,
            expires=stamp - timedelta(days=1),
        )
        self._login()
        self.assertEqual(
            self.client.delete(f"/api/admin/users/{self.bob.id}").status_code, 200
        )
        self.assertIsNotNone(Invite[pending.id].revoked_at)
        self.assertIsNone(Invite[used.id].revoked_at)
        self.assertIsNone(Invite[expired.id].revoked_at)

    def test_delete_forgets_the_provider_grants_but_not_other_people_s(self):
        from neezer.db import ProviderGrant

        ProviderGrant.create(
            provider="deezer", account="an-account", feature="*",
            subject=f"u:{self.bob.id}",
        )
        ProviderGrant.create(
            provider="deezer", account="an-account", feature="*",
            subject=f"u:{self.alice.id}",
        )
        self._login()
        self.assertEqual(
            self.client.delete(f"/api/admin/users/{self.bob.id}").status_code, 200
        )
        subjects = {row.subject for row in ProviderGrant.select()}
        self.assertNotIn(f"u:{self.bob.id}", subjects)
        self.assertIn(f"u:{self.alice.id}", subjects)

    def test_delete_refuses_the_holder_of_the_server_deezer_account(self):
        row = ProviderAccount.create(
            provider="deezer", user=self.bob, mode="server", status="ok"
        )
        self._login()
        response = self.client.delete(f"/api/admin/users/{self.bob.id}")
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "server_account_holder")
        self.assertIsNotNone(User.get_or_none(User.id == self.bob.id))
        self.assertIsNotNone(ProviderAccount.get_or_none(ProviderAccount.id == row.id))

    def test_delete_never_leaves_zero_active_admin_even_below_the_routes(self):
        with self.assertRaises(LastAdmin):
            UserManager.delete_guarded(self.alice.id, self.bob)
        self.assertTrue(User.get_or_none(User.id == self.alice.id).admin)

    def test_delete_costs_fifty_one_statements_for_a_small_account(self):
        self._upload(self.bob)
        Playlist.create(user=self.bob, name="Bob's")
        Invite.create(
            id="pending-invite-000000001",
            token_hash="1" * 64,
            created_by=self.bob,
            created=now(),
            expires=now() + timedelta(days=1),
        )
        self._login()
        # 4 reads (session, target, guard, server account), then the invite
        # revocation, the grants, the upload reassignment and Peewee's cascades
        # over every table that names a user. Deletion is rare; nothing here is
        # per-row except the uploads, which are one UPDATE.
        with count_statements() as counts:
            response = self.client.delete(f"/api/admin/users/{self.bob.id}")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(sum(counts.values()), 51, dict(counts))

    # -- the legacy /user routes use the same guards --------------------------

    def test_legacy_delete_cannot_remove_you_or_the_last_admin(self):
        client = self._legacy()
        response = client.post(
            f"/user/del/{self.alice.id}", data={"_csrf": "test-csrf"}, follow_redirects=True
        )
        self.assertIn(b"delete your own account", response.data)
        self.assertIsNotNone(User.get_or_none(User.id == self.alice.id))
        self.assertIsNotNone(User.get_or_none(User.id == self.bob.id))

        response = client.post(
            f"/user/del/{self.bob.id}", data={"_csrf": "test-csrf"}, follow_redirects=True
        )
        self.assertIn(b"Deleted user", response.data)
        self.assertIsNone(User.get_or_none(User.id == self.bob.id))

    def test_legacy_demotion_cannot_leave_zero_admin(self):
        client = self._legacy()
        response = client.post(
            f"/user/{self.alice.id}/changeusername",
            data={"user": "alice", "_csrf": "test-csrf"},
            follow_redirects=True,
        )
        self.assertIn(b"last active administrator", response.data)
        self.assertTrue(User[self.alice.id].admin)

    # -- the Subsonic contract ------------------------------------------------

    def test_rest_create_user_turns_a_bad_name_into_a_clean_error(self):
        for name in ("..", "a", "x y", "bob"):
            response = self._rest(
                "createUser", username=name, password="Quiet9-Meadow", email="x@y.z"
            )
            self.assertEqual(response.get("status"), "failed", name)
            error = response.find(SUB + "error")
            self.assertIsNotNone(error)
            self.assertEqual(error.get("code"), "0")
            self.assertNotIn("Traceback", ElementTree.tostring(response).decode())
        self.assertEqual(User.select().count(), 2)

    def test_rest_delete_user_is_guarded_and_reassigns(self):
        track, path = self._upload(self.bob)
        response = self._rest("deleteUser", username="alice")
        self.assertEqual(response.get("status"), "failed")
        self.assertIsNotNone(User.get_or_none(User.id == self.alice.id))

        response = self._rest("deleteUser", username="bob")
        self.assertEqual(response.get("status"), "ok")
        self.assertIsNone(User.get_or_none(User.id == self.bob.id))
        self.assertEqual(Track[track.id].owner_id, self.alice.id)
        self.assertTrue(os.path.isfile(path))

    def test_rest_update_user_protects_the_last_admin_and_bumps_the_epoch(self):
        response = self._rest("updateUser", username="alice", adminRole="false")
        self.assertEqual(response.get("status"), "failed")
        self.assertTrue(User[self.alice.id].admin)

        epoch = User[self.bob.id].session_epoch
        response = self._rest("updateUser", username="bob", adminRole="true")
        self.assertEqual(response.get("status"), "ok")
        self.assertTrue(User[self.bob.id].admin)
        self.assertEqual(User[self.bob.id].session_epoch, epoch)

        response = self._rest("updateUser", username="bob", adminRole="false")
        self.assertEqual(response.get("status"), "ok")
        self.assertFalse(User[self.bob.id].admin)
        self.assertEqual(User[self.bob.id].session_epoch, epoch + 1)

    def test_rest_never_returns_a_password(self):
        bob = User.get(User.name == "bob")
        for endpoint, params in (
            ("getUsers", {}),
            ("getUser", {"username": "bob"}),
        ):
            body = self._rest(endpoint, **params)
            text = ElementTree.tostring(body).decode()
            self.assertNotIn(bob.password, text, endpoint)
            self.assertNotIn(bob.password_clear, text, endpoint)
            self.assertNotIn("session_epoch", text, endpoint)


if __name__ == "__main__":
    unittest.main()
