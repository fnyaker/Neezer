# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The server's LAN trust chain: layouts, signatures and the shared vectors.

Every byte of `docs/agents/lan-nodes.md`'s *Trust chain* is asserted here, and
the three vectors of the note are recomputed from their seeds — that is what
keeps this Python encoder and LN2's Rust decoder compatible without either
waiting for the other. The enrolment proof is pinned on the message LN2 fixed
(``at`` as DECIMAL TEXT, never an 8-byte big-endian integer), with a check that
the binary spelling is a DIFFERENT proof.

A test database lives under ``~/.cache`` (the rules forbid test data in ``/tmp``,
a tmpfs on most machines), and every test drops it.
"""

import base64
import json
import os
import shutil
import tempfile
import unittest
import uuid
from datetime import datetime, timedelta
from unittest import mock

from Crypto.Signature import eddsa

from neezer import lan
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
    init_database,
    release_database,
)
from neezer.managers.user import UserManager
from tests.test_deezer import count_statements

# -- the note's reference material -------------------------------------------------

LAN_SEED = bytes(range(0, 32))
DEVICE_SEED = bytes(range(32, 64))
LAN_PUB = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8"
DEVICE_PUB = "29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7"
KID = "56475aa75463474c"
SUBJECT = bytes.fromhex("00112233445566778899aabbccddeeff")
NOT_BEFORE = 1798761600
NOT_AFTER = 1801353600

VECTOR_CREDENTIAL = (
    "4e5a4c430156475aa75463474c00112233445566778899aabbccddeeff010101010101010101010101"
    "29acbae141bccaf0b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd7000000006b36ec80"
    "000000006b5e798003a839e52b155515b9f88b70ab9844585cb0ed07a9a685618cce2243fedfce50"
    "968c222fb9c69068b1ded70f7eff5f1d66acc9419e4f4b524add9fa8bf7efcde09"
)
VECTOR_REVOCATIONS = (
    "4e5a4c520156475aa75463474c0000000000000007000000006b36ec800000000129acbae141bccaf0"
    "b22e1a94d34d0bc7361e526d0bfe12c89794bc9322966dd705c36844f72116363d35c16696464d37f6"
    "88ada1a8c22f6a2f40c6a7c718da4def658ef1e6d64f8bbc5308c208fd72f661f7060efdb9dcd17a87"
    "1035e0ebd005"
)
VECTOR_AUTH_PROOF = (
    "8cac16e29f13ab9811527b65c630d17c0774e759eff96a937ca365f050f91b6306424a132990da39"
    "4b6082123d0c7825db8adb538ee1b31242a2e036aa78b709"
)
# LN2's `the_enroll_proof_signs_the_note_message` signs this exact message with
# the same device seed; the proof below is what both sides must produce.
VECTOR_ENROLL_PROOF = (
    "a5f56aececcb5e4ded2132d770f6a3cb835e0910da516c90436a688841cabbf45ee4ed5bb6a4d9d952"
    "ff6da14a54c8ad66ae138016b9fd2e36a20de2d34ecb08"
)

ROOT = os.path.join(os.path.expanduser("~"), ".cache", "neezer-lan-tests")


def _key(seed):
    return lan.LanKey(seed)


def _forget_secret(name):
    """Drop ``get_secret_key``'s process cache for one name.

    ``neezer.utils`` caches every secret it hands out; a test that changes one
    must clear it, and the module keeps no public API for that.
    """
    vars(__import__("neezer.utils", fromlist=["utils"]))["__key_cache"].pop(
        name, None
    )


def _forget_all_secrets():
    """Every database gets its own secrets, so a test never inherits another
    database's key through the process-wide cache."""
    vars(__import__("neezer.utils", fromlist=["utils"]))["__key_cache"].clear()


