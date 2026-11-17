# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Copyright (C) 2013-2022 Alban 'spl0k' Féron
#                    2017 Óscar García Amor
#
# Distributed under terms of the GNU AGPLv3 license.

import base64
import hashlib
import hmac
import logging
import os
import re
import secrets
import string
import threading
import time
import unicodedata
import uuid

from argon2 import PasswordHasher
from argon2.exceptions import Argon2Error
from peewee import IntegrityError

try:  # pycryptodome (the maintained fork of pyCrypto), or pycryptodomex
    from Crypto.Cipher import AES  # nosec B413 - pycryptodome, not the abandoned pyCrypto
    from Crypto.Random import get_random_bytes  # nosec B413 - pycryptodome, not the abandoned pyCrypto
except ImportError:  # pycryptodomex
    from Cryptodome.Cipher import AES  # nosec B413 - pycryptodomex, not the abandoned pyCrypto
    from Cryptodome.Random import get_random_bytes  # nosec B413 - pycryptodomex, not the abandoned pyCrypto

from ..db import Invite, PodcastChannel, ProviderAccount, Track, User, db, now
from ..utils import get_secret_key

logger = logging.getLogger(__name__)

# Password hashing: argon2id for everything new. Legacy users carry a 40-char
# SHA1(salt+password) hash and are transparently upgraded on next login.
_hasher = PasswordHasher()


def _password_key():
    return hashlib.sha256(get_secret_key("password_secret")).digest()


# Marker for the authenticated (AES-GCM) format. Blobs written before this are
# raw base64 of `iv || AES-CFB(password)`: unauthenticated, hence malleable —
# anyone with write access to the database could flip plaintext bits without
# detection. They still decrypt (see below) and are rewritten in the new format
# on the user's next password change or login backfill.
_GCM_PREFIX = "gcm:"


def encrypt_password(plaintext):
    """Reversibly encrypt a password with the server secret (for token auth)."""
    nonce = get_random_bytes(12)
    cipher = AES.new(_password_key(), AES.MODE_GCM, nonce=nonce)
    ct, tag = cipher.encrypt_and_digest(plaintext.encode("utf-8"))
    return _GCM_PREFIX + base64.b64encode(nonce + tag + ct).decode()


def decrypt_password(blob):
    if blob.startswith(_GCM_PREFIX):
        raw = base64.b64decode(blob[len(_GCM_PREFIX) :])
        nonce, tag, ct = raw[:12], raw[12:28], raw[28:]
        cipher = AES.new(_password_key(), AES.MODE_GCM, nonce=nonce)
        # Raises ValueError if the ciphertext was tampered with.
        return cipher.decrypt_and_verify(ct, tag).decode("utf-8")

    # Legacy unauthenticated AES-CFB blob.
    raw = base64.b64decode(blob)
    iv, ct = raw[:16], raw[16:]
    cipher = AES.new(_password_key(), AES.MODE_CFB, iv)
    return cipher.decrypt(ct).decode("utf-8")


# Passwords that are effectively public knowledge — including the ones this
# project's own docs and docker-compose use as placeholders, which is exactly
# how they end up in production untouched. Always refused, no configuration.
BANNED_PASSWORDS = frozenset(
    {
        "changeme",
        "neezer",
        "password",
        "passw0rd",
        "admin",
        "administrator",
        "letmein",
        "qwerty",
        "azerty",
        "123456",
        "1234567",
        "12345678",
        "123456789",
        "1234567890",
        "iloveyou",
        "welcome",
        "abc123",
        "motdepasse",
    }
)

# Minimum password length. Off (0) by default so existing accounts and small
# LAN installs keep working; set NEEZER_MIN_PASSWORD_LENGTH=12 on anything
# reachable from the internet. The pre-rebrand SUPYSONIC_MIN_PASSWORD_LENGTH is
# still read.
_MIN_PASSWORD_LENGTH_ENV = "NEEZER_MIN_PASSWORD_LENGTH"  # nosec B105 - the NAME of an environment variable, not a password
_LEGACY_MIN_PASSWORD_LENGTH_ENV = "SUPYSONIC_MIN_PASSWORD_LENGTH"  # nosec B105 - the NAME of an environment variable, not a password


