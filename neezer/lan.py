# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The server's half of the LAN trust chain (docs/agents/lan-nodes.md, LN1).

The native apps talk to each other on the local network without this server
(QUIC, exporter-bound Ed25519 proofs). This module is the AUTHORITY they all
trust, and it is the only place that holds a signing key:

* one **LAN key** per server (Ed25519), created on first use, its 32-byte seed
  stored in ``Meta`` under ``lan_ca_seed`` **encrypted** (AES-GCM under
  ``sha256(get_secret_key("lan_ca_secret"))`` — the ``encrypt_password`` shape
  with its own derived key), so a database dump alone does not sign anything;
  ``kid = SHA-256(public key)[0..8]`` and a rotation keeps the previous key for
  90 days, so credentials it signed stay valid until they expire;
* the **subject**: ``HMAC-SHA256(get_secret_key("lan_subject"),
  b"neezer-lan/1 subject" + str(user_id))[0..16]`` — an opaque 16-byte name for
  one account, stable, unguessable, with no column and never the account name;
* the **credential** (154 bytes, *Trust chain*'s fixed layout) a device presents
  on every LAN connection, and the **revocation list** (``NZLR``) that names the
  device keys a peer must refuse;
* the **roster** (signed canonical JSON) that tells one account's devices who
  they may talk to and exactly what they may take.

Nothing here is Flask: the routes in ``neezer/webui/lan.py`` are a thin layer,
and the same builders serve a test or a CLI. No key, seed, credential or roster
is ever logged: an error names the ACTION and a French cause, never a byte of
key material.

The three test vectors of the note (credential, revocation list, auth proof)
and the enrolment message pinned with LN2 are asserted in
``tests/test_lan_crypto.py``; they are what keeps this encoder and the Rust
decoder compatible without waiting for each other.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import json
import logging
import re
import secrets
import time
import uuid

from datetime import timedelta

from Crypto.Cipher import AES  # nosec B413 - pycryptodome, not the abandoned pyCrypto
from Crypto.Signature import eddsa  # nosec B413 - pycryptodome, not the abandoned pyCrypto
from peewee import IntegrityError

from .db import (
    LanDevice,
    LanDeviceKey,
    LanFriend,
    LanFriendPlaylist,
    Meta,
    Playlist,
    PlaylistMember,
    PlaylistTrack,
    Track,
    User,
    db,
    now,
)
from . import playlist_access as placcess
from .managers.user import fold_name
from .utils import get_secret_key

logger = logging.getLogger(__name__)

# -- the layout (docs/agents/lan-nodes.md, *Trust chain*) -----------------------

CRED_MAGIC = b"NZLC"
REV_MAGIC = b"NZLR"
LAYOUT_VERSION = 1
CRED_BODY_LEN = 90
CRED_LEN = 154
REV_HEADER_LEN = 33  # magic + version + kid + serial + issued + count
FLAG_SERVE = 0b01
FLAG_FETCH = 0b10
DEFAULT_FLAGS = FLAG_SERVE | FLAG_FETCH
MAX_REVOKED = 4096
MAX_FRIENDS = 64
MAX_SHARED_TRACKS = 20_000
MAX_SCOPED_PLAYLISTS = 100
CRED_DAYS_MIN = 7
CRED_DAYS_MAX = 90
CRED_DAYS_DEFAULT = 30
PREV_KEY_DAYS = 90
ENROLL_SKEW = 300
MAX_DEVICES = 20
MAX_NAME = 64

# One domain string per signed object, so a signature can never be replayed as
# another object of the same key.
DOMAIN_CREDENTIAL = b"neezer-lan/1 credential\x00"
DOMAIN_REVOCATIONS = b"neezer-lan/1 revocations\x00"
DOMAIN_ROSTER = b"neezer-lan/1 roster\x00"
DOMAIN_ENROLL = b"neezer-lan/1 enroll\x00"
DOMAIN_SUBJECT = b"neezer-lan/1 subject"
#: The proof each node signs over the TLS exporter of the connection it is on
#: (*Transport and authentication*). The server never verifies one — the
#: devices do, against each other — but the domain is part of the one contract
#: both sides share, so it lives here with the others and the note's third
#: vector is asserted against it.
DOMAIN_AUTH = b"neezer-lan/1 auth\x00"

PLATFORMS = ("desktop", "android", "ios")
DEVICE_ID_RE = re.compile(r"\A[0-9a-f]{24}\Z")
HEX32_RE = re.compile(r"\A[0-9a-f]{64}\Z")
UUID_RE = re.compile(
    r"\A[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\Z"
)
_CTRL_RE = re.compile(r"[\x00-\x1f\x7f]")
_SEAL_PREFIX = "gcm:"

# Meta keys. ``Meta.key`` is 32 characters, which is why the roster counter is a
# column on ``user`` (``user.lan_serial``) and not a per-account Meta row: the
# note leaves the choice to the lot, and 18 + 36 characters did not fit.
META_SEED = "lan_ca_seed"
META_PREV = "lan_ca_prev"
META_REV_SERIAL = "lan_revocation_serial"
META_REV_DIGEST = "lan_revoked_digest"
META_ENABLED = "lan_enabled"
META_CRED_DAYS = "lan_credential_days"


class Refused(Exception):
    """An action the server will not do, with the cause it answers.

    The route turns it into ``{"error": <French>, "code": <machine>}``. The
    same constructor is used for every failure of one kind, so a refusal can
    never be told apart by its shape.
    """

    def __init__(self, code, message, status=400, field=None, retry_after=None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = int(status)
        self.field = field
        self.retry_after = retry_after


def _epoch():
    """Unix seconds. The credentials' dates are UTC epochs; the database's own
    ``DATETIME`` columns stay the server's local naive time, like everywhere."""
    return int(time.time())


def _iso(value):
    return value.isoformat(timespec="seconds") if value is not None else None


# -- Meta, portably -------------------------------------------------------------


def _meta(key, default=None):
    try:
        return Meta[key].value
    except Meta.DoesNotExist:
        return default


def _meta_set(key, value):
    """Create or replace one ``Meta`` row, on all three engines.

    PostgreSQL has no ``REPLACE``, and a bare ``create`` after a losing race
    poisons the caller's transaction there, so the insert sits in its own
    SAVEPOINT and a conflict falls back to the update.
    """
    value = str(value)
    with db.atomic():
        if Meta.update(value=value).where(Meta.key == key).execute():
            return value
        try:
            with db.atomic():
                Meta.create(key=key, value=value)
        except IntegrityError:
            Meta.update(value=value).where(Meta.key == key).execute()
    return value


# -- the LAN key ------------------------------------------------------------------


def _secret():
    """The AES key that protects the LAN seed at rest — its OWN derivation, so
    it is not the password store's key and neither can decrypt the other."""
    return hashlib.sha256(get_secret_key("lan_ca_secret")).digest()


def _seal(plaintext):
    nonce = secrets.token_bytes(12)
    cipher = AES.new(_secret(), AES.MODE_GCM, nonce=nonce)
    ciphertext, tag = cipher.encrypt_and_digest(plaintext)
    return _SEAL_PREFIX + base64.b64encode(nonce + tag + ciphertext).decode("ascii")


def _unseal(blob):
    """The plaintext, or None when the blob is not ours any more.

    A changed server secret makes every stored blob undecryptable: that is not
    an error to retry, it is the documented "create a new key, every device
    re-enrols at its next contact".
    """
    if not isinstance(blob, str) or not blob.startswith(_SEAL_PREFIX):
        return None
    try:
        raw = base64.b64decode(blob[len(_SEAL_PREFIX) :], validate=True)
        if len(raw) < 12 + 16:
            return None
        nonce, tag, ciphertext = raw[:12], raw[12:28], raw[28:]
        return AES.new(_secret(), AES.MODE_GCM, nonce=nonce).decrypt_and_verify(
            ciphertext, tag
        )
    except (ValueError, TypeError, KeyError):
        return None


class LanKey:
    """One Ed25519 LAN key: the seed, its public half and its 8-byte ``kid``."""

    __slots__ = ("seed", "pub", "kid", "until", "_private")

    def __init__(self, seed, until=None):
        if not isinstance(seed, (bytes, bytearray)) or len(seed) != 32:
            raise ValueError("a LAN seed is 32 bytes")
        self.seed = bytes(seed)
        self.until = until
        self._private = eddsa.import_private_key(self.seed)
        self.pub = self._private.public_key().export_key(format="raw")
        self.kid = hashlib.sha256(self.pub).digest()[:8]

    def sign(self, message):
        return eddsa.new(self._private, "rfc8032").sign(message)

    def __repr__(self):  # never the seed
        return f"LanKey(kid={self.kid.hex()})"


# (secret digest, key): the cache is dropped when the server secret changes, so
# a test that swaps it never signs with the previous key.
_cache = {}


def _reset_for_tests():
    _cache.clear()


def _load_key():
    sealed = _meta(META_SEED)
    if sealed:
        seed = _unseal(sealed)
        if seed is not None and len(seed) == 32:
            return LanKey(seed)
    # No key yet, or the blob no longer decrypts (the secret changed): a new key
    # is made, and every device re-enrols silently at its next contact.
    seed = secrets.token_bytes(32)
    _meta_set(META_SEED, _seal(seed))
    return LanKey(seed)


def lan_key():
    """The current signing key, created on first use and kept in ``Meta``."""
    secret = _secret()
    cached = _cache.get("key")
    if cached is not None and cached[0] == secret:
        return cached[1]
    key = _load_key()
    _cache["key"] = (secret, key)
    return key


def previous_key():
    """The key a rotation retired, while credentials it signed can still be
    valid. ``None`` once its 90 days are over (or before any rotation)."""
    sealed = _meta(META_PREV)
    if not sealed:
        return None
    blob = _unseal(sealed)
    if blob is None or len(blob) != 40:
        return None
    seed, until = blob[:32], int.from_bytes(blob[32:], "big", signed=True)
    if until <= _epoch():
        return None
    return LanKey(seed, until=until)


def status_keys():
    """``GET /api/lan/status``'s ``keys``: the current key, then the previous."""
    keys = [{"kid": lan_key().kid.hex(), "pub": lan_key().pub.hex(), "until": None}]
    previous = previous_key()
    if previous is not None:
        keys.append(
            {
                "kid": previous.kid.hex(),
                "pub": previous.pub.hex(),
                "until": previous.until,
            }
        )
    return keys


def rotate():
    """A new LAN key; the previous one signs valid credentials for 90 days."""
    current = lan_key()
    until = _epoch() + PREV_KEY_DAYS * 86400
    _meta_set(
        META_PREV, _seal(current.seed + until.to_bytes(8, "big", signed=True))
    )
    seed = secrets.token_bytes(32)
    _meta_set(META_SEED, _seal(seed))
    key = LanKey(seed)
    _cache["key"] = (_secret(), key)
    logger.info("LAN key rotated, new kid %s", key.kid.hex())
    return key


# -- the feature's two admin settings (rule 6) -------------------------------------


def enabled():
    return _meta(META_ENABLED, "1") != "0"


def set_enabled(flag):
    _meta_set(META_ENABLED, "1" if flag else "0")


def credential_days():
    try:
        days = int(_meta(META_CRED_DAYS, CRED_DAYS_DEFAULT))
    except (TypeError, ValueError):
        return CRED_DAYS_DEFAULT
    return max(CRED_DAYS_MIN, min(CRED_DAYS_MAX, days))


def set_credential_days(days):
    days = max(CRED_DAYS_MIN, min(CRED_DAYS_MAX, int(days)))
    _meta_set(META_CRED_DAYS, days)
    return days


def settings_json():
    return {"enabled": enabled(), "credential_days": credential_days()}


def save_settings(data):
    if not isinstance(data, dict):
        raise Refused("invalid_body", "Requête invalide.", 400)
    if "enabled" in data:
        if not isinstance(data["enabled"], bool):
            raise Refused(
                "invalid_enabled",
                "L'interrupteur doit être vrai ou faux.",
                400,
                field="enabled",
            )
        set_enabled(data["enabled"])
    if "credential_days" in data:
        raw = data["credential_days"]
        if isinstance(raw, bool) or not isinstance(raw, int):
            raise Refused(
                "invalid_credential_days",
                "La durée doit être un nombre de jours.",
                400,
                field="credential_days",
            )
        if not CRED_DAYS_MIN <= raw <= CRED_DAYS_MAX:
            raise Refused(
                "invalid_credential_days",
                "La durée d'une clé LAN doit être comprise entre 7 et 90 jours.",
                400,
                field="credential_days",
            )
        set_credential_days(raw)
    return settings_json()


# -- the subject -------------------------------------------------------------------


def subject_bytes(user):
    """The 16 opaque bytes that name ``user`` on the LAN."""
    return hmac.new(
        get_secret_key("lan_subject"),
        DOMAIN_SUBJECT + str(user.id).encode("utf-8"),
        hashlib.sha256,
    ).digest()[:16]


def subject_hex(user):
    return subject_bytes(user).hex()


def auth_message(role, exporter):
    """What a node signs to prove its device key on ONE connection:
    ``Domain ‖ role ‖ exporter``, the role's own ASCII bytes and the 32-byte TLS
    exporter. A proof is bound to its connection, so a relayed one fails."""
    exporter = bytes(exporter)
    if role not in (b"initiator", b"responder") or len(exporter) != 32:
        raise ValueError("an auth message is a role and a 32-byte exporter")
    return DOMAIN_AUTH + role + exporter


# -- the credential ----------------------------------------------------------------


def encode_credential(
    key,
    *,
    subject,
    device_id,
    device_pub,
    not_before,
    not_after,
    flags=DEFAULT_FLAGS,
):
    """The 154 bytes of *Trust chain*'s credential, signed by ``key``."""
    subject = bytes(subject)
    device_id = bytes(device_id)
    device_pub = bytes(device_pub)
    if len(subject) != 16:
        raise ValueError("a subject is 16 bytes")
    if len(device_id) != 12:
        raise ValueError("a device id is 12 bytes")
    if len(device_pub) != 32:
        raise ValueError("a device key is 32 bytes")
    if flags & ~DEFAULT_FLAGS:
        raise ValueError("reserved credential flag bits must be zero")
    body = (
        CRED_MAGIC
        + bytes([LAYOUT_VERSION])
        + key.kid
        + subject
        + device_id
        + device_pub
        + int(not_before).to_bytes(8, "big", signed=True)
        + int(not_after).to_bytes(8, "big", signed=True)
        + bytes([flags])
    )
    if len(body) != CRED_BODY_LEN:  # pragma: no cover - the layout is constant
        raise ValueError("credential layout drifted")
    return body + key.sign(DOMAIN_CREDENTIAL + body)


# -- revocations --------------------------------------------------------------------


def revoked_pubkeys():
    """Every key of a revoked device — or of a disabled account's device —
    whose credential may still be valid. Sorted and unique, because a peer
    refuses a list that is not strictly increasing."""
    cutoff = now()
    rows = (
        LanDeviceKey.select(LanDeviceKey.pubkey, LanDeviceKey.not_after)
        .join(LanDevice)
        .switch(LanDevice)
        .join(User)
        .where(
            ((LanDevice.revoked.is_null(False)) | (User.disabled == True))  # noqa: E712
            & (LanDeviceKey.not_after > cutoff)
        )
        .tuples()
    )
    seen = {}
    for pubkey, not_after in rows:
        seen[pubkey] = max(seen.get(pubkey, not_after), not_after)
    if len(seen) > MAX_REVOKED:
        # Keep the keys that stay valid the longest: they are the ones a peer
        # must still refuse. The list is capped, never silently incomplete in
        # the dangerous direction.
        kept = sorted(seen, key=lambda k: seen[k], reverse=True)[:MAX_REVOKED]
        logger.warning(
            "the LAN revocation list is capped at %s entries (%s live keys)",
            MAX_REVOKED,
            len(seen),
        )
        seen = {k: seen[k] for k in kept}
    return sorted(bytes.fromhex(pubkey) for pubkey in seen)


def revocation_serial():
    try:
        return max(0, int(_meta(META_REV_SERIAL, "0")))
    except (TypeError, ValueError):
        return 0


def _bump_revocation_serial():
    value = revocation_serial() + 1
    _meta_set(META_REV_SERIAL, value)
    return value


def _digest(keys):
    return hashlib.sha256(b"".join(keys)).hexdigest()


def revocation_state():
    """``(revoked keys, serial)``, with the serial bumped when the list changed.

    The serial is what a peer compares to know whether its list is older, so it
    must move whenever the list does — including when an account is disabled,
    which happens outside this module. The stored digest makes that automatic
    instead of needing a hook in the account manager (a file this lot does not
    own): the next device that syncs learns the new list.
    """
    keys = revoked_pubkeys()
    digest = _digest(keys)
    stored = _meta(META_REV_DIGEST)
    serial = revocation_serial()
    if stored is None:
        _meta_set(META_REV_DIGEST, digest)
    elif stored != digest:
        serial = _bump_revocation_serial()
        _meta_set(META_REV_DIGEST, digest)
    return keys, serial


def encode_revocations(key, serial, issued, pubkeys):
    """The signed ``NZLR`` blob a device fetches and gossips between peers."""
    pubkeys = list(pubkeys)
    if len(pubkeys) > MAX_REVOKED:
        raise ValueError("too many revoked keys")
    body = (
        REV_MAGIC
        + bytes([LAYOUT_VERSION])
        + key.kid
        + int(serial).to_bytes(8, "big")
        + int(issued).to_bytes(8, "big")
        + len(pubkeys).to_bytes(4, "big")
        + b"".join(pubkeys)
    )
    return body + key.sign(DOMAIN_REVOCATIONS + body)


def signed_revocations(keys=None, serial=None):
    key = lan_key()
    if keys is None or serial is None:
        keys, serial = revocation_state()
    blob = encode_revocations(key, serial, _epoch(), keys)
    return base64.b64encode(blob).decode("ascii"), serial


# -- the roster ---------------------------------------------------------------------


def _canonical(payload):
    """The exact bytes a signature covers: sorted keys, no space, UTF-8.

    The device verifies over the bytes it received and only then parses, so this
    is a convention rather than a requirement; it makes two servers sign the
    same roster identically, which is what a test can hold.
    """
    return json.dumps(
        payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")


def _friend_rows(user):
    return list(
        LanFriend.select(LanFriend.friend, LanFriend.share_all)
        .where(LanFriend.owner == user)
        .order_by(LanFriend.created, LanFriend.friend)
        .tuples()
    )


def _shared_playlists(user, pairs):
    """``{(owner_id, friend_id): [(playlist_id, title)]}`` in ONE statement."""
    if not pairs:
        return {}
    owners = {owner for owner, _ in pairs}
    friends = {friend for _, friend in pairs}
    rows = (
        LanFriendPlaylist.select(
            LanFriendPlaylist.owner,
            LanFriendPlaylist.friend,
            Playlist.id,
            Playlist.name,
        )
        .join(Playlist, on=(LanFriendPlaylist.playlist == Playlist.id))
        .where(
            LanFriendPlaylist.owner.in_(owners) & LanFriendPlaylist.friend.in_(friends)
        )
        .order_by(
            LanFriendPlaylist.owner,
            LanFriendPlaylist.friend,
            Playlist.name,
            Playlist.id,
        )
        .tuples()
    )
    out = {}
    for owner, friend, playlist_id, title in rows:
        if (owner, friend) not in pairs:
            continue
        out.setdefault((owner, friend), []).append((playlist_id, title))
    return out


def _playlist_tracks(playlist_ids, owner):
    """``{playlist_id: [track id, …]}`` for the sharer's whole selection, in ONE
    statement — a 4 000-title playlist must not cost a query per title.

    The filter is deliberately STRICTER than ``Track.visible_clause``: a share
    carries the sharer's own music (the shared library plus their own uploads),
    never another person's private upload — not even when the sharer is an admin
    who may see it. A friend must not receive the id of somebody else's file.
    """
    if not playlist_ids:
        return {}
    query = (
        Track.select(PlaylistTrack.playlist, Track.id)
        .join(PlaylistTrack, on=(PlaylistTrack.track == Track.id))
        .where(
            PlaylistTrack.playlist.in_(playlist_ids)
            & (Track.owner.is_null(True) | (Track.owner == owner.id))
        )
        .order_by(PlaylistTrack.playlist, PlaylistTrack.index)
    )
    out = {}
    for playlist_id, track_id in query.tuples():
        out.setdefault(playlist_id, []).append(str(track_id))
    return out


def roster_serial(user):
    serial = (
        User.select(User.lan_serial).where(User.id == user.id).scalar()
    )
    return max(0, int(serial or 0))


def roster_body(user, *, key=None, issued=None, serial=None):
    """The canonical body of one account's roster.

    ``shares_to`` is what I let others take (each playlist with its own track
    ids, the server's snapshot at ``issued``); ``trusted_by`` is who may ask my
    devices. Caps: 64 friends, 20 000 track ids in total.
    """
    key = key or lan_key()
    issued = _epoch() if issued is None else int(issued)
    serial = roster_serial(user) if serial is None else int(serial)

    pairs = _friend_rows(user)
    playlists = _shared_playlists(user, {(user.id, friend_id) for friend_id, _ in pairs})
    all_playlist_ids = [
        playlist_id for listed in playlists.values() for playlist_id, _ in listed
    ]
    tracks = _playlist_tracks(all_playlist_ids, user)

    shares = []
    budget = MAX_SHARED_TRACKS
    for friend_id, share_all in pairs[:MAX_FRIENDS]:
        entry_playlists = []
        if not share_all:
            for playlist_id, title in playlists.get((user.id, friend_id), []):
                if budget <= 0:
                    break
                ids = tracks.get(playlist_id, [])[:budget]
                budget -= len(ids)
                entry_playlists.append(
                    {"id": str(playlist_id), "title": title, "tracks": ids}
                )
        shares.append(
            {
                "subject": subject_hex_id(friend_id),
                "all": bool(share_all),
                "playlists": entry_playlists,
            }
        )

    trusted = [
        {"subject": subject_hex_id(owner_id), "name": name}
        for owner_id, name in list(
            LanFriend.select(LanFriend.owner, User.name)
            .join(User, on=(LanFriend.owner == User.id))
            .where(LanFriend.friend == user)
            .order_by(LanFriend.created, User.name)
            .tuples()
        )[:MAX_FRIENDS]
    ]

    return {
        "v": LAYOUT_VERSION,
        "kid": key.kid.hex(),
        "subject": subject_hex(user),
        "serial": serial,
        "issued": issued,
        "expires": issued + credential_days() * 86400,
        "shares_to": shares,
        "trusted_by": trusted,
    }


def subject_hex_id(user_id):
    """The subject of an account named by its id (the roster's own spelling)."""
    return hmac.new(
        get_secret_key("lan_subject"),
        DOMAIN_SUBJECT + str(user_id).encode("utf-8"),
        hashlib.sha256,
    ).digest()[:16].hex()


def roster_state(user):
    """What decides the ETag, before any JSON is built or signed."""
    keys, revocation_serial_value = revocation_state()
    return {
        "serial": roster_serial(user),
        "revocation_serial": revocation_serial_value,
        "revoked": keys,
    }


def roster_etag(state):
    return f'"{state["serial"]}-{state["revocation_serial"]}"'


def roster_answer(user, state):
    key = lan_key()
    issued = _epoch()
    body = roster_body(user, key=key, issued=issued, serial=state["serial"])
    raw = _canonical(body)
    signed = {
        "body": base64.b64encode(raw).decode("ascii"),
        "sig": base64.b64encode(key.sign(DOMAIN_ROSTER + raw)).decode("ascii"),
    }
    revocations = base64.b64encode(
        encode_revocations(key, state["revocation_serial"], issued, state["revoked"])
    ).decode("ascii")
    return {
        "roster": signed,
        "revocations": revocations,
        "etag": roster_etag(state),
    }


# -- devices --------------------------------------------------------------------------


def _clean_name(raw):
    if not isinstance(raw, str):
        raise Refused(
            "invalid_name", "Le nom doit faire 1 à 64 caractères.", 400, field="name"
        )
    name = " ".join(_CTRL_RE.sub("", raw).split())
    if not 1 <= len(name) <= MAX_NAME:
        raise Refused(
            "invalid_name", "Le nom doit faire 1 à 64 caractères.", 400, field="name"
        )
    return name


def device_json(device, current_id=None):
    return {
        "id": device.id,
        "name": device.name,
        "platform": device.platform,
        "created": _iso(device.created),
        "last_seen": _iso(device.last_seen),
        "revoked": _iso(device.revoked),
        "current": current_id is not None and device.id == current_id,
    }


def devices_json(user, current_id=None):
    rows = (
        LanDevice.select()
        .where(LanDevice.user == user)
        .order_by(LanDevice.created.desc(), LanDevice.id)
    )
    return [device_json(row, current_id) for row in rows]


def _own_device(user, device_id):
    if not isinstance(device_id, str) or not DEVICE_ID_RE.match(device_id):
        raise Refused("unknown_device", "Cet appareil n'existe pas.", 404)
    device = LanDevice.get_or_none(
        (LanDevice.id == device_id) & (LanDevice.user == user)
    )
    if device is None:
        raise Refused("unknown_device", "Cet appareil n'existe pas.", 404)
    return device


def rename_device(user, device_id, name, current_id=None):
    name = _clean_name(name)
    device = _own_device(user, current_id if device_id == "self" else device_id)
    LanDevice.update(name=name).where(LanDevice.id == device.id).execute()
    device.name = name
    return device_json(device, current_id)


def revoke_device(user, device_id, current_id=None):
    """Revoke one of the caller's devices (``self`` allowed): 204, and the next
    revocation list names every key it held while they can still be valid."""
    device = _own_device(user, current_id if device_id == "self" else device_id)
    if device.revoked is None:
        LanDevice.update(revoked=now()).where(LanDevice.id == device.id).execute()
        device.revoked = now()
        _bump_revocation_serial()
        # Keep the lazy ``revocation_state`` from bumping a second time.
        _meta_set(META_REV_DIGEST, _digest(revoked_pubkeys()))
    return device


def live_device_count(user):
    return (
        LanDevice.select()
        .where(
            (LanDevice.user == user)
            & LanDevice.revoked.is_null(True)
            & (LanDevice.not_after > now())
        )
        .count()
    )


# -- enrolment --------------------------------------------------------------------------


def enroll_message(subject_hex_value, pubkey_hex, at):
    """The bytes a device signs to prove it holds the key it enrols with.

    ``at`` is its DECIMAL TEXT, not an i64: the two neighbours are already
    strings, and the note spells binary fields as ``i64``. LN2 pins the same
    message in ``the_enroll_proof_signs_the_note_message``.
    """
    return (
        DOMAIN_ENROLL
        + str(subject_hex_value).encode("ascii")
        + str(pubkey_hex).encode("ascii")
        + str(int(at)).encode("ascii")
    )


def verify_signature(pubkey, message, signature):
    """Whether ``signature`` is ``pubkey``'s over ``message``. Never raises on a
    hostile key or signature: a malformed one is simply not a proof."""
    if not isinstance(pubkey, (bytes, bytearray)) or len(pubkey) != 32:
        return False
    if not isinstance(signature, (bytes, bytearray)) or len(signature) != 64:
        return False
    try:
        verifier = eddsa.new(eddsa.import_public_key(bytes(pubkey)), "rfc8032")
        verifier.verify(bytes(message), bytes(signature))
    except (ValueError, TypeError):
        return False
    return True


def _bad_proof():
    return Refused("bad_proof", "La preuve de l'appareil est invalide.", 403)


def enroll(user, *, pubkey, name, platform, at, proof, device_id=None):
    """Issue (or renew) one device's credential.

    Everything that can be refused is refused BEFORE a row is written, and the
    refusals are the note's: a bad proof, a proof 301 s old, someone else's
    subject, a 21st live device, a revoked device. A renewal rotates the key
    (the old one stays in ``lan_device_key`` with its own expiry, so a later
    revocation can name it).
    """
    if not isinstance(pubkey, str) or not HEX32_RE.match(pubkey):
        raise Refused(
            "bad_pubkey", "La clé de l'appareil est invalide.", 400, field="pubkey"
        )
    name = _clean_name(name)
    if platform not in PLATFORMS:
        raise Refused(
            "bad_platform",
            "La plateforme de l'appareil est inconnue.",
            400,
            field="platform",
        )
    if isinstance(at, bool) or not isinstance(at, int):
        raise Refused("bad_proof", "La preuve de l'appareil est invalide.", 403)
    if abs(_epoch() - at) > ENROLL_SKEW:
        raise Refused(
            "stale_proof",
            "La preuve de l'appareil est trop ancienne (300 s au plus).",
            403,
        )
    if not isinstance(proof, str):
        raise _bad_proof()
    try:
        signature = base64.b64decode(proof, validate=True)
    except (ValueError, TypeError):
        raise _bad_proof() from None

    own_subject = subject_bytes(user)
    if not verify_signature(
        bytes.fromhex(pubkey), enroll_message(own_subject.hex(), pubkey, at), signature
    ):
        # Someone else's subject lands here too: the proof only verifies over
        # THIS account's subject, and the refusal says nothing more.
        raise _bad_proof()

    device = None
    if device_id is not None:
        device = _own_device(user, device_id)
        if device.revoked is not None:
            raise Refused(
                "device_revoked",
                "Cet appareil a été révoqué : réenrôlez-le comme un nouvel appareil.",
                409,
            )

    holder = LanDevice.get_or_none(LanDevice.pubkey == pubkey)
    if holder is not None and (device is None or holder.id != device.id):
        raise Refused(
            "key_in_use", "Cette clé d'appareil est déjà enrôlée.", 409, field="pubkey"
        )
    retired = (
        LanDeviceKey.select(LanDevice.revoked, User.disabled)
        .join(LanDevice)
        .switch(LanDevice)
        .join(User)
        .where(LanDeviceKey.pubkey == pubkey)
        .tuples()
        .first()
    )
    if retired is not None and (retired[0] is not None or retired[1]):
        raise Refused("key_revoked", "Cette clé d'appareil a été révoquée.", 409)

    if device is None and live_device_count(user) >= MAX_DEVICES:
        raise Refused(
            "too_many_devices",
            "Trop d'appareils enrôlés (20 au plus) : révoquez-en un dans Réglages.",
            409,
        )

    at_time = now()
    days = credential_days()
    not_after = at_time + timedelta(days=days)
    device_pub_bytes = bytes.fromhex(pubkey)

    with db.atomic():
        if device is None:
            device = LanDevice.create(
                id=secrets.token_hex(12),
                user=user,
                pubkey=pubkey,
                name=name,
                platform=platform,
                created=at_time,
                last_seen=at_time,
                not_after=not_after,
            )
        else:
            previous_pubkey, previous_end = device.pubkey, device.not_after
            LanDevice.update(
                pubkey=pubkey,
                name=name,
                platform=platform,
                last_seen=at_time,
                not_after=not_after,
            ).where(LanDevice.id == device.id).execute()
            device.pubkey, device.name, device.platform = pubkey, name, platform
            device.last_seen, device.not_after = at_time, not_after
            if previous_pubkey != pubkey:
                # The retired key keeps ITS expiry — that is until when a
                # credential it signed may still be presented.
                _register_key(previous_pubkey, device.id, previous_end)
        _register_key(pubkey, device.id, not_after)

    key = lan_key()
    credential = encode_credential(
        key,
        subject=own_subject,
        device_id=bytes.fromhex(device.id),
        device_pub=device_pub_bytes,
        not_before=_epoch(),
        not_after=int(not_after.timestamp()),
        flags=DEFAULT_FLAGS,
    )
    state = roster_state(user)
    answer = roster_answer(user, state)
    return {
        "device_id": device.id,
        "credential": base64.b64encode(credential).decode("ascii"),
        "roster": answer["roster"],
        "revocations": answer["revocations"],
    }


def _register_key(pubkey, device_id, not_after):
    """Keep every key a device ever had, with its own expiry. Insert-if-missing
    so an older row is never extended."""
    if LanDeviceKey.get_or_none(LanDeviceKey.pubkey == pubkey) is not None:
        return
    try:
        with db.atomic():
            LanDeviceKey.create(pubkey=pubkey, device=device_id, not_after=not_after)
    except IntegrityError:  # a concurrent enrolment won; the row exists
        pass


# -- friends ------------------------------------------------------------------------------


def _friend_json(user, friend, share_all):
    """One friend as the screen names them. ``user_id`` is the ACCOUNT's id (a
    UUID); ``subject`` is the LAN's own opaque name for them — the trust chain
    never uses the account id."""
    playlists = [
        {"id": str(playlist_id), "title": title}
        for playlist_id, title in _shared_playlists(
            user, {(user.id, friend.id)}
        ).get((user.id, friend.id), [])
    ]
    return {
        "user_id": str(friend.id),
        "subject": subject_hex(friend),
        "name": friend.name,
        "all": bool(share_all),
        "playlists": [] if share_all else playlists,
    }


def friends_json(user):
    rows = (
        LanFriend.select(LanFriend.friend, LanFriend.share_all)
        .where(LanFriend.owner == user)
        .order_by(LanFriend.created, LanFriend.friend)
        .tuples()
    )
    friend_ids = [friend_id for friend_id, _ in rows]
    accounts = {
        account.id: account
        for account in User.select(User.id, User.name).where(
            User.id.in_(friend_ids)
        )
    } if friend_ids else {}
    shared = _shared_playlists(user, {(user.id, fid) for fid in friend_ids})
    shares_to = []
    for friend_id, share_all in rows:
        account = accounts.get(friend_id)
        if account is None:  # pragma: no cover - the FK cascade removes these
            continue
        shares_to.append(
            {
                "user_id": str(friend_id),
                "subject": subject_hex_id(friend_id),
                "name": account.name,
                "all": bool(share_all),
                "playlists": [
                    {"id": str(playlist_id), "title": title}
                    for playlist_id, title in shared.get((user.id, friend_id), [])
                ],
            }
        )
    trusted_by = [
        {"user_id": str(owner_id), "subject": subject_hex_id(owner_id), "name": name}
        for owner_id, name in LanFriend.select(LanFriend.owner, User.name)
        .join(User, on=(LanFriend.owner == User.id))
        .where(LanFriend.friend == user)
        .order_by(LanFriend.created, User.name)
        .tuples()
    ]
    return {"shares_to": shares_to, "trusted_by": trusted_by}


def bump_serial(*users):
    """Move the roster serial of each given account: 1 UPDATE per account, and
    the value is re-read by whoever signs next, never cached here."""
    ids = {u.id for u in users if u is not None}
    for uid in ids:
        User.update(lan_serial=User.lan_serial + 1).where(User.id == uid).execute()


def find_account(name):
    """The account whose FOLDED name matches, or None. Folding is
    ``User.name_key``'s (NFKD, diacritics dropped, lowercase), so "Léa" typed on
    a phone finds the account created as "lea"."""
    if not isinstance(name, str):
        return None
    key = fold_name(name)
    if not key:
        return None
    return User.get_or_none(User.name_key == key)


def add_friend(user, name):
    name = _clean_name(name)
    target = find_account(name)
    if target is None or target.id == user.id or target.disabled:
        # One answer for "unknown", "yourself" and "disabled": the route is not
        # an account-existence oracle.
        raise Refused("unknown_account", "Aucun compte de ce nom.", 404)
    if LanFriend.get_or_none(
        (LanFriend.owner == user) & (LanFriend.friend == target)
    ) is None:
        if (
            LanFriend.select().where(LanFriend.owner == user).count()
            >= MAX_FRIENDS
        ):
            raise Refused(
                "too_many_friends",
                "Trop d'amis de confiance (64 au plus).",
                409,
            )
        with db.atomic():
            LanFriend.create(owner=user, friend=target, share_all=False, created=now())
            bump_serial(user, target)
        logger.info("LAN friend added")
    return _friend_json(user, target, False)


def _readable_playlists(user, playlist_ids):
    """The caller's readable playlists among ``playlist_ids``, in two statements.

    A playlist the owner cannot read is indistinguishable from an unknown id, so
    the route answers 404 for both — and none of them ever enters a roster.
    """
    if not playlist_ids:
        return []
    if len(playlist_ids) > MAX_SCOPED_PLAYLISTS:
        raise Refused(
            "invalid_playlists",
            "Trop de listes de lecture partagées (100 au plus).",
            400,
            field="playlists",
        )
    ids = []
    for raw in playlist_ids:
        if not isinstance(raw, str) or not UUID_RE.match(raw):
            raise Refused(
                "playlist_not_found",
                "Liste de lecture introuvable.",
                404,
                field="playlists",
            )
        ids.append(uuid.UUID(raw))
    rows = list(Playlist.select().where(Playlist.id.in_(ids)))
    members = {
        member.playlist_id: member.role
        for member in PlaylistMember.select(
            PlaylistMember.playlist, PlaylistMember.role
        ).where(
            (PlaylistMember.user == user) & (PlaylistMember.playlist.in_(ids))
        )
    }
    by_id = {playlist.id: playlist for playlist in rows}
    out = []
    for playlist_id in ids:
        playlist = by_id.get(playlist_id)
        if playlist is None or placcess.role_for(
            playlist, user, member=members.get(playlist_id)
        ) is None:
            raise Refused(
                "playlist_not_found",
                "Liste de lecture introuvable.",
                404,
                field="playlists",
            )
        if playlist_id not in {p.id for p in out}:
            out.append(playlist)
    return out


def set_scope(user, friend_id, *, all_=None, playlists=None):
    """What one friend may fetch: everything, or the listed playlists."""
    if not isinstance(friend_id, str) or not UUID_RE.match(friend_id):
        raise Refused("unknown_friend", "Cet ami n'est pas dans votre liste.", 404)
    row = LanFriend.get_or_none(
        (LanFriend.owner == user) & (LanFriend.friend == friend_id)
    )
    target = User.get_or_none(User.id == friend_id)
    if row is None or target is None:
        raise Refused("unknown_friend", "Cet ami n'est pas dans votre liste.", 404)

    wanted = None
    if playlists is not None:
        if not isinstance(playlists, list):
            raise Refused(
                "invalid_playlists",
                "La liste de lectures partagées est invalide.",
                400,
                field="playlists",
            )
        wanted = _readable_playlists(user, playlists)
        total = (
            PlaylistTrack.select()
            .where(PlaylistTrack.playlist.in_([p.id for p in wanted]))
            .count()
            if wanted
            else 0
        )
        if total > MAX_SHARED_TRACKS:
            raise Refused(
                "too_many_tracks",
                "Trop de titres partagés (20 000 au plus).",
                400,
                field="playlists",
            )

    new_all = row.share_all if all_ is None else bool(all_)
    with db.atomic():
        if new_all != row.share_all:
            LanFriend.update(share_all=new_all).where(
                (LanFriend.owner == user) & (LanFriend.friend == target)
            ).execute()
            row.share_all = new_all
        if wanted is not None or new_all:
            LanFriendPlaylist.delete().where(
                (LanFriendPlaylist.owner == user)
                & (LanFriendPlaylist.friend == target)
            ).execute()
        if wanted and not new_all:
            for playlist in wanted:
                LanFriendPlaylist.create(
                    owner=user, friend=target, playlist=playlist.id
                )
        bump_serial(user, target)
    return _friend_json(user, target, row.share_all)


def remove_friend(user, friend_id):
    if not isinstance(friend_id, str) or not UUID_RE.match(friend_id):
        raise Refused("unknown_friend", "Cet ami n'est pas dans votre liste.", 404)
    row = LanFriend.get_or_none(
        (LanFriend.owner == user) & (LanFriend.friend == friend_id)
    )
    if row is None:
        raise Refused("unknown_friend", "Cet ami n'est pas dans votre liste.", 404)
    target = User.get_or_none(User.id == friend_id)
    with db.atomic():
        LanFriendPlaylist.delete().where(
            (LanFriendPlaylist.owner == user)
            & (LanFriendPlaylist.friend == friend_id)
        ).execute()
        LanFriend.delete().where(
            (LanFriend.owner == user) & (LanFriend.friend == friend_id)
        ).execute()
        bump_serial(user, target)
    logger.info("LAN friend removed")


# -- the status answer ----------------------------------------------------------------


def status_json(user):
    return {
        "enabled": enabled(),
        "subject": subject_hex(user),
        "keys": status_keys(),
        "credential_days": credential_days(),
        "revocation_serial": revocation_serial(),
    }