class LanDatabaseCase(unittest.TestCase):
    """A throwaway SQLite database under ``~/.cache``, never in ``/tmp``."""

    def setUp(self):
        os.makedirs(ROOT, exist_ok=True)
        self.directory = tempfile.mkdtemp(prefix="crypto-", dir=ROOT)
        self.path = os.path.join(self.directory, "neezer.db")
        self.uri = "sqlite:///" + self.path
        _forget_all_secrets()
        lan._reset_for_tests()
        init_database(self.uri)
        UserManager.add("alice", "Alic3", admin=True)
        UserManager.add("bob", "B0bbb")
        self.alice = User.get(User.name == "alice")
        self.bob = User.get(User.name == "bob")

    def tearDown(self):
        release_database()
        lan._reset_for_tests()
        shutil.rmtree(self.directory, ignore_errors=True)

    def make_tracks(self, count, owner=None, playlist=None):
        """``count`` ordinary shared-library tracks (or ``owner``'s uploads)."""
        if not hasattr(self, "_library"):
            self._root = Folder.create(root=True, name="root", path="/music")
            self._folder = Folder.create(
                root=False, name="Albums", path="/music/albums", parent=self._root
            )
            self._artist = Artist.create(name="Artiste")
            self._album = Album.create(name="Album", artist=self._artist)
            self._library = True
        out = []
        with db.atomic():
            for index in range(count):
                out.append(
                    Track.create(
                        disc=1,
                        number=index + 1,
                        title=f"Titre {index}",
                        duration=200,
                        album=self._album,
                        artist=self._artist,
                        bitrate=900,
                        path=f"/music/albums/{uuid.uuid4()}.flac",
                        last_modification=0,
                        root_folder=self._root,
                        folder=self._folder,
                        owner=owner,
                    )
                )
        if playlist is not None:
            with db.atomic():
                for index, track in enumerate(out):
                    PlaylistTrack.create(playlist=playlist, track=track, index=index)
        return out


class VectorTestCase(unittest.TestCase):
    """The three vectors of the note, recomputed from their seeds."""

    def test_the_three_vectors_are_reproduced_byte_for_byte(self):
        key = _key(LAN_SEED)
        device = _key(DEVICE_SEED)
        self.assertEqual(key.pub.hex(), LAN_PUB)
        self.assertEqual(device.pub.hex(), DEVICE_PUB)
        self.assertEqual(key.kid.hex(), KID)

        credential = lan.encode_credential(
            key,
            subject=SUBJECT,
            device_id=b"\x01" * 12,
            device_pub=bytes.fromhex(DEVICE_PUB),
            not_before=NOT_BEFORE,
            not_after=NOT_AFTER,
            flags=3,
        )
        self.assertEqual(len(credential), 154)
        self.assertEqual(credential.hex(), VECTOR_CREDENTIAL)

        revocations = lan.encode_revocations(
            key, 7, NOT_BEFORE, [bytes.fromhex(DEVICE_PUB)]
        )
        self.assertEqual(len(revocations), 129)
        self.assertEqual(revocations.hex(), VECTOR_REVOCATIONS)

        proof = device.sign(lan.auth_message(b"initiator", b"\xaa" * 32))
        self.assertEqual(proof.hex(), VECTOR_AUTH_PROOF)

    def test_the_vectors_verify_against_the_documented_public_keys(self):
        key = eddsa.import_public_key(bytes.fromhex(LAN_PUB))
        credential = bytes.fromhex(VECTOR_CREDENTIAL)
        eddsa.new(key, "rfc8032").verify(
            lan.DOMAIN_CREDENTIAL + credential[: lan.CRED_BODY_LEN],
            credential[lan.CRED_BODY_LEN :],
        )
        revocations = bytes.fromhex(VECTOR_REVOCATIONS)
        eddsa.new(key, "rfc8032").verify(
            lan.DOMAIN_REVOCATIONS + revocations[: lan.REV_HEADER_LEN + 32],
            revocations[lan.REV_HEADER_LEN + 32 :],
        )
        device = eddsa.import_public_key(bytes.fromhex(DEVICE_PUB))
        eddsa.new(device, "rfc8032").verify(
            lan.auth_message(b"initiator", b"\xaa" * 32),
            bytes.fromhex(VECTOR_AUTH_PROOF),
        )

    def test_the_enrollment_proof_is_the_message_ln2_pins(self):
        """``at`` is DECIMAL TEXT. An 8-byte big-endian ``at`` is a different
        message and therefore a different signature: a disagreement here would
        break enrolment in LN7 without either side noticing."""
        subject_hex = SUBJECT.hex()
        proof = _key(DEVICE_SEED).sign(
            lan.enroll_message(subject_hex, DEVICE_PUB, NOT_BEFORE)
        )
        self.assertEqual(proof.hex(), VECTOR_ENROLL_PROOF)

        message = (
            lan.DOMAIN_ENROLL
            + subject_hex.encode("ascii")
            + DEVICE_PUB.encode("ascii")
            + b"1798761600"
        )
        self.assertEqual(
            lan.enroll_message(subject_hex, DEVICE_PUB, NOT_BEFORE), message
        )
        self.assertTrue(
            lan.verify_signature(bytes.fromhex(DEVICE_PUB), message, proof)
        )

        binary = message[: -len(b"1798761600")] + NOT_BEFORE.to_bytes(
            8, "big", signed=True
        )
        self.assertFalse(
            lan.verify_signature(bytes.fromhex(DEVICE_PUB), binary, proof),
            "an 8-byte big-endian `at` must NOT verify: LN2 pins decimal text",
        )