def _min_password_length() -> int:
    try:
        raw = os.environ.get(_MIN_PASSWORD_LENGTH_ENV) or os.environ.get(
            _LEGACY_MIN_PASSWORD_LENGTH_ENV, "0"
        )
        return max(0, int(raw))
    except ValueError:
        return 0


_RESERVED_NAMES = frozenset(
    {
        "admin",
        "administrator",
        "anonymous",
        "api",
        "app",
        "guest",
        "invite",
        "login",
        "logout",
        "me",
        "neezer",
        "none",
        "null",
        "rest",
        "root",
        "server",
        "settings",
        "static",
        "support",
        "system",
        "undefined",
    }
)

_NAME_RE = re.compile(r"[a-z0-9](?:[a-z0-9_.-]{1,30}[a-z0-9])")


class InvalidUserName(ValueError):
    pass


class UserNameTaken(ValueError):
    pass


class UserActionRefused(ValueError):
    """An administrative action the server refuses on purpose."""

    code = "refused"


class SelfDelete(UserActionRefused):
    code = "self_delete"


class SelfDisable(UserActionRefused):
    code = "self_disable"


class LastAdmin(UserActionRefused):
    code = "last_admin"


class ServerAccountHolder(UserActionRefused):
    code = "server_account_holder"


class WeakPassword(ValueError):
    pass


class PasswordMismatch(ValueError):
    pass


INVITE_MIN_PASSWORD = 12
INVITE_MAX_PASSWORD = 128


def fold_name(name):
    """The lookup key of a user name: NFKD, diacritics dropped, lowercase."""
    if not isinstance(name, str):
        raise TypeError("Invalid user name")
    decomposed = unicodedata.normalize("NFKD", name)
    stripped = "".join(char for char in decomposed if not unicodedata.combining(char))
    return stripped.strip().lower()


def validate_name(name, reserved=False):
    """Return the folded name, or raise InvalidUserName."""
    key = fold_name(name)
    if len(name.strip()) > 64 or not _NAME_RE.fullmatch(key):
        raise InvalidUserName(
            "The name must be 3 to 32 characters: a-z, 0-9, dot, dash, underscore"
        )
    if reserved and key in _RESERVED_NAMES:
        raise InvalidUserName("This name is reserved")
    return key


def check_invite_password(name_key, password):
    """The stronger policy of accounts created by an invitation.

    Length and a blocklist (NIST SP 800-63B, OWASP), no composition rule, and
    no password that carries the account name in either direction.
    """
    if not isinstance(password, str) or not (
        INVITE_MIN_PASSWORD <= len(password) <= INVITE_MAX_PASSWORD
    ):
        raise WeakPassword("The password is too weak")
    if password.strip().lower() in BANNED_PASSWORDS:
        raise WeakPassword("This password is too common, pick another one")
    folded = fold_name(password)
    if name_key in folded or name_key[::-1] in folded:
        raise WeakPassword("The password contains the user name")


