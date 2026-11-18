# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The `/api/lan/*` routes: enrolment, roster, devices, friends and settings.

What is pinned here is exactly what the trust chain must never get wrong: a
refusal never reveals whether an account exists, a bad or stale proof is
refused before a row is written, a renewal rotates the key while a revocation
names EVERY key a device held, a device of another account is a 404, a
remote-control session can reach none of it, and the two admin settings are in
``Meta`` (rule 6) and change the NEXT credential.

The bodies are the ones `nz-model/src/lan.rs` parses; the friends' ``user_id``
is the ACCOUNT's UUID (never the LAN subject, which travels in its own field).

A test database lives under ``~/.cache``, never in ``/tmp``.
"""

import base64
import hashlib
import json
import logging
import os
import shutil
import tempfile
import time
import unittest

from Crypto.Signature import eddsa

from neezer import lan
from neezer.config import DefaultConfig
from neezer.db import (
    Album,
    Artist,
    Folder,
    LanDevice,
    LanDeviceKey,
    LanFriend,
    LanFriendPlaylist,
    Meta,
    Playlist,
    PlaylistTrack,
    Track,
    User,
    db,
    release_database,
)
from neezer.managers.user import UserManager
from neezer.web import create_application
from neezer.webui import lan as routes
from neezer.webui import remote

ROOT = os.path.join(os.path.expanduser("~"), ".cache", "neezer-lan-tests")
DEVICE = "livingroom-pc-0001"


def _seed(index):
    return hashlib.sha256(b"lan-test-device-%d" % index).digest()


class LanRouteCase(unittest.TestCase):
    def setUp(self):
        os.makedirs(ROOT, exist_ok=True)
        self.directory = tempfile.mkdtemp(prefix="routes-", dir=ROOT)
        db_path = os.path.join(self.directory, "neezer.db")
        cache = os.path.join(self.directory, "cache")
        vars(__import__("neezer.utils", fromlist=["utils"]))["__key_cache"].clear()
        lan._reset_for_tests()
        routes._reset_for_tests()
        remote._reset_for_tests()

        class Config(DefaultConfig):
            TESTING = True

            def __init__(self):
                super().__init__()
                self.BASE = dict(self.BASE, database_uri="sqlite:///" + db_path)
                self.WEBAPP = dict(
                    self.WEBAPP,
                    cache_dir=cache,
                    mount_webui=True,
                    mount_api=True,
                )

        self.app = create_application(Config())
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb", admin=False)
        self.alice = User.get(User.name == "alice")
        self.bob = User.get(User.name == "bob")
        self.alice_client = self._login("alice", "Alic3")
        self.bob_client = self._login("bob", "B0bbb")

    def tearDown(self):
        routes._reset_for_tests()
        lan._reset_for_tests()
        remote._reset_for_tests()
        release_database()
        shutil.rmtree(self.directory, ignore_errors=True)

    # -- fixtures ---------------------------------------------------------------

    def _login(self, name, password):
        client = self.app.test_client()
        answer = client.post("/api/login", json={"username": name, "password": password})
        self.assertEqual(answer.status_code, 200, answer.get_json())
        return client

    def _status(self, client):
        answer = client.get("/api/lan/status")
        self.assertEqual(answer.status_code, 200, answer.get_json())
        return answer.get_json()

    def _enroll(
        self,
        client,
        seed,
        name="PC du salon",
        platform="desktop",
        device_id=None,
        at=None,
        subject_hex=None,
    ):
        key = lan.LanKey(seed)
        pubkey = key.pub.hex()
        at = int(time.time()) if at is None else int(at)
        if subject_hex is None:
            subject_hex = self._status(client)["subject"]
        proof = key.sign(lan.enroll_message(subject_hex, pubkey, at))
        body = {
            "pubkey": pubkey,
            "name": name,
            "platform": platform,
            "at": at,
            "proof": base64.b64encode(proof).decode(),
        }
        if device_id is not None:
            body["device_id"] = device_id
        return client.post("/api/lan/enroll", json=body)

    def _enrolled(self, client, seed, **kwargs):
        answer = self._enroll(client, seed, **kwargs)
        self.assertEqual(answer.status_code, 200, answer.get_json())
        return answer.get_json()

    def _revocations(self, client):
        answer = client.get("/api/lan/roster")
        self.assertEqual(answer.status_code, 200, answer.get_json())
        return base64.b64decode(answer.get_json()["revocations"])

    def _revoked_keys(self, client):
        blob = self._revocations(client)
        count = int.from_bytes(blob[29:33], "big")
        return [blob[33 + 32 * i : 33 + 32 * (i + 1)] for i in range(count)]

    def _roster(self, client):
        answer = client.get("/api/lan/roster")
        self.assertEqual(answer.status_code, 200, answer.get_json())
        return json.loads(base64.b64decode(answer.get_json()["roster"]["body"]))

    def _library(self):
        if not hasattr(self, "_root"):
            self._root = Folder.create(root=True, name="root", path="/music")
            self._folder = Folder.create(
                root=False, name="A", path="/music/a", parent=self._root
            )
            self._artist = Artist.create(name="Artiste")
            self._album = Album.create(name="Album", artist=self._artist)
        return self._root, self._folder, self._artist, self._album

    def _playlist(self, user, name, tracks=0):
        root, folder, artist, album = self._library()
        playlist = Playlist.create(user=user, name=name)
        with db.atomic():
            for index in range(tracks):
                track = Track.create(
                    disc=1,
                    number=index + 1,
                    title=f"Titre {index}",
                    duration=200,
                    album=album,
                    artist=artist,
                    bitrate=900,
                    path=f"/music/a/{name}-{index}-{os.urandom(4).hex()}.flac",
                    last_modification=0,
                    root_folder=root,
                    folder=folder,
                )
                PlaylistTrack.create(playlist=playlist, track=track, index=index)
        return playlist

    def _remote_session(self):
        """A queue-level remote link claimed by its own client."""
        answer = self.alice_client.post(
            "/api/remote/links",
            json={"level": "queue", "device": DEVICE, "device_name": "PC"},
        )
        self.assertEqual(answer.status_code, 200, answer.get_json())
        client = self.app.test_client()
        claimed = client.post(
            "/api/remote/claim", json={"token": answer.get_json()["link"]["token"]}
        )
        self.assertEqual(claimed.status_code, 200, claimed.get_json())
        return client


class PolicyTestCase(LanRouteCase):
    def test_every_lan_route_is_classified_and_never_lent(self):
        endpoints = {
            rule.endpoint.rsplit(".", 1)[-1]
            for rule in self.app.url_map.iter_rules()
            if rule.rule.startswith("/api/lan/")
        }
        self.assertEqual(len(endpoints), 12, endpoints)
        self.assertEqual(set(routes.LAN_POLICY), endpoints)
        for name, level in routes.LAN_POLICY.items():
            self.assertEqual(level, remote.NEVER, name)
        self.assertEqual(
            {name: remote.POLICY.get(name) for name in endpoints},
            {name: remote.NEVER for name in endpoints},
        )

    def test_every_route_answers_401_without_a_session(self):
        anonymous = self.app.test_client()
        calls = (
            ("get", "/api/lan/status", None),
            ("post", "/api/lan/enroll", {}),
            ("get", "/api/lan/roster", None),
            ("get", "/api/lan/devices", None),
            ("patch", "/api/lan/devices/0123456789abcdef01234567", {"name": "x"}),
            ("delete", "/api/lan/devices/0123456789abcdef01234567", None),
            ("get", "/api/lan/friends", None),
            ("post", "/api/lan/friends", {"name": "bob"}),
            ("patch", "/api/lan/friends/%s" % self.bob.id, {"all": True}),
            ("delete", "/api/lan/friends/%s" % self.bob.id, None),
            ("get", "/api/lan/settings", None),
            ("put", "/api/lan/settings", {"enabled": True}),
            ("post", "/api/lan/rotate", None),
        )
        for method, path, body in calls:
            answer = getattr(anonymous, method)(path, json=body)
            self.assertEqual(answer.status_code, 401, (method, path))

    def test_a_remote_control_session_reaches_none_of_it(self):
        controller = self._remote_session()
        for method, path, body in (
            ("get", "/api/lan/status", None),
            ("post", "/api/lan/enroll", {}),
            ("get", "/api/lan/roster", None),
            ("get", "/api/lan/devices", None),
            ("get", "/api/lan/friends", None),
            ("post", "/api/lan/friends", {"name": "alice"}),
            ("get", "/api/lan/settings", None),
            ("post", "/api/lan/rotate", None),
        ):
            answer = getattr(controller, method)(path, json=body)
            self.assertEqual(answer.status_code, 403, (method, path))
        # The owner's own session still reaches it.
        self.assertEqual(self.alice_client.get("/api/lan/status").status_code, 200)

    def test_options_on_the_status_probe_answers_200(self):
        self.assertEqual(
            self.app.test_client().options("/api/lan/status").status_code, 200
        )


class StatusTestCase(LanRouteCase):
    def test_the_status_carries_the_subject_the_keys_and_the_days(self):
        status = self._status(self.alice_client)
        self.assertTrue(status["enabled"])
        self.assertRegex(status["subject"], r"\A[0-9a-f]{32}\Z")
        self.assertEqual(len(status["subject"]), 32)
        self.assertEqual(len(status["keys"]), 1)
        key = status["keys"][0]
        self.assertRegex(key["kid"], r"\A[0-9a-f]{16}\Z")
        self.assertRegex(key["pub"], r"\A[0-9a-f]{64}\Z")
        self.assertIsNone(key["until"])
        self.assertEqual(status["credential_days"], 30)
        self.assertEqual(status["revocation_serial"], 0)
        # The subject is this account's and nobody else's.
        self.assertNotEqual(status["subject"], self._status(self.bob_client)["subject"])


class EnrollTestCase(LanRouteCase):
    def test_enrolment_answers_a_credential_the_note_can_verify(self):
        answer = self._enrolled(self.alice_client, _seed(1))
        self.assertRegex(answer["device_id"], r"\A[0-9a-f]{24}\Z")
        credential = base64.b64decode(answer["credential"])
        self.assertEqual(len(credential), 154)
        self.assertEqual(credential[:4], b"NZLC")
        self.assertEqual(credential[4], 1)
        status = self._status(self.alice_client)
        keys = {key["kid"]: key["pub"] for key in status["keys"]}
        kid = credential[5:13].hex()
        self.assertIn(kid, keys)
        eddsa.new(
            eddsa.import_public_key(bytes.fromhex(keys[kid])), "rfc8032"
        ).verify(
            lan.DOMAIN_CREDENTIAL + credential[:90], credential[90:]
        )
        self.assertEqual(credential[13:29].hex(), status["subject"])
        self.assertEqual(credential[29:41].hex(), answer["device_id"])
        self.assertEqual(credential[41:73].hex(), lan.LanKey(_seed(1)).pub.hex())
        not_before = int.from_bytes(credential[73:81], "big")
        not_after = int.from_bytes(credential[81:89], "big")
        self.assertLessEqual(abs(not_before - int(time.time())), 60)
        self.assertLessEqual(
            abs((not_after - not_before) - 30 * 86400), 5
        )
        self.assertEqual(credential[89], 3)
        # The roster and the revocation list come along, signed, so a login
        # costs no second round trip.
        roster = json.loads(base64.b64decode(answer["roster"]["body"]))
        self.assertEqual(roster["subject"], status["subject"])
        eddsa.new(
            eddsa.import_public_key(bytes.fromhex(keys[roster["kid"]])), "rfc8032"
        ).verify(
            lan.DOMAIN_ROSTER + base64.b64decode(answer["roster"]["body"]),
            base64.b64decode(answer["roster"]["sig"]),
        )
        self.assertEqual(base64.b64decode(answer["revocations"])[:4], b"NZLR")

        # The device is listed, and `current` names the one that asked.
        devices = self.alice_client.get("/api/lan/devices").get_json()
        self.assertEqual(len(devices), 1)
        self.assertEqual(devices[0]["id"], answer["device_id"])
        self.assertTrue(devices[0]["current"])
        self.assertEqual(devices[0]["name"], "PC du salon")
        self.assertEqual(devices[0]["platform"], "desktop")
        self.assertIsNone(devices[0]["revoked"])

    def test_a_bad_proof_is_refused_with_a_french_cause(self):
        other = lan.LanKey(_seed(99))
        pubkey = lan.LanKey(_seed(2)).pub.hex()
        at = int(time.time())
        subject = self._status(self.alice_client)["subject"]
        body = {
            "pubkey": pubkey,
            "name": "PC",
            "platform": "desktop",
            "at": at,
            "proof": base64.b64encode(
                other.sign(lan.enroll_message(subject, pubkey, at))
            ).decode(),
        }
        answer = self.alice_client.post("/api/lan/enroll", json=body)
        self.assertEqual(answer.status_code, 403)
        payload = answer.get_json()
        self.assertEqual(payload["code"], "bad_proof")
        self.assertIn("preuve", payload["error"])
        self.assertEqual(LanDevice.select().count(), 0)

        # Someone else's subject: the proof is valid for THAT account, so it is
        # a bad proof for this one — and the refusal says nothing more.
        bob_subject = self._status(self.bob_client)["subject"]
        answer = self._enroll(self.alice_client, _seed(2), subject_hex=bob_subject)
        self.assertEqual(answer.status_code, 403)
        self.assertEqual(answer.get_json(), payload)
        self.assertEqual(LanDevice.select().count(), 0)

    def test_a_proof_301_seconds_old_is_refused(self):
        answer = self._enroll(
            self.alice_client, _seed(3), at=int(time.time()) - 301
        )
        self.assertEqual(answer.status_code, 403)
        self.assertEqual(answer.get_json()["code"], "stale_proof")
        self.assertIn("300", answer.get_json()["error"])
        self.assertEqual(LanDevice.select().count(), 0)
        # 295 s is inside the window (a second of slack either side: the server
        # reads its own clock, and 300 exactly is the boundary the crypto test
        # pins deterministically).
        self.assertEqual(
            self._enroll(self.alice_client, _seed(3), at=int(time.time()) - 295).status_code,
            200,
        )

    def test_bad_fields_are_refused_before_a_row_is_written(self):
        at = int(time.time())
        subject = self._status(self.alice_client)["subject"]
        key = lan.LanKey(_seed(4))
        for override in (
            {"pubkey": "zz" * 32},
            {"pubkey": key.pub.hex().upper()},
            {"platform": "windows"},
            {"name": ""},
            {"name": "x" * 65},
            {"at": "now"},
            {"proof": "not base64 at all"},
        ):
            body = {
                "pubkey": key.pub.hex(),
                "name": "PC",
                "platform": "desktop",
                "at": at,
                "proof": base64.b64encode(
                    key.sign(lan.enroll_message(subject, key.pub.hex(), at))
                ).decode(),
            }
            body.update(override)
            answer = self.alice_client.post("/api/lan/enroll", json=body)
            self.assertIn(answer.status_code, (400, 403), override)
            self.assertIn("error", answer.get_json())
        self.assertEqual(LanDevice.select().count(), 0)

    def test_a_21st_live_device_is_refused_with_a_cause(self):
        for index in range(lan.MAX_DEVICES):
            self._enrolled(self.alice_client, _seed(100 + index), name=f"PC {index}")
        self.assertEqual(LanDevice.select().count(), lan.MAX_DEVICES)
        answer = self._enroll(self.alice_client, _seed(200), name="PC de trop")
        self.assertEqual(answer.status_code, 409)
        self.assertEqual(answer.get_json()["code"], "too_many_devices")
        self.assertIn("20", answer.get_json()["error"])
        self.assertEqual(LanDevice.select().count(), lan.MAX_DEVICES)
        # Another account is not affected by this one's cap.
        self.assertEqual(
            self._enroll(self.bob_client, _seed(201)).status_code, 200
        )

    def test_a_key_already_enrolled_is_refused_even_across_accounts(self):
        self._enrolled(self.alice_client, _seed(5))
        answer = self._enroll(self.bob_client, _seed(5))
        self.assertEqual(answer.status_code, 409)
        self.assertEqual(answer.get_json()["code"], "key_in_use")
        self.assertEqual(LanDevice.select().count(), 1)

    def test_a_rotation_names_the_old_key_only_after_a_revoke(self):
        first = self._enrolled(self.alice_client, _seed(6))
        device = first["device_id"]
        old_key = bytes.fromhex(lan.LanKey(_seed(6)).pub.hex())
        new_key = bytes.fromhex(lan.LanKey(_seed(7)).pub.hex())
        second = self._enrolled(
            self.alice_client, _seed(7), device_id=device, name="PC renommé"
        )
        self.assertEqual(second["device_id"], device)
        current = LanDevice.get(LanDevice.id == device)
        self.assertEqual(current.pubkey, new_key.hex())
        self.assertEqual(current.name, "PC renommé")
        # Both keys are remembered, and NEITHER is revoked: an old credential is
        # not a revocation, it simply expires.
        self.assertEqual(
            {row[0] for row in LanDeviceKey.select(LanDeviceKey.pubkey).tuples()},
            {old_key.hex(), new_key.hex()},
        )
        self.assertEqual(self._revoked_keys(self.alice_client), [])
        self.assertEqual(
            int.from_bytes(self._revocations(self.alice_client)[13:21], "big"), 0
        )
        # The new credential is signed over the NEW key.
        credential = base64.b64decode(second["credential"])
        self.assertEqual(credential[41:73], new_key)

        answer = self.alice_client.delete(f"/api/lan/devices/{device}")
        self.assertEqual(answer.status_code, 204)
        self.assertEqual(answer.get_data(), b"")
        revoked = self._revoked_keys(self.alice_client)
        self.assertEqual(sorted(revoked), sorted([old_key, new_key]))
        blob = self._revocations(self.alice_client)
        self.assertEqual(int.from_bytes(blob[13:21], "big"), 1)
        self.assertNotEqual(LanDevice.get(LanDevice.id == device).revoked, None)
        # A revoked device cannot come back with its old key, nor with a new one.
        answer = self._enroll(self.alice_client, _seed(7), device_id=device)
        self.assertEqual(answer.status_code, 409)
        answer = self._enroll(self.alice_client, _seed(8), device_id=device)
        self.assertEqual(answer.status_code, 409)
        self.assertEqual(answer.get_json()["code"], "device_revoked")

    def test_delete_self_revokes_the_device_this_browser_enrolled(self):
        answer = self._enrolled(self.alice_client, _seed(9))
        revoked = self.alice_client.delete("/api/lan/devices/self")
        self.assertEqual(revoked.status_code, 204)
        devices = self.alice_client.get("/api/lan/devices").get_json()
        self.assertEqual(devices[0]["revoked"] is not None, True)
        # ...and 'self' is 404 once this browser has not enrolled anything.
        fresh = self._login("alice", "Alic3")
        self.assertEqual(fresh.delete("/api/lan/devices/self").status_code, 404)
        self.assertEqual(answer["device_id"], devices[0]["id"])

    def test_a_device_of_another_account_is_a_404(self):
        answer = self._enrolled(self.alice_client, _seed(10))
        device = answer["device_id"]
        self.assertEqual(
            self.bob_client.delete(f"/api/lan/devices/{device}").status_code, 404
        )
        self.assertEqual(
            self.bob_client.patch(
                f"/api/lan/devices/{device}", json={"name": "volé"}
            ).status_code,
            404,
        )
        # Unchanged: still owned by alice, still live.
        row = LanDevice.get(LanDevice.id == device)
        self.assertIsNone(row.revoked)
        self.assertEqual(row.name, "PC du salon")
        # A malformed id is the same 404, not a 500.
        self.assertEqual(
            self.bob_client.delete("/api/lan/devices/not-an-id").status_code, 404
        )

    def test_rename_accepts_one_to_sixty_four_characters(self):
        device = self._enrolled(self.alice_client, _seed(11))["device_id"]
        answer = self.alice_client.patch(
            f"/api/lan/devices/{device}", json={"name": "  Le PC  du salon  "}
        )
        self.assertEqual(answer.status_code, 200, answer.get_json())
        self.assertEqual(answer.get_json()["name"], "Le PC du salon")
        for body in ({"name": ""}, {"name": "   "}, {"name": "x" * 65}, {}):
            self.assertEqual(
                self.alice_client.patch(
                    f"/api/lan/devices/{device}", json=body
                ).status_code,
                400,
                body,
            )

    def test_the_enrol_budget_is_thirty_an_hour(self):
        device = self._enrolled(self.alice_client, _seed(12))["device_id"]
        for index in range(29):
            answer = self._enroll(
                self.alice_client, _seed(1000 + index), device_id=device
            )
            self.assertEqual(answer.status_code, 200, (index, answer.get_json()))
        refused = self._enroll(
            self.alice_client, _seed(2000), device_id=device
        )
        self.assertEqual(refused.status_code, 429)
        self.assertEqual(refused.get_json()["code"], "rate_limited")
        self.assertIn("Retry-After", refused.headers)
        self.assertGreaterEqual(int(refused.headers["Retry-After"]), 1)
        # Another account has its own budget.
        self.assertEqual(
            self._enroll(self.bob_client, _seed(3000)).status_code, 200
        )


class FriendsTestCase(LanRouteCase):
    def test_an_unknown_self_or_disabled_account_is_the_same_404(self):
        unknown = self.alice_client.post("/api/lan/friends", json={"name": "carol"})
        itself = self.alice_client.post("/api/lan/friends", json={"name": "alice"})
        User.update(disabled=True).where(User.id == self.bob.id).execute()
        disabled = self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        User.update(disabled=False).where(User.id == self.bob.id).execute()
        self.assertEqual(unknown.status_code, 404)
        for answer in (itself, disabled):
            self.assertEqual(answer.status_code, 404)
            self.assertEqual(answer.get_json(), unknown.get_json())
            self.assertEqual(answer.get_data(), unknown.get_data())
        self.assertEqual(LanFriend.select().count(), 0)

    def test_a_friend_is_added_by_folded_name_and_listed_both_ways(self):
        answer = self.alice_client.post("/api/lan/friends", json={"name": "BOB"})
        self.assertEqual(answer.status_code, 200, answer.get_json())
        friend = answer.get_json()
        self.assertEqual(friend["user_id"], str(self.bob.id))
        self.assertEqual(friend["name"], "bob")
        self.assertFalse(friend["all"])
        self.assertEqual(friend["playlists"], [])
        self.assertEqual(
            friend["subject"], lan.subject_hex(self.bob)
        )
        alice_view = self.alice_client.get("/api/lan/friends").get_json()
        self.assertEqual([f["user_id"] for f in alice_view["shares_to"]], [str(self.bob.id)])
        self.assertEqual(alice_view["trusted_by"], [])
        bob_view = self.bob_client.get("/api/lan/friends").get_json()
        self.assertEqual(bob_view["shares_to"], [])
        self.assertEqual(
            [f["user_id"] for f in bob_view["trusted_by"]], [str(self.alice.id)]
        )
        # Adding twice is the same friend, not a duplicate and not an error.
        again = self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        self.assertEqual(again.status_code, 200)
        self.assertEqual(LanFriend.select().count(), 1)

    def test_a_playlist_the_owner_cannot_read_is_refused(self):
        self.assertEqual(
            self.alice_client.post("/api/lan/friends", json={"name": "bob"}).status_code,
            200,
        )
        # An ADMIN reads every playlist (`playlist_access`'s override), so the
        # refused case must be a non-admin owner: bob cannot read alice's.
        theirs = self._playlist(self.alice, "À moi")
        self.assertEqual(
            self.bob_client.post("/api/lan/friends", json={"name": "alice"}).status_code,
            200,
        )
        answer = self.bob_client.patch(
            f"/api/lan/friends/{self.alice.id}",
            json={"all": False, "playlists": [str(theirs.id)]},
        )
        self.assertEqual(answer.status_code, 404)
        self.assertEqual(answer.get_json()["code"], "playlist_not_found")
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        # An unknown id is the very same answer.
        unknown = self.bob_client.patch(
            f"/api/lan/friends/{self.alice.id}",
            json={"all": False, "playlists": ["00000000-0000-0000-0000-000000000000"]},
        )
        self.assertEqual(unknown.get_data(), answer.get_data())
        # And his own playlist is accepted.
        mine = self._playlist(self.bob, "À moi aussi", tracks=1)
        self.assertEqual(
            self.bob_client.patch(
                f"/api/lan/friends/{self.alice.id}",
                json={"all": False, "playlists": [str(mine.id)]},
            ).status_code,
            200,
        )

    def test_the_scope_lists_exactly_the_shared_tracks_in_the_roster(self):
        mine = self._playlist(self.alice, "Route", tracks=3)
        self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        answer = self.alice_client.patch(
            f"/api/lan/friends/{self.bob.id}",
            json={"all": False, "playlists": [str(mine.id)]},
        )
        self.assertEqual(answer.status_code, 200, answer.get_json())
        self.assertEqual(
            answer.get_json()["playlists"],
            [{"id": str(mine.id), "title": "Route"}],
        )
        roster = self._roster(self.alice_client)
        share = roster["shares_to"][0]
        self.assertFalse(share["all"])
        self.assertEqual(share["playlists"][0]["title"], "Route")
        self.assertEqual(
            sorted(share["playlists"][0]["tracks"]),
            sorted(
                str(track.id)
                for track in Track.select()
                .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
                .where(PlaylistTrack.playlist == mine.id)
            ),
        )
        # 'all' drops the playlist list: everything is shared.
        answer = self.alice_client.patch(
            f"/api/lan/friends/{self.bob.id}", json={"all": True}
        )
        self.assertTrue(answer.get_json()["all"])
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        share = self._roster(self.alice_client)["shares_to"][0]
        self.assertTrue(share["all"])
        self.assertEqual(share["playlists"], [])

    def test_adding_and_removing_a_friend_bumps_both_rosters(self):
        def serials():
            return (
                self._roster(self.alice_client)["serial"],
                self._roster(self.bob_client)["serial"],
            )

        before = serials()
        self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        after_add = serials()
        self.assertEqual(after_add[0], before[0] + 1)
        self.assertEqual(after_add[1], before[1] + 1)
        playlist = self._playlist(self.alice, "Route", tracks=1)
        self.alice_client.patch(
            f"/api/lan/friends/{self.bob.id}",
            json={"all": False, "playlists": [str(playlist.id)]},
        )
        after_scope = serials()
        self.assertEqual(after_scope[0], after_add[0] + 1)
        self.assertEqual(after_scope[1], after_add[1] + 1)
        answer = self.alice_client.delete(f"/api/lan/friends/{self.bob.id}")
        self.assertEqual(answer.status_code, 204)
        after_remove = serials()
        self.assertEqual(after_remove[0], after_scope[0] + 1)
        self.assertEqual(after_remove[1], after_scope[1] + 1)
        self.assertEqual(LanFriend.select().count(), 0)
        self.assertEqual(LanFriendPlaylist.select().count(), 0)
        # Removing again is a 404, and changes nothing.
        self.assertEqual(
            self.alice_client.delete(f"/api/lan/friends/{self.bob.id}").status_code,
            404,
        )

    def test_the_friend_budget_is_thirty_an_hour(self):
        for index in range(30):
            name = f"ami{index}"
            UserManager.add(name, "Passw0rd!", admin=False)
            answer = self.alice_client.post("/api/lan/friends", json={"name": name})
            self.assertEqual(answer.status_code, 200, (index, answer.get_json()))
        UserManager.add("ami30", "Passw0rd!", admin=False)
        refused = self.alice_client.post("/api/lan/friends", json={"name": "ami30"})
        self.assertEqual(refused.status_code, 429)
        self.assertIn("Retry-After", refused.headers)


class RosterRouteTestCase(LanRouteCase):
    def test_the_etag_answers_a_304_with_zero_bytes(self):
        answer = self.alice_client.get("/api/lan/roster")
        self.assertEqual(answer.status_code, 200)
        etag = answer.headers["ETag"]
        self.assertRegex(etag, r'\A"[0-9]+-[0-9]+"\Z')
        self.assertEqual(answer.get_json()["etag"], etag)
        again = self.alice_client.get(
            "/api/lan/roster", headers={"If-None-Match": etag}
        )
        self.assertEqual(again.status_code, 304)
        self.assertEqual(again.get_data(), b"")
        self.assertEqual(again.headers["ETag"], etag)
        # A weak validator of the same tag is the same thing to a browser.
        weak = self.alice_client.get(
            "/api/lan/roster", headers={"If-None-Match": "W/" + etag}
        )
        self.assertEqual(weak.status_code, 304)
        # A friend added moves the roster's serial, so the old tag is stale.
        self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        fresh = self.alice_client.get(
            "/api/lan/roster", headers={"If-None-Match": etag}
        )
        self.assertEqual(fresh.status_code, 200)
        self.assertNotEqual(fresh.headers["ETag"], etag)

    def test_a_four_thousand_track_share_stays_a_reasonable_answer(self):
        playlist = self._playlist(self.alice, "Route", tracks=4_000)
        self.alice_client.post("/api/lan/friends", json={"name": "bob"})
        self.alice_client.patch(
            f"/api/lan/friends/{self.bob.id}",
            json={"all": False, "playlists": [str(playlist.id)]},
        )
        answer = self.alice_client.get("/api/lan/roster")
        self.assertEqual(answer.status_code, 200)
        payload = answer.get_json()
        body = base64.b64decode(payload["roster"]["body"])
        self.assertEqual(
            len(json.loads(body)["shares_to"][0]["playlists"][0]["tracks"]), 4_000
        )
        self.assertLessEqual(len(body), 200_000, f"body is {len(body)} B")
        # The base64 envelope the note mandates is 4/3 of the signed body; the
        # server gzips a JSON answer of this size before sending it.
        self.assertLessEqual(len(answer.get_data()), 4 * len(body) // 3 + 4096)


class DeleteCostTestCase(LanRouteCase):
    """Deleting an account must not cost a statement per enrolled device.

    Peewee's recursive delete emits ONE ``DELETE`` per reverse relation, and
    the engine's own ``ON DELETE CASCADE`` removes that relation's rows. The
    LAN lot adds four such relations, which moved `tests/test_admin_users.py`'s
    pinned 58 to 65 — but the number must be the same for an account with no
    device and one with twenty, or an admin deleting a person who enrolled
    twenty devices would pay twenty statements.

    Measured (SQLite, 2026-11-18): **60 statements either way** — 4 SELECT,
    10 UPDATE, 46 DELETE, with 20 devices as with none. The four relations cost
    their one DELETE each, and the engine's CASCADE removes the rows.
    """

    def test_the_cost_is_the_same_with_twenty_devices_and_with_none(self):
        from tests.test_deezer import count_statements

        for index in range(lan.MAX_DEVICES):
            self._enrolled(self.bob_client, _seed(600 + index))
        UserManager.add("carol", "Passw0rd!", admin=False)
        carol = User.get(User.name == "carol")
        self.assertEqual(
            LanDevice.select().where(LanDevice.user == self.bob).count(),
            lan.MAX_DEVICES,
        )
        self.assertEqual(
            LanDeviceKey.select()
            .join(LanDevice)
            .where(LanDevice.user == self.bob)
            .count(),
            lan.MAX_DEVICES,
        )
        with count_statements() as many:
            UserManager.delete_guarded(self.bob.id, self.alice)
        with count_statements() as none:
            UserManager.delete_guarded(carol.id, self.alice)
        self.assertEqual(sum(many.values()), sum(none.values()), (dict(many), dict(none)))
        # The rows really went, through the engine's cascade.
        self.assertEqual(
            LanDevice.select().where(LanDevice.user == self.bob).count(), 0
        )
        self.assertEqual(
            LanDeviceKey.select().join(LanDevice).count(), 0
        )


class SettingsTestCase(LanRouteCase):
    def test_the_settings_are_admin_only_and_land_in_meta(self):
        self.assertEqual(self.alice_client.get("/api/lan/settings").status_code, 200)
        self.assertEqual(self.bob_client.get("/api/lan/settings").status_code, 403)
        self.assertEqual(
            self.bob_client.put(
                "/api/lan/settings", json={"enabled": True}
            ).status_code,
            403,
        )
        answer = self.alice_client.put(
            "/api/lan/settings", json={"enabled": True, "credential_days": 7}
        )
        self.assertEqual(answer.status_code, 200, answer.get_json())
        self.assertEqual(
            answer.get_json(), {"enabled": True, "credential_days": 7}
        )
        self.assertEqual(Meta.get(Meta.key == "lan_enabled").value, "1")
        self.assertEqual(Meta.get(Meta.key == "lan_credential_days").value, "7")
        # The NEXT credential carries the new lifetime; nothing already issued
        # is rewritten.
        credential = base64.b64decode(
            self._enrolled(self.alice_client, _seed(20))["credential"]
        )
        lifetime = int.from_bytes(credential[81:89], "big") - int.from_bytes(
            credential[73:81], "big"
        )
        self.assertLessEqual(abs(lifetime - 7 * 86400), 5)
        for days in (6, 91, "trente", True):
            self.assertEqual(
                self.alice_client.put(
                    "/api/lan/settings", json={"credential_days": days}
                ).status_code,
                400,
                days,
            )
        self.assertEqual(
            self.alice_client.put(
                "/api/lan/settings", json={"enabled": "yes"}
            ).status_code,
            400,
        )

    def test_the_feature_switch_hides_the_offer_and_refuses_enrolment(self):
        device = self._enrolled(self.alice_client, _seed(22))
        self.alice_client.put("/api/lan/settings", json={"enabled": False})
        self.assertFalse(self._status(self.alice_client)["enabled"])
        self.assertFalse(self._status(self.bob_client)["enabled"])
        refused = self._enroll(self.alice_client, _seed(21))
        self.assertEqual(refused.status_code, 403)
        self.assertEqual(refused.get_json()["code"], "disabled")
        self.assertIn("désactivé", refused.get_json()["error"])
        self.assertEqual(LanDevice.select().count(), 1)  # the one above, no more
        # Revoking and reading still work: an admin must be able to cut a
        # device's access while the feature is switched off.
        self.assertEqual(
            self.alice_client.delete(f"/api/lan/devices/{device['device_id']}").status_code,
            204,
        )
        self.assertEqual(self.alice_client.get("/api/lan/friends").status_code, 200)


class RotateTestCase(LanRouteCase):
    def test_rotate_is_admin_only_and_keeps_the_previous_key(self):
        before = self._status(self.alice_client)["keys"][0]
        self.assertEqual(self.bob_client.post("/api/lan/rotate").status_code, 403)
        answer = self.alice_client.post("/api/lan/rotate")
        self.assertEqual(answer.status_code, 200, answer.get_json())
        new_kid = answer.get_json()["kid"]
        self.assertRegex(new_kid, r"\A[0-9a-f]{16}\Z")
        keys = self._status(self.alice_client)["keys"]
        self.assertEqual([key["kid"] for key in keys], [new_kid, before["kid"]])
        self.assertIsNone(keys[0]["until"])
        self.assertGreater(keys[1]["until"], int(time.time()) + 89 * 86400)
        # A device enrolled before the rotation keeps a credential whose kid is
        # still in the status: the rotation alone revokes nobody.
        credential = base64.b64decode(
            self._enrolled(self.alice_client, _seed(30))["credential"]
        )
        self.assertEqual(credential[5:13].hex(), new_kid)
        self.assertEqual(self._revoked_keys(self.alice_client), [])


class DisabledAccountTestCase(LanRouteCase):
    def test_a_disabled_account_cannot_enrol_and_its_devices_are_listed(self):
        device = self._enrolled(self.bob_client, _seed(40))
        keys = {
            row.pubkey for row in LanDeviceKey.select().where(
                LanDeviceKey.device == device["device_id"]
            )
        }
        self.assertEqual(self._revoked_keys(self.alice_client), [])
        User.update(disabled=True).where(User.id == self.bob.id).execute()
        # The account's own session is gone, so an enrolment is a 401 (the very
        # first thing `login_required` does), and its devices are refused by
        # every peer through the next list another device syncs.
        self.assertEqual(
            self._enroll(
                self.bob_client, _seed(41), subject_hex="00" * 16
            ).status_code,
            401,
        )
        revoked = self._revoked_keys(self.alice_client)
        self.assertEqual({key.hex() for key in revoked}, keys)
        blob = self._revocations(self.alice_client)
        self.assertGreaterEqual(int.from_bytes(blob[13:21], "big"), 1)
        # The device cannot be renewed while the account is disabled either.
        self.assertEqual(
            self.alice_client.delete(f"/api/lan/devices/{device['device_id']}")
            .status_code,
            404,
        )


class NoSecretInTheLogTestCase(LanRouteCase):
    def test_no_key_credential_roster_or_seed_is_ever_logged(self):
        records = []

        class Capture(logging.Handler):
            def emit(self, record):
                records.append(record.getMessage())

        handler = Capture()
        logger = logging.getLogger("neezer")
        logger.addHandler(handler)
        previous = logger.level
        logger.setLevel(logging.DEBUG)
        try:
            answer = self._enrolled(self.alice_client, _seed(50))
            self.alice_client.get("/api/lan/roster")
            self.alice_client.post("/api/lan/rotate")
        finally:
            logger.removeHandler(handler)
            logger.setLevel(previous)
        text = "\n".join(records)
        self.assertIn("LAN", text)  # it does log the action, not the material
        key = lan.lan_key()
        for secret in (
            key.seed.hex(),
            base64.b64encode(key.seed).decode(),
            Meta.get(Meta.key == lan.META_SEED).value,
            answer["credential"],
            answer["roster"]["body"],
            answer["roster"]["sig"],
            answer["revocations"],
        ):
            self.assertNotIn(secret, text)
        self.assertNotIn(str(self.alice.id), text)


if __name__ == "__main__":
    unittest.main()