class CredentialLayoutTestCase(unittest.TestCase):
    def test_the_body_is_the_documented_layout(self):
        key = _key(LAN_SEED)
        credential = lan.encode_credential(
            key,
            subject=SUBJECT,
            device_id=b"\x01" * 12,
            device_pub=bytes.fromhex(DEVICE_PUB),
            not_before=NOT_BEFORE,
            not_after=NOT_AFTER,
        )
        self.assertEqual(credential[0:4], b"NZLC")
        self.assertEqual(credential[4], 1)
        self.assertEqual(credential[5:13], bytes.fromhex(KID))
        self.assertEqual(credential[13:29], SUBJECT)
        self.assertEqual(credential[29:41], b"\x01" * 12)
        self.assertEqual(credential[41:73], bytes.fromhex(DEVICE_PUB))
        self.assertEqual(int.from_bytes(credential[73:81], "big"), NOT_BEFORE)
        self.assertEqual(int.from_bytes(credential[81:89], "big"), NOT_AFTER)
        self.assertEqual(credential[89], lan.FLAG_SERVE | lan.FLAG_FETCH)

    def test_a_malformed_field_is_refused_before_it_is_signed(self):
        key = _key(LAN_SEED)
        base = dict(
            subject=SUBJECT,
            device_id=b"\x01" * 12,
            device_pub=bytes.fromhex(DEVICE_PUB),
            not_before=NOT_BEFORE,
            not_after=NOT_AFTER,
        )
        for override in (
            {"subject": SUBJECT[:15]},
            {"device_id": b"\x01" * 11},
            {"device_pub": bytes.fromhex(DEVICE_PUB)[:31]},
            {"flags": 0b100},
        ):
            with self.assertRaises(ValueError, msg=override):
                lan.encode_credential(key, **{**base, **override})

    def test_a_revocation_list_refuses_more_than_the_cap(self):
        key = _key(LAN_SEED)
        with self.assertRaises(ValueError):
            lan.encode_revocations(
                key, 1, NOT_BEFORE, [b"\x01" * 32] * (lan.MAX_REVOKED + 1)
            )

    def test_the_caps_are_the_note_numbers(self):
        self.assertEqual(lan.MAX_REVOKED, 4096)
        self.assertEqual(lan.MAX_FRIENDS, 64)
        self.assertEqual(lan.MAX_SHARED_TRACKS, 20_000)
        self.assertEqual(lan.MAX_DEVICES, 20)
        self.assertEqual((lan.CRED_DAYS_MIN, lan.CRED_DAYS_MAX), (7, 90))
        self.assertEqual(lan.CRED_DAYS_DEFAULT, 30)
        self.assertEqual(lan.ENROLL_SKEW, 300)
        self.assertEqual(lan.PREV_KEY_DAYS, 90)