class UserManager:
    @staticmethod
    def check_password_policy(password):
        """Raise ValueError if `password` is unacceptable.

        Deliberately minimal by default: the only unconditional rules are "not
        empty" and "not one of the passwords everybody tries first". A length
        floor is opt-in through NEEZER_MIN_PASSWORD_LENGTH.
        """
        if not password:
            raise ValueError("The password can't be empty")
        if password.strip().lower() in BANNED_PASSWORDS:
            raise ValueError("This password is too common, pick another one")
        minimum = _min_password_length()
        if minimum and len(password) < minimum:
            raise ValueError(f"The password must be at least {minimum} characters long")

    @staticmethod
    def get(uid):
        if isinstance(uid, uuid.UUID):
            pass
        elif isinstance(uid, str):
            uid = uuid.UUID(uid)
        else:
            raise TypeError("Invalid user id")

        return User[uid]

    # The only User columns a caller may set at creation time. Everything else
    # (password, salt, password_clear, session_epoch, last_play...) is derived
    # here. Without this whitelist, any form field that reached **kwargs became
    # a column write — `admin=1` on the add-user form was a one-request
    # privilege escalation.
    CREATABLE_FIELDS = frozenset({"mail", "admin", "jukebox"})

    @staticmethod
    def add(name, password, **kwargs):
        unknown = set(kwargs) - UserManager.CREATABLE_FIELDS
        if unknown:
            raise ValueError("Unknown field: " + ", ".join(sorted(unknown)))

        key = validate_name(name)
        if UserManager._name_taken(key):
            raise UserNameTaken(f"User '{name}' exists")

        UserManager.check_password_policy(password)
        crypt, salt = UserManager._hash_fields(password)
        try:
            return User.create(
                name=name.strip(),
                name_key=key,
                password=crypt,
                salt=salt,
                password_clear=encrypt_password(password),
                disabled=False,
                onboarding_step="done",
                **kwargs,
            )
        except IntegrityError as exc:
            raise UserNameTaken(f"User '{name}' exists") from exc

    @staticmethod
    def create_from_invite(name, password, password_confirm=None, at=None):
        """The one account a redeemed invitation creates: never an admin,
        always at the first onboarding step."""
        key = validate_name(name, reserved=True)
        check_invite_password(key, password)
        if password_confirm is not None and (
            not isinstance(password_confirm, str)
            or not hmac.compare_digest(
                password.encode("utf-8"), password_confirm.encode("utf-8")
            )
        ):
            raise PasswordMismatch("The passwords do not match")
        if UserManager._name_taken(key):
            raise UserNameTaken(f"User '{name}' exists")

        crypt, salt = UserManager._hash_fields(password)
        try:
            return User.create(
                name=key,
                name_key=key,
                password=crypt,
                salt=salt,
                password_clear=encrypt_password(password),
                admin=False,
                jukebox=False,
                disabled=False,
                onboarding_step="artists",
                last_login=at or now(),
            )
        except IntegrityError as exc:
            raise UserNameTaken(f"User '{name}' exists") from exc

    @staticmethod
    def rename(user_or_name, new_name):
        if isinstance(user_or_name, User):
            user = user_or_name
        elif isinstance(user_or_name, str):
            user = User.get_or_none(name=user_or_name)
            if user is None:
                user = User.get_or_none(name_key=fold_name(user_or_name))
            if user is None:
                raise User.DoesNotExist(user_or_name)
        else:
            raise TypeError("Requires a User instance or a user name (string)")

        key = validate_name(new_name)
        if user.name_key != key or user.name != new_name.strip():
            if UserManager._name_taken(key, exclude=user.id):
                raise UserNameTaken(f"User '{new_name}' exists")
            user.name = new_name.strip()
            user.name_key = key
            try:
                user.save()
            except IntegrityError as exc:
                raise UserNameTaken(f"User '{new_name}' exists") from exc
        return user

    @staticmethod
    def _name_taken(key, exclude=None):
        query = User.select(User.id, User.name, User.name_key).where(
            (User.name_key == key) | (User.name_key.is_null(True))
        )
        if exclude is not None:
            query = query.where(User.id != exclude)
        return any(
            row.name_key == key or fold_name(row.name or "") == key for row in query
        )

    @staticmethod
    def delete(uid):
        user = UserManager.get(uid)
        user.delete_instance(recursive=True)

    @staticmethod
    def delete_by_name(name):
        user = User.get(name=name)
        user.delete_instance(recursive=True)

    @staticmethod
    def _active_admin_ids(exclude=None, lock=False):
        """The ids of the admins that are not disabled (the ones a server cannot
        lose), optionally locking those rows inside a write transaction."""
        query = User.select(User.id).where(
            (User.admin == True) & (User.disabled == False)
        )
        if exclude is not None:
            query = query.where(User.id != exclude)
        if lock and db.obj.for_update:
            query = query.for_update()
        return [row[0] for row in query.tuples()]

    @staticmethod
    def _locked(uid):
        query = User.select().where(User.id == uid)
        if db.obj.for_update:
            query = query.for_update()
        user = query.first()
        if user is None:
            raise User.DoesNotExist(uid)
        return user

    @staticmethod
    def update_guarded(uid, actor, *, admin=None, disabled=None):
        """Change a role and/or the disabled state, with the last-active-admin
        invariant and the session revocation rules. The invariant is counted
        inside the write and re-checked after it, because SQLite has no row
        locks. Returns the fresh row."""
        if admin is None and disabled is None:
            raise ValueError("Nothing to change")
        for name, value in (("admin", admin), ("disabled", disabled)):
            if value is not None and not isinstance(value, bool):
                raise ValueError(f"'{name}' must be a boolean")
        if actor is None:
            raise ValueError("An acting admin is required")
        uid = uuid.UUID(str(uid))

        with db.atomic():
            target = UserManager._locked(uid)
            if disabled is True and target.id == actor.id:
                raise SelfDisable("Can't disable your own account")
            was_active_admin = target.admin and not target.disabled
            demote = admin is False and target.admin
            disable = disabled is True and not target.disabled
            enable = disabled is False and target.disabled
            if was_active_admin and (demote or disable):
                if not UserManager._active_admin_ids(exclude=target.id, lock=True):
                    raise LastAdmin("Can't remove the last active administrator")

            changed = False
            if admin is not None and admin != target.admin:
                target.admin = admin
                changed = True
            if disabled is not None and disabled != target.disabled:
                target.disabled = disabled
                changed = True
            if not changed:
                return target
            if demote or disable or enable:
                target.session_epoch = (target.session_epoch or 0) + 1
            target.save()
            if was_active_admin and (demote or disable):
                if not UserManager._active_admin_ids(lock=True):
                    raise LastAdmin("Can't remove the last active administrator")
            return target

    @staticmethod
    def delete_guarded(uid, actor):
        """Delete an account, refusing to lock the server out, to publish the
        account's private uploads, or to cut the server's Deezer account.
        Returns the number of uploads reassigned to the actor."""
        if actor is None:
            raise ValueError("An acting admin is required")
        uid = uuid.UUID(str(uid))

        from .. import access

        with db.atomic():
            target = UserManager._locked(uid)
            was_active_admin = target.admin and not target.disabled
            if target.id == actor.id:
                raise SelfDelete("Can't delete your own account")
            if was_active_admin:
                if not UserManager._active_admin_ids(exclude=target.id, lock=True):
                    raise LastAdmin("Can't remove the last active administrator")
            if (
                ProviderAccount.select()
                .where(
                    (ProviderAccount.user == target.id)
                    & (ProviderAccount.mode == access.SERVER)
                )
                .exists()
            ):
                raise ServerAccountHolder(
                    "This account holds the server's Deezer account"
                )

            stamp = now()
            Invite.update(revoked_at=stamp).where(
                (Invite.created_by == target.id)
                & Invite.used_at.is_null(True)
                & Invite.revoked_at.is_null(True)
                & (Invite.expires > stamp)
            ).execute()
            access.forget_subject(f"u:{target.id}")
            accounts = list(
                ProviderAccount.select().where(ProviderAccount.user == target.id)
            )
            for account in accounts:
                access.forget_account(account.id)
            if accounts:
                ProviderAccount.delete().where(
                    ProviderAccount.user == target.id
                ).execute()
            moved = (
                Track.update(owner=actor.id).where(Track.owner == target.id).execute()
            )
            PodcastChannel.update(user=actor.id).where(
                PodcastChannel.user == target.id
            ).execute()
            target.delete_instance(recursive=True)
            if was_active_admin and not UserManager._active_admin_ids(lock=True):
                raise LastAdmin("Can't remove the last active administrator")
            logger.info(
                "admin %s deleted account %s (uploads reassigned: %s)",
                actor.name,
                target.name,
                moved,
            )
        return moved

    _last_login_touch = {}
    _last_login_lock = threading.Lock()
    LAST_LOGIN_EVERY = 3600.0

    @staticmethod
    def touch_last_login(user):
        """Write ``last_login`` at most once an hour per account."""
        stamp = time.monotonic()
        with UserManager._last_login_lock:
            previous = UserManager._last_login_touch.get(user.id)
            if previous is not None and stamp - previous < UserManager.LAST_LOGIN_EVERY:
                return False
            UserManager._last_login_touch[user.id] = stamp
        at = now()
        User.update(last_login=at).where(User.id == user.id).execute()
        user.last_login = at
        return True

    @staticmethod
    def reset_last_login_cache():
        with UserManager._last_login_lock:
            UserManager._last_login_touch.clear()

    @staticmethod
    def _by_name(name):
        if not isinstance(name, str) or not name.strip():
            return None
        exact = User.get_or_none(User.name == name)
        if exact is not None:
            return exact
        try:
            return User.get_or_none(User.name_key == fold_name(name))
        except TypeError:
            return None

    @staticmethod
    def try_auth(name, password):
        user = UserManager._by_name(name)
        if user is None:
            # Verify against a throwaway hash so a missing account costs the
            # same as a wrong password. Returning early made the response time
            # a reliable "does this user exist?" oracle with production argon2
            # parameters (~60 ms vs ~1 ms).
            UserManager._burn_verify_time()
            return None
        if not UserManager._verify_password(user, password):
            return None
        if user.disabled:
            return None
        # Backfill the recoverable password so token auth works for users
        # created before that was added (after one password login).
        if not user.password_clear:
            user.password_clear = encrypt_password(password)
            user.save()
        UserManager.touch_last_login(user)
        return user

    @staticmethod
    def try_auth_token(name, token, salt):
        """Subsonic token auth: token == md5(password + salt)."""
        user = UserManager._by_name(name)
        if user is None or user.disabled or not user.password_clear:
            return None
        try:
            password = decrypt_password(user.password_clear)
        except Exception:
            return None
        expected = hashlib.md5((password + salt).encode("utf-8")).hexdigest()
        if hmac.compare_digest(expected, str(token).lower()):
            UserManager.touch_last_login(user)
            return user
        return None

    @staticmethod
    def change_password(uid, old_pass, new_pass):
        user = UserManager.get(uid)
        if not UserManager._verify_password(user, old_pass):
            raise ValueError("Wrong password")

        UserManager.check_password_policy(new_pass)
        user.password, user.salt = UserManager._hash_fields(new_pass)
        user.password_clear = encrypt_password(new_pass)
        # Revoke every session minted with the old password (see db.User).
        user.session_epoch = (user.session_epoch or 0) + 1
        user.save()

    @staticmethod
    def change_password2(name_or_user, new_pass):
        if isinstance(name_or_user, User):
            user = name_or_user
        elif isinstance(name_or_user, str):
            user = User.get(name=name_or_user)
        else:
            raise TypeError("Requires a User instance or a user name (string)")

        UserManager.check_password_policy(new_pass)
        user.password, user.salt = UserManager._hash_fields(new_pass)
        user.password_clear = encrypt_password(new_pass)
        user.session_epoch = (user.session_epoch or 0) + 1
        user.save()

    # A hash of a value nobody can supply, used to equalise the timing of a
    # login for a nonexistent user. Built lazily so importing this module stays
    # cheap, and reused so the cost matches a real verify.
    __dummy_hash = None

    @staticmethod
    def _burn_verify_time():
        if UserManager.__dummy_hash is None:
            UserManager.__dummy_hash = _hasher.hash(secrets.token_hex(16))
        try:
            _hasher.verify(UserManager.__dummy_hash, "")
        except Argon2Error:
            pass

    @staticmethod
    def _hash_fields(password):
        """argon2id hash + a throwaway salt (the salt column is NOT NULL but is
        unused for argon2, whose salt is embedded in the hash)."""
        salt = "".join(secrets.choice(string.printable.strip()) for _ in range(6))
        return _hasher.hash(password), salt

    @staticmethod
    def _verify_password(user, password):
        """Verify `password` against the stored hash, upgrading legacy hashes.

        Handles both argon2id and legacy SHA1(salt+password). On a successful
        legacy verification the password is transparently rehashed with argon2
        and saved, so old accounts migrate on their next login.
        """
        stored = user.password or ""
        if stored.startswith("$argon2"):
            try:
                _hasher.verify(stored, password)
            except Argon2Error:
                return False
            if _hasher.check_needs_rehash(stored):
                user.password, user.salt = UserManager._hash_fields(password)
                user.save()
            return True

        # Legacy SHA1(salt + password), constant-time compared.
        if not hmac.compare_digest(
            UserManager.__legacy_sha1(password, user.salt), stored
        ):
            return False
        user.password, user.salt = UserManager._hash_fields(password)
        user.save()
        return True

    @staticmethod
    def __legacy_sha1(password, salt):
        return hashlib.sha1(
            salt.encode("utf-8") + password.encode("utf-8")
        ).hexdigest()
