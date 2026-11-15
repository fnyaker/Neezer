# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Single-use invitations: token, expiry, revocation, concurrency, limits.

What is pinned here is the contract of ``neezer/invites.py`` and
``neezer/webui/invites.py``: a token exists once in the create response and
never in the database, the logs or a later GET; an invalid token answers the
same 404 whatever the reason; one link makes exactly one account even under
concurrent redeems; the username and password rules of a new account hold; and
the public routes are bounded before they parse a body.
"""

import json
import logging
import os
import shutil
import tempfile
import threading
import unittest
from datetime import datetime, timedelta

from neezer.config import DefaultConfig
from neezer.db import Invite, SCHEMA_VERSION, User, db, now, release_database
from neezer.invites import token_for
from neezer.managers.user import UserManager
from neezer.utils import get_secret_key
from neezer.web import create_application
from neezer.webui import invites as I
from neezer.webui import remote as R
from neezer.webui.invites import invites_api

from .test_deezer import count_statements

GOOD = "CorrectHorse1"


class _LogCapture(logging.Handler):
    def __init__(self):
        super().__init__()
        self.messages = []

    def emit(self, record):
        try:
            self.messages.append(record.getMessage())
            if record.args:
                self.messages.append(repr(record.args))
        except Exception:
            self.messages.append(repr(record.msg))


class InvitesTestCase(unittest.TestCase):
    def setUp(self):
        I._reset_for_tests()
        self._fd, self._db_path = tempfile.mkstemp(suffix=".db")
        self._cache = tempfile.mkdtemp()
        db_path = self._db_path
        cache = self._cache

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP, cache_dir=cache, mount_webui=True, mount_api=True
                )

        self.app = create_application(Config())
        if "invites_api" not in self.app.blueprints:
            self.app.register_blueprint(invites_api, url_prefix="/api")
        get_secret_key("invite_secret")
        UserManager.add("alice", "Alic3pass", admin=True)
        UserManager.add("bob", "B0bbbpass")
        self.alice = self._login("alice", "Alic3pass")
        self.bob = self._login("bob", "B0bbbpass")

    def tearDown(self):
        for name in R.INVITE_POLICY:
            R.POLICY.pop(name, None)
        I._reset_for_tests()
        release_database()
        shutil.rmtree(self._cache, ignore_errors=True)
        os.close(self._fd)
        try:
            os.remove(self._db_path)
        except OSError:
            pass

    def _login(self, name, password):
        client = self.app.test_client()
        response = client.post(
            "/api/login", json={"username": name, "password": password}
        )
        self.assertEqual(response.status_code, 200, response.get_data(as_text=True))
        return client

    def _create(self, note=None, expires_in=None, client=None):
        body = {}
        if note is not None:
            body["note"] = note
        if expires_in is not None:
            body["expires_in"] = expires_in
        response = (client or self.alice).post("/api/admin/invites", json=body)
        self.assertEqual(response.status_code, 201, response.get_data(as_text=True))
        payload = response.get_json()
        return payload["invite"], payload["token"]

    def _redeem(
        self,
        token,
        username="elodie",
        password=GOOD,
        confirm=None,
        request_id=None,
        client=None,
    ):
        body = {
            "token": token,
            "username": username,
            "password": password,
            "password_confirm": password if confirm is None else confirm,
        }
        if request_id is not None:
            body["request_id"] = request_id
        return (client or self.app.test_client()).post(
            "/api/invites/redeem", json=body
        )

    def _invite_row(self, invite):
        return Invite.get(Invite.id == invite["id"])

    def test_the_create_contract_bounds_the_note_and_the_duration(self):
        for value in (0, 3599, 30 * 86400 + 1, "week", True):
            response = self.alice.post(
                "/api/admin/invites", json={"expires_in": value}
            )
            self.assertEqual(response.status_code, 400, value)
            self.assertEqual(response.get_json()["code"], "invalid_expires_in")
        response = self.alice.post(
            "/api/admin/invites", json={"note": "x" * 65}
        )
        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.get_json()["code"], "invalid_note")
        invite, _token = self._create(expires_in=3600)
        created = datetime.fromisoformat(invite["created"])
        expires = datetime.fromisoformat(invite["expires"])
        self.assertEqual((expires - created).total_seconds(), 3600)
        default, _token = self._create()
        self.assertEqual(
            (
                datetime.fromisoformat(default["expires"])
                - datetime.fromisoformat(default["created"])
            ).total_seconds(),
            7 * 86400,
        )

    def test_the_token_is_shown_once_and_the_database_keeps_only_its_hash(self):
        invite, token = self._create("pour Léa")
        self.assertRegex(token, r"\A[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{24}\Z")
        self.assertEqual(len(token), 47)
        self.assertEqual(invite["state"], "pending")
        self.assertEqual(invite["note"], "pour Léa")
        self.assertNotIn("token", invite)
        self.assertNotIn("token_hash", invite)

        row = self._invite_row(invite)
        self.assertEqual(len(row.token_hash), 64)
        self.assertNotIn(token, row.token_hash)
        self.assertNotIn(token, repr(row.__data__))
        self.assertFalse(hasattr(row, "token"))
        self.assertFalse(hasattr(row, "mac"))

        listed = self.alice.get("/api/admin/invites")
        text = listed.get_data(as_text=True)
        self.assertEqual(listed.status_code, 200)
        self.assertNotIn(token, text)
        self.assertNotIn("token_hash", text)
        self.assertNotIn(token, json.dumps(listed.get_json()))

        response = self.alice.post("/api/admin/invites", json={})
        self.assertEqual(response.status_code, 201)
        self.assertEqual(response.headers.get("Cache-Control"), "no-store")
        self.assertEqual(response.headers.get("Pragma"), "no-cache")

    def test_the_token_never_reaches_the_logs(self):
        capture = _LogCapture()
        root = logging.getLogger()
        root.addHandler(capture)
        old = root.level
        root.setLevel(logging.DEBUG)
        try:
            invite, token = self._create("invite log")
            self.app.test_client().post("/api/invites/inspect", json={"token": token})
            self._redeem(token)
        finally:
            root.removeHandler(capture)
            root.setLevel(old)
        joined = "\n".join(capture.messages)
        self.assertNotIn(token, joined)
        self.assertNotIn(token.split(".", 1)[1], joined)

    def test_every_invalid_token_gets_the_same_404(self):
        invite, token = self._create()
        expired = self._create()[0]
        revoked = self._create()[0]
        _unused, used_token = self._create()
        Invite.update(expires=now() - timedelta(minutes=1)).where(
            Invite.id == expired["id"]
        ).execute()
        self.alice.post(f"/api/admin/invites/{revoked['id']}/revoke")
        self.assertEqual(self._redeem(used_token).status_code, 201)

        bad = [
            "not-a-token",
            "A" * 22 + "." + "B" * 24,
            invite["id"] + "." + "C" * 24,
        ]
        responses = [
            self.app.test_client().post("/api/invites/inspect", json={"token": value})
            for value in bad
        ]
        for row in (expired, revoked):
            responses.append(
                self.app.test_client().post(
                    "/api/invites/inspect", json={"token": token_for(row["id"])}
                )
            )
        responses.append(
            self.app.test_client().post(
                "/api/invites/inspect", json={"token": used_token}
            )
        )
        for response in responses:
            self.assertEqual(response.status_code, 404)
            self.assertEqual(
                response.get_json(),
                {
                    "error": "Cette invitation n'est plus valide.",
                    "code": "invite_invalid",
                },
            )

    def test_expiration_refuses_inspect_and_redeem_without_consuming(self):
        invite, token = self._create(expires_in=3600)
        Invite.update(expires=now() - timedelta(seconds=1)).where(
            Invite.id == invite["id"]
        ).execute()
        inspected = self.app.test_client().post(
            "/api/invites/inspect", json={"token": token}
        )
        self.assertEqual(inspected.status_code, 404)
        redeemed = self._redeem(token)
        self.assertEqual(redeemed.status_code, 404)
        self.assertIsNone(self._invite_row(invite).used_at)
        self.assertIsNone(self._invite_row(invite).used_by_id)
        self.assertEqual(User.select().where(User.name_key == "elodie").count(), 0)

    def test_revocation_is_immediate_and_idempotent(self):
        invite, token = self._create()
        first = self.alice.post(f"/api/admin/invites/{invite['id']}/revoke")
        self.assertEqual(first.status_code, 200)
        self.assertEqual(first.get_json(), {"revoked": True})
        again = self.alice.post(f"/api/admin/invites/{invite['id']}/revoke")
        self.assertEqual(again.status_code, 200)
        self.assertEqual(
            self.app.test_client()
            .post("/api/invites/inspect", json={"token": token})
            .status_code,
            404,
        )
        self.assertEqual(self._redeem(token).status_code, 404)
        self.assertIsNotNone(self._invite_row(invite).revoked_at)

    def test_a_used_invitation_cannot_be_revoked_and_replays_are_refused(self):
        invite, token = self._create()
        self.assertEqual(self._redeem(token).status_code, 201)
        row = self._invite_row(invite)
        self.assertIsNotNone(row.used_at)
        self.assertIsNotNone(row.used_by_id)
        response = self.alice.post(f"/api/admin/invites/{invite['id']}/revoke")
        self.assertEqual(response.status_code, 409)
        self.assertEqual(response.get_json()["code"], "invite_used")
        self.assertEqual(self._redeem(token).status_code, 404)
        self.assertEqual(User.select().where(User.name_key == "elodie").count(), 1)

    def test_renew_revokes_the_old_link_and_shows_the_new_one_once(self):
        old, old_token = self._create("pour Léa", expires_in=7200)
        response = self.alice.post(f"/api/admin/invites/{old['id']}/renew")
        self.assertEqual(response.status_code, 201, response.get_data(as_text=True))
        payload = response.get_json()
        self.assertNotIn("token_hash", payload["invite"])
        self.assertEqual(payload["invite"]["note"], "pour Léa")
        self.assertNotEqual(payload["token"], old_token)
        self.assertEqual(
            self.app.test_client()
            .post("/api/invites/inspect", json={"token": old_token})
            .status_code,
            404,
        )
        self.assertEqual(
            self.app.test_client()
            .post("/api/invites/inspect", json={"token": payload["token"]})
            .status_code,
            200,
        )
        self.assertIsNotNone(self._invite_row(old).revoked_at)

    def test_request_id_makes_a_lost_response_replayable(self):
        _invite, token = self._create()
        first = self._redeem(token, request_id="retry-42")
        self.assertEqual(first.status_code, 201, first.get_data(as_text=True))
        second = self._redeem(token, request_id="retry-42")
        self.assertEqual(second.status_code, 201, second.get_data(as_text=True))
        self.assertEqual(second.get_json()["user"]["name"], "elodie")
        self.assertEqual(User.select().where(User.name_key == "elodie").count(), 1)
        self.assertEqual(self._redeem(token, request_id="other").status_code, 404)

    def test_creating_and_redeeming_an_invite_leaves_a_trace(self):
        from neezer.db import AuditLog

        invite, token = self._create(note="pour Léa")
        self.assertEqual(self._redeem(token).status_code, 201)
        rows = list(AuditLog.select().order_by(AuditLog.id))
        self.assertEqual([row.action for row in rows], ["invite.create", "invite.redeem"])
        self.assertEqual(rows[0].actor, "alice")
        self.assertEqual(rows[0].target, invite["id"])
        self.assertEqual(rows[1].target, invite["id"])
        self.assertEqual(rows[1].actor, "elodie")

    def test_a_replay_never_resurrects_a_revoked_session(self):
        _invite, token = self._create()
        first = self._redeem(token, request_id="retry-7")
        self.assertEqual(first.status_code, 201)
        created = User.get(User.name_key == "elodie")
        User.update(disabled=True, session_epoch=created.session_epoch + 1).where(
            User.id == created.id
        ).execute()
        disabled = self._redeem(token, request_id="retry-7")
        self.assertEqual(disabled.status_code, 404)
        self.assertIsNone(disabled.headers.get("Set-Cookie"))

        User.update(disabled=False, session_epoch=created.session_epoch + 2).where(
            User.id == created.id
        ).execute()
        replayed = self._redeem(token, request_id="retry-7")
        self.assertEqual(replayed.status_code, 404)
        self.assertIsNone(replayed.headers.get("Set-Cookie"))

    def test_two_concurrent_redeems_make_one_account(self):
        _invite, token = self._create()
        barrier = threading.Barrier(2)
        results = []

        def redeem():
            client = self.app.test_client()
            barrier.wait()
            try:
                response = self._redeem(token, client=client)
                results.append(response.status_code)
            finally:
                db.close()

        threads = [threading.Thread(target=redeem) for _ in range(2)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)
        self.assertEqual(sorted(results), [201, 404])
        self.assertEqual(User.select().where(User.name_key == "elodie").count(), 1)
        self.assertEqual(Invite.select().where(Invite.used_at.is_null(False)).count(), 1)

    def test_the_username_is_folded_and_its_duplicate_is_case_insensitive(self):
        invite, token = self._create()
        for name in ("BOB", "Bób"):
            I._reset_for_tests()
            response = self._redeem(token, username=name)
            self.assertEqual(response.status_code, 400, name)
            self.assertEqual(response.get_json()["code"], "username_unavailable")
            self.assertEqual(response.get_json()["field"], "username")
        I._reset_for_tests()
        response = self._redeem(token, username="Élodie")
        self.assertEqual(response.status_code, 201, response.get_data(as_text=True))
        self.assertEqual(response.get_json()["user"]["name"], "elodie")
        self.assertEqual(response.get_json()["user"]["onboarding"], "artists")
        self.assertFalse(response.get_json()["user"]["admin"])
        self.assertEqual(User.select().where(User.name_key == "elodie").count(), 1)

    def test_invalid_and_reserved_names_are_refused_before_consuming(self):
        invite, token = self._create()
        for name in ("ab", "a" * 33, "a b", "../evil"):
            I._reset_for_tests()
            response = self._redeem(token, username=name)
            self.assertEqual(response.status_code, 400, name)
            self.assertEqual(response.get_json()["code"], "username_invalid")
        for name in ("admin", "api", "server"):
            I._reset_for_tests()
            response = self._redeem(token, username=name)
            self.assertEqual(response.status_code, 400, name)
            self.assertEqual(response.get_json()["code"], "username_unavailable")
        self.assertIsNone(self._invite_row(invite).used_at)
        self.assertEqual(User.select().count(), 2)

    def test_the_new_account_password_policy_holds(self):
        cases = [
            ("elodie", "Abcdefghij1", "password_weak"),
            ("elodie", "administrator", "password_weak"),
            ("elodie", "Elodie2026Secret", "password_weak"),
            ("elodie", "eidole2026Secret", "password_weak"),
            ("elodie", "A" * 129, "password_weak"),
        ]
        for username, password, code in cases:
            I._reset_for_tests()
            _invite, token = self._create()
            response = self._redeem(token, username=username, password=password)
            self.assertEqual(response.status_code, 400, password)
            self.assertEqual(response.get_json()["code"], code)
            self.assertEqual(response.get_json()["field"], "password")
        I._reset_for_tests()
        _invite, token = self._create()
        mismatch = self._redeem(token, confirm="OtherCorrectHorse1")
        self.assertEqual(mismatch.status_code, 400)
        self.assertEqual(mismatch.get_json()["code"], "password_mismatch")
        self.assertEqual(mismatch.get_json()["field"], "password_confirm")
        self.assertEqual(User.select().count(), 2)

    def test_a_unicode_password_is_accepted_and_compared_as_bytes(self):
        _invite, token = self._create()
        response = self._redeem(token, password="CorrectHorseÉ1")
        self.assertEqual(response.status_code, 201, response.get_data(as_text=True))
        self.assertEqual(response.get_json()["user"]["name"], "elodie")

    def test_only_an_admin_reaches_the_admin_routes(self):
        self.assertEqual(self.bob.get("/api/admin/invites").status_code, 403)
        self.assertEqual(self.bob.post("/api/admin/invites", json={}).status_code, 403)
        anonymous = self.app.test_client()
        self.assertEqual(anonymous.get("/api/admin/invites").status_code, 401)
        self.assertEqual(anonymous.post("/api/admin/invites", json={}).status_code, 401)

    def test_the_remote_policy_classifies_the_invite_routes(self):
        endpoints = {
            rule.endpoint.rsplit(".", 1)[-1]
            for rule in self.app.url_map.iter_rules()
            if rule.rule.startswith("/api/")
        }
        self.assertEqual(sorted(endpoints - set(R.POLICY)), [])
        self.assertEqual(sorted(set(R.POLICY) - endpoints), [])
        self.assertEqual(R.POLICY["invite_inspect"], R.NEVER)
        self.assertEqual(R.POLICY["invite_redeem"], R.NEVER)
        for name in (
            "admin_invites_list",
            "admin_invite_create",
            "admin_invite_revoke",
            "admin_invite_renew",
        ):
            self.assertEqual(R.POLICY[name], "admin")
        self.assertIsNone(R.required_level("invites_api.invite_inspect", "POST"))
        self.assertIsNone(R.required_level("invites_api.invite_redeem", "POST"))
        self.assertEqual(
            R.required_level("invites_api.admin_invite_create", "POST"), "admin"
        )

    def test_both_public_routes_refuse_a_cross_site_request(self):
        _invite, token = self._create()
        evil = {"Origin": "https://evil.example"}
        response = self.app.test_client().post(
            "/api/invites/inspect", json={"token": token}, headers=evil
        )
        self.assertEqual(response.status_code, 403)
        response = self.app.test_client().post(
            "/api/invites/redeem",
            json={
                "token": token,
                "username": "elodie",
                "password": GOOD,
                "password_confirm": GOOD,
            },
            headers={"Sec-Fetch-Site": "cross-site"},
        )
        self.assertEqual(response.status_code, 403)
        response = self.alice.post("/api/admin/invites", json={}, headers=evil)
        self.assertEqual(response.status_code, 403)

    def test_an_oversized_body_is_refused_before_it_is_parsed(self):
        response = self.app.test_client().post(
            "/api/invites/inspect",
            data=b'{"token": "' + b"A" * (9 * 1024) + b'"}',
            content_type="application/json",
        )
        self.assertEqual(response.status_code, 413)
        self.assertEqual(response.get_json()["code"], "too_large")

    def test_twenty_creations_an_hour_then_429(self):
        for index in range(20):
            invite, _token = self._create(note=f"n{index}")
            self.assertEqual(invite["state"], "pending")
        response = self.alice.post("/api/admin/invites", json={})
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "rate_limited")
        self.assertTrue(response.headers.get("Retry-After"))

    def test_two_hundred_live_invitations_then_429(self):
        at = now()
        for index in range(200):
            Invite.create(
                id=f"i{index:021d}",
                token_hash=f"{index:064x}",
                created=at,
                expires=at + timedelta(days=1),
            )
        response = self.alice.post("/api/admin/invites", json={})
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "rate_limited")
        self.assertEqual(response.get_json()["reason"], "invite_cap")
        self.assertTrue(response.headers.get("Retry-After"))
        self.assertEqual(Invite.select().count(), 200)

    def test_inspect_has_its_own_limit_and_does_not_consume_redeem(self):
        for _ in range(60):
            response = self.app.test_client().post(
                "/api/invites/inspect", json={"token": "nope"}
            )
            self.assertEqual(response.status_code, 404)
        response = self.app.test_client().post(
            "/api/invites/inspect", json={"token": "nope"}
        )
        self.assertEqual(response.status_code, 429)
        self.assertTrue(response.headers.get("Retry-After"))
        redeem = self.app.test_client().post(
            "/api/invites/redeem", json={"token": "nope"}
        )
        self.assertEqual(redeem.status_code, 404)

    def test_six_failed_redeems_then_429(self):
        for _ in range(5):
            response = self.app.test_client().post(
                "/api/invites/redeem", json={"token": "nope"}
            )
            self.assertEqual(response.status_code, 404)
        response = self.app.test_client().post(
            "/api/invites/redeem", json={"token": "nope"}
        )
        self.assertEqual(response.status_code, 429)
        self.assertEqual(response.get_json()["code"], "rate_limited")
        self.assertTrue(response.headers.get("Retry-After"))

    def test_bogus_tokens_do_not_block_a_valid_invite_from_the_same_ip(self):
        _invite, token = self._create()
        for _ in range(5):
            response = self.app.test_client().post(
                "/api/invites/redeem", json={"token": "nope"}
            )
            self.assertEqual(response.status_code, 404)
        valid = self.app.test_client().post(
            "/api/invites/redeem",
            json={
                "token": token,
                "username": "nouveau",
                "password": "Cheval-Pile-Agrafe-9",
                "password_confirm": "Cheval-Pile-Agrafe-9",
            },
        )
        self.assertEqual(valid.status_code, 201, valid.get_json())

    def test_a_disabled_account_gets_the_generic_401_and_loses_its_session(self):
        User.update(disabled=True).where(User.name == "bob").execute()
        response = self.app.test_client().post(
            "/api/login", json={"username": "bob", "password": "B0bbbpass"}
        )
        self.assertEqual(response.status_code, 401)
        self.assertEqual(response.get_json(), {"error": "invalid credentials"})
        self.assertEqual(self.bob.get("/api/me").status_code, 401)
        User.update(disabled=False).where(User.name == "bob").execute()
        self.assertEqual(self._login("bob", "B0bbbpass").get("/api/me").status_code, 200)

    def test_last_login_is_written_at_most_once_an_hour(self):
        first = User.get(User.name == "alice").last_login
        self.assertIsNotNone(first)
        self._login("alice", "Alic3pass")
        self.assertEqual(User.get(User.name == "alice").last_login, first)
        User.update(last_login=now() - timedelta(hours=2)).where(
            User.name == "alice"
        ).execute()
        UserManager.reset_last_login_cache()
        self._login("alice", "Alic3pass")
        self.assertGreater(
            User.get(User.name == "alice").last_login,
            now() - timedelta(minutes=1),
        )

    def test_the_session_payloads_carry_the_onboarding_step(self):
        self.assertEqual(
            self.bob.get("/api/me").get_json()["user"]["onboarding"], "done"
        )
        login = self.app.test_client().post(
            "/api/login", json={"username": "bob", "password": "B0bbbpass"}
        )
        self.assertEqual(login.get_json()["user"]["onboarding"], "done")
        _invite, token = self._create()
        redeemed = self._redeem(token)
        self.assertEqual(redeemed.get_json()["user"]["onboarding"], "artists")

    def test_the_list_is_keyset_ordered_and_costs_two_statements(self):
        self._create("a")
        self._create("b")
        with count_statements() as counts:
            response = self.alice.get("/api/admin/invites?limit=1")
        self.assertEqual(response.status_code, 200)
        self.assertEqual(counts["SELECT"], 3)
        page = response.get_json()
        self.assertEqual(len(page["invites"]), 1)
        self.assertIsNotNone(page["next"])
        second = self.alice.get(
            "/api/admin/invites",
            query_string={
                "limit": 1,
                "after_created": page["next"]["created"],
                "after_id": page["next"]["id"],
            },
        )
        self.assertEqual(second.status_code, 200)
        self.assertEqual(len(second.get_json()["invites"]), 1)
        self.assertNotEqual(
            page["invites"][0]["id"], second.get_json()["invites"][0]["id"]
        )
        self.assertIsNone(second.get_json()["next"])

    def test_states_are_filtered_and_ordered_by_precedence(self):
        pending = self._create()[0]
        revoked = self._create()[0]
        used, used_token = self._create()
        self.alice.post(f"/api/admin/invites/{revoked['id']}/revoke")
        self._redeem(used_token)

        states = {
            row["id"]: row["state"]
            for row in self.alice.get("/api/admin/invites").get_json()["invites"]
        }
        self.assertEqual(states[pending["id"]], "pending")
        self.assertEqual(states[revoked["id"]], "revoked")
        self.assertEqual(states[used["id"]], "used")
        filtered = self.alice.get("/api/admin/invites?state=used").get_json()
        self.assertEqual([row["id"] for row in filtered["invites"]], [used["id"]])
        invalid = self.alice.get("/api/admin/invites?state=nope")
        self.assertEqual(invalid.status_code, 400)

    def test_the_schema_upgrade_leaves_existing_accounts_done(self):
        self.assertGreaterEqual(SCHEMA_VERSION, "20261118")
        self.assertTrue(Invite.table_exists())
        me = self.bob.get("/api/me").get_json()["user"]
        self.assertEqual(me["onboarding"], "done")
        _invite, token = self._create()
        new = self._redeem(token).get_json()["user"]
        self.assertEqual(new["onboarding"], "artists")
        self.assertNotEqual(
            new["name"], self.bob.get("/api/me").get_json()["user"]["name"]
        )