class SubjectTestCase(LanDatabaseCase):
    def test_a_subject_is_stable_sixteen_bytes_and_keyed(self):
        first = lan.subject_bytes(self.alice)
        self.assertEqual(len(first), 16)
        self.assertEqual(first, lan.subject_bytes(self.alice))
        self.assertNotEqual(first, lan.subject_bytes(self.bob))
        self.assertNotIn(self.alice.name.encode(), first)
        # A different secret is a different subject: the name is never the id.
        original = os.environ.get("NEEZER_SECRET_LAN_SUBJECT")
        try:
            os.environ["NEEZER_SECRET_LAN_SUBJECT"] = "a different subject secret"
            _forget_secret("lan_subject")
            self.assertNotEqual(first, lan.subject_bytes(self.alice))
        finally:
            if original is None:
                os.environ.pop("NEEZER_SECRET_LAN_SUBJECT", None)
            else:
                os.environ["NEEZER_SECRET_LAN_SUBJECT"] = original
            _forget_secret("lan_subject")
        self.assertEqual(first, lan.subject_bytes(self.alice))


class LanKeyTestCase(LanDatabaseCase):
    def test_the_seed_is_sealed_at_rest_and_the_key_survives_a_restart(self):
        key = lan.lan_key()
        stored = Meta.get(Meta.key == lan.META_SEED).value
        self.assertFalse(stored.startswith(key.seed.hex()))
        self.assertNotIn(key.seed.hex(), json.dumps(list(Meta.select().dicts())))
        self.assertNotIn(base64.b64encode(key.seed).decode(), stored)
        self.assertTrue(stored.startswith("gcm:"))
        # Nothing else in Meta holds the seed in the clear.
        self.assertNotIn(key.seed.hex(), Meta.get(Meta.key == lan.META_SEED).value)

        # A restart: a fresh process cache reading the same database.
        release_database()
        init_database(self.uri)
        lan._reset_for_tests()
        self.assertEqual(lan.lan_key().kid, key.kid)
        self.assertEqual(lan.status_keys()[0]["kid"], key.kid.hex())

    def test_a_changed_server_secret_makes_a_new_key(self):
        before = lan.lan_key()
        original = os.environ.get("NEEZER_SECRET_LAN_CA_SECRET")
        try:
            os.environ["NEEZER_SECRET_LAN_CA_SECRET"] = "a brand new CA secret"
            _forget_secret("lan_ca_secret")
            lan._reset_for_tests()
            after = lan.lan_key()
        finally:
            if original is None:
                os.environ.pop("NEEZER_SECRET_LAN_CA_SECRET", None)
            else:
                os.environ["NEEZER_SECRET_LAN_CA_SECRET"] = original
            _forget_secret("lan_ca_secret")
            lan._reset_for_tests()
        self.assertNotEqual(before.kid, after.kid)
        # The stored blob was undecryptable, so it was replaced; a device's old
        # credential is refused at its next renewal, never silently accepted.
        self.assertNotIn(before.kid.hex(), [k["kid"] for k in lan.status_keys()])
        self.assertEqual(lan.revocation_serial(), 0)

    def test_rotate_keeps_the_previous_key_for_ninety_days(self):
        before = lan.lan_key()
        rotated = lan.rotate()
        self.assertNotEqual(before.kid, rotated.kid)
        keys = lan.status_keys()
        self.assertEqual(
            [k["kid"] for k in keys], [rotated.kid.hex(), before.kid.hex()]
        )
        self.assertIsNone(keys[0]["until"])
        self.assertGreater(keys[1]["until"], lan._epoch() + 89 * 86400)
        self.assertLessEqual(keys[1]["until"], lan._epoch() + 91 * 86400)
        # The retired key still signs valid credentials, and its own kid is the
        # one a peer looks up in the status it fetched.
        self.assertEqual(lan.previous_key().kid, before.kid)


class ProofTestCase(unittest.TestCase):
    def test_a_malformed_proof_is_never_a_proof_and_never_raises(self):
        pubkey = bytes.fromhex(DEVICE_PUB)
        message = lan.enroll_message(SUBJECT.hex(), DEVICE_PUB, NOT_BEFORE)
        proof = _key(DEVICE_SEED).sign(message)
        self.assertTrue(lan.verify_signature(pubkey, message, proof))
        for bad_key, bad_sig in (
            (pubkey[:31], proof),
            (pubkey + b"\x00", proof),
            (b"\x00" * 32, proof),
            (b"\xff" * 32, proof),
            (pubkey, proof[:63]),
            (pubkey, proof + b"\x00"),
            (pubkey, b"\x00" * 64),
        ):
            self.assertFalse(
                lan.verify_signature(bad_key, message, bad_sig),
                (bad_key.hex()[:8], len(bad_sig)),
            )
        # A single flipped byte of the signature is refused too.
        mutated = bytearray(proof)
        mutated[7] ^= 0x01
        self.assertFalse(lan.verify_signature(pubkey, message, bytes(mutated)))
        # And the message is bound: another subject's proof is not ours.
        other = lan.enroll_message("00" * 16, DEVICE_PUB, NOT_BEFORE)
        self.assertFalse(lan.verify_signature(pubkey, other, proof))


class RosterTestCase(LanDatabaseCase):
    def _playlist(self, owner, name, tracks=0, track_owner=None):
        playlist = Playlist.create(user=owner, name=name)
        if tracks:
            self.make_tracks(tracks, owner=track_owner, playlist=playlist)
        return playlist

    def test_the_roster_is_canonical_signed_over_the_exact_bytes(self):
        playlist = self._playlist(self.alice, "Route", tracks=3)
        lan.add_friend(self.alice, "bob")
        lan.set_scope(
            self.alice, str(self.bob.id), all_=False, playlists=[str(playlist.id)]
        )
        key = lan.lan_key()
        state = lan.roster_state(self.alice)
        first = lan.roster_answer(self.alice, state)
        raw = base64.b64decode(first["roster"]["body"])
        signature = base64.b64decode(first["roster"]["sig"])
        eddsa.new(eddsa.import_public_key(key.pub), "rfc8032").verify(
            lan.DOMAIN_ROSTER + raw, signature
        )
        body = json.loads(raw)
        self.assertEqual(body["v"], 1)
        self.assertEqual(body["kid"], key.kid.hex())
        self.assertEqual(body["subject"], lan.subject_hex(self.alice))
        self.assertEqual(body["serial"], state["serial"])
        self.assertEqual(
            body["expires"] - body["issued"], lan.credential_days() * 86400
        )
        self.assertEqual(
            [share["subject"] for share in body["shares_to"]],
            [lan.subject_hex(self.bob)],
        )
        self.assertEqual(body["shares_to"][0]["playlists"][0]["title"], "Route")
        self.assertEqual(len(body["shares_to"][0]["playlists"][0]["tracks"]), 3)
        self.assertEqual(body["trusted_by"], [])
        # Canonical: sorted keys, no spaces. Two identical bodies sign the same.
        self.assertEqual(raw, lan._canonical(body))
        self.assertNotIn(b' ": ', raw)
        self.assertEqual(
            first["etag"],
            '"%s-%s"' % (state["serial"], state["revocation_serial"]),
        )

        # A single flipped byte no longer verifies: the signature covers the
        # EXACT bytes, never a re-serialisation.
        mutated = bytearray(raw)
        mutated[10] ^= 0x01
        with self.assertRaises(ValueError):
            eddsa.new(eddsa.import_public_key(key.pub), "rfc8032").verify(
                lan.DOMAIN_ROSTER + bytes(mutated), signature
            )

    def test_a_track_id_never_leaves_the_sharers_own_music(self):
        """A share is the sharer's own music: the shared library plus their own
        uploads. Another person's private upload never travels through it, even
        when the sharer is an admin who may see that file."""
        playlist = Playlist.create(user=self.alice, name="Route")
        mine = self.make_tracks(2, owner=self.alice, playlist=playlist)
        hidden = self.make_tracks(1, owner=self.bob, playlist=playlist)[0]
        lan.add_friend(self.alice, "bob")
        lan.set_scope(
            self.alice, str(self.bob.id), all_=False, playlists=[str(playlist.id)]
        )
        body = lan.roster_body(self.alice)
        shared = body["shares_to"][0]["playlists"][0]["tracks"]
        self.assertEqual(sorted(shared), sorted(str(t.id) for t in mine))
        self.assertNotIn(str(hidden.id), shared)

    def test_the_caps_truncate_a_roster_instead_of_blowing_it_up(self):
        """The write path refuses an over-cap share, so the encoder's own cap
        is the safety net for rows written before a cap moved: it truncates,
        it never emits a body a peer must refuse."""
        playlist = Playlist.create(user=self.alice, name="Big")
        self.make_tracks(8, playlist=playlist)
        LanFriend.create(
            owner=self.alice, friend=self.bob, share_all=False, created=datetime.now()
        )
        LanFriendPlaylist.create(owner=self.alice, friend=self.bob, playlist=playlist)
        with mock.patch.object(lan, "MAX_SHARED_TRACKS", 5):
            body = lan.roster_body(self.alice)
        self.assertEqual(len(body["shares_to"][0]["playlists"][0]["tracks"]), 5)

        # Friends: 3 rows, a cap of 2.
        for name in ("carol", "dave"):
            UserManager.add(name, "Passw0rd!", admin=False)
        for name in ("carol", "dave"):
            LanFriend.create(
                owner=self.alice,
                friend=User.get(User.name == name),
                share_all=True,
                created=datetime.now(),
            )
        with mock.patch.object(lan, "MAX_FRIENDS", 2):
            body = lan.roster_body(self.alice)
        self.assertEqual(len(body["shares_to"]), 2)

    def test_a_playlist_above_the_shared_track_cap_is_refused_by_number(self):
        playlist = Playlist.create(user=self.alice, name="Big")
        self.make_tracks(6, playlist=playlist)
        lan.add_friend(self.alice, "bob")
        with mock.patch.object(lan, "MAX_SHARED_TRACKS", 5):
            with self.assertRaises(lan.Refused) as caught:
                lan.set_scope(
                    self.alice,
                    str(self.bob.id),
                    all_=False,
                    playlists=[str(playlist.id)],
                )
        self.assertEqual(caught.exception.code, "too_many_tracks")
        self.assertEqual(caught.exception.status, 400)
        self.assertIn("20 000", caught.exception.message)
        self.assertFalse(
            LanFriendPlaylist.select()
            .where(LanFriendPlaylist.owner == self.alice)
            .exists()
        )

    def test_a_friend_who_shares_with_me_appears_in_trusted_by(self):
        lan.add_friend(self.bob, "alice")
        body = lan.roster_body(self.alice)
        self.assertEqual(
            body["trusted_by"],
            [{"subject": lan.subject_hex(self.bob), "name": "bob"}],
        )
        self.assertEqual(body["shares_to"], [])
        # The other direction is bob's own roster, not alice's.
        self.assertEqual(
            [s["subject"] for s in lan.roster_body(self.bob)["shares_to"]],
            [lan.subject_hex(self.alice)],
        )

    def test_a_four_thousand_track_share_stays_under_200_kb_and_two_statements(
        self,
    ):
        """The note's budget: one roster built in ≤ 2 SQL statements per shared
        playlist (so no statement per title) and a body of at most 200 KB.

        Measured on this machine (SQLite, 2026-11-18): a share of one
        4 000-title playlist costs `empty + 2` statements, the body is
        156 0xx bytes of canonical JSON (the base64 envelope around it is 4/3 of
        that, and the wire copy is gzipped by `web.py`), and the same roster
        with 40 titles costs exactly the same number of statements.
        """
        playlist = self._playlist(self.alice, "Route", tracks=4_000)

        with count_statements() as empty:
            lan.roster_answer(self.alice, lan.roster_state(self.alice))

        lan.add_friend(self.alice, "bob")
        lan.set_scope(
            self.alice, str(self.bob.id), all_=False, playlists=[str(playlist.id)]
        )
        with count_statements() as shared:
            answer = lan.roster_answer(self.alice, lan.roster_state(self.alice))

        body = base64.b64decode(answer["roster"]["body"])
        self.assertEqual(len(body), len(lan._canonical(json.loads(body))))
        self.assertLessEqual(
            len(body), 200_000, f"the 4 000-title roster body is {len(body)} B"
        )
        self.assertEqual(
            len(json.loads(body)["shares_to"][0]["playlists"][0]["tracks"]), 4_000
        )
        extra = sum(shared.values()) - sum(empty.values())
        self.assertLessEqual(
            extra,
            2,
            "one shared playlist must cost at most 2 statements, never one per "
            f"title (empty={dict(empty)}, shared={dict(shared)})",
        )

        # No statement depends on the number of titles: cut the playlist down to
        # 40 and the build costs exactly the same.
        PlaylistTrack.delete().where(
            PlaylistTrack.id.in_(
                PlaylistTrack.select(PlaylistTrack.id)
                .where(PlaylistTrack.playlist == playlist.id)
                .order_by(PlaylistTrack.index.desc())
                .limit(3_960)
            )
        ).execute()
        with count_statements() as small:
            lan.roster_answer(self.alice, lan.roster_state(self.alice))
        self.assertEqual(sum(small.values()), sum(shared.values()))

    def test_the_revocation_list_is_sorted_unique_and_named_by_device(self):
        device = LanDevice.create(
            id="0123456789abcdef01234567",
            user=self.alice,
            pubkey="0" * 64,
            name="PC",
            platform="desktop",
            not_after=datetime.now() + timedelta(days=30),
        )
        LanDeviceKey.create(
            pubkey="f" * 64, device=device, not_after=datetime.now()
            + timedelta(days=30)
        )
        LanDeviceKey.create(
            pubkey="1" * 64, device=device, not_after=datetime.now()
            + timedelta(days=1)
        )
        # Not revoked yet: nothing is listed.
        self.assertEqual(lan.revoked_pubkeys(), [])
        LanDevice.update(revoked=datetime.now()).where(
            LanDevice.id == device.id
        ).execute()
        keys = lan.revoked_pubkeys()
        self.assertEqual(keys, sorted(keys))
        self.assertEqual(len(keys), 2)
        # An expired key is not listed: it can no longer be presented.
        LanDeviceKey.update(not_after=datetime.now() - timedelta(days=1)).where(
            LanDeviceKey.pubkey == "1" * 64
        ).execute()
        self.assertEqual(len(lan.revoked_pubkeys()), 1)
        # A disabled account is refused through the same list, without a revoke.
        other = LanDevice.create(
            id="abcdefabcdefabcdefabcdef",
            user=self.bob,
            pubkey="2" * 64,
            name="Téléphone",
            platform="android",
            not_after=datetime.now() + timedelta(days=30),
        )
        LanDeviceKey.create(
            pubkey="2" * 64,
            device=other,
            not_after=datetime.now() + timedelta(days=30),
        )
        User.update(disabled=True).where(User.id == self.bob.id).execute()
        keys = lan.revoked_pubkeys()
        self.assertIn(bytes.fromhex("2" * 64), keys)
        self.assertEqual(keys, sorted(keys))


if __name__ == "__main__":
    unittest.main()
