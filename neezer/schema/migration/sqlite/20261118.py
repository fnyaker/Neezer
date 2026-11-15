# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Invitations, folded account names and the account state columns.

Idempotent: every column and the table are created only when missing, the name
backfill only fills NULL keys, and the unique index is created last. Replayed by
``_repair_skipped_migrations`` whenever a column or the table is missing.
"""

import sqlite3
import unicodedata

COLUMNS = (
    ("name_key", "VARCHAR(64)"),
    ("disabled", "BOOLEAN NOT NULL DEFAULT 0"),
    ("last_login", "DATETIME"),
    ("onboarding_step", "VARCHAR(16) NOT NULL DEFAULT 'done'"),
)

TABLE = """
CREATE TABLE IF NOT EXISTS invite (
    id VARCHAR(24) PRIMARY KEY,
    token_hash CHAR(64) NOT NULL,
    created_by CHAR(36) REFERENCES user ON DELETE SET NULL,
    created DATETIME NOT NULL,
    expires DATETIME NOT NULL,
    used_by CHAR(36) REFERENCES user ON DELETE SET NULL,
    used_at DATETIME,
    revoked_at DATETIME,
    note VARCHAR(64)
)
"""

INDEXES = (
    "CREATE UNIQUE INDEX IF NOT EXISTS index_user_name_key ON user(name_key)",
    "CREATE UNIQUE INDEX IF NOT EXISTS index_invite_token_hash ON invite(token_hash)",
    "CREATE INDEX IF NOT EXISTS index_invite_expires ON invite(expires)",
    "CREATE INDEX IF NOT EXISTS index_invite_created_by ON invite(created_by)",
    "CREATE INDEX IF NOT EXISTS index_invite_used_by ON invite(used_by)",
)


def _fold(name):
    decomposed = unicodedata.normalize("NFKD", name or "")
    stripped = "".join(
        char for char in decomposed if not unicodedata.combining(char)
    )
    return stripped.strip().lower()


def _backfill_names(cursor):
    rows = cursor.execute('SELECT id, name, name_key FROM "user" ORDER BY id').fetchall()
    used = {key for _, _, key in rows if key}
    for uid, name, key in rows:
        if key is not None:
            continue
        folded = _fold(name)
        if folded and len(folded) <= 64 and folded not in used:
            cursor.execute(
                'UPDATE "user" SET name_key = ? WHERE id = ? AND name_key IS NULL',
                (folded, uid),
            )
            used.add(folded)
    seen = set()
    for uid, key in cursor.execute(
        'SELECT id, name_key FROM "user" WHERE name_key IS NOT NULL ORDER BY id'
    ).fetchall():
        if key in seen:
            cursor.execute('UPDATE "user" SET name_key = NULL WHERE id = ?', (uid,))
        else:
            seen.add(key)


def apply(args):
    file = args.pop("database")
    with sqlite3.connect(file, **args) as conn:
        c = conn.cursor()
        c.execute("PRAGMA table_info(user)")
        have = {row[1] for row in c.fetchall()}
        for name, ddl in COLUMNS:
            if name not in have:
                c.execute(f"ALTER TABLE user ADD COLUMN {name} {ddl}")
        c.execute(TABLE)
        c.execute("UPDATE user SET disabled = 0 WHERE disabled IS NULL")
        _backfill_names(c)
        c.execute(
            "UPDATE user SET onboarding_step = 'done'"
            " WHERE onboarding_step IS NULL"
            " OR onboarding_step NOT IN ('artists', 'account', 'done')"
        )
        for statement in INDEXES:
            c.execute(statement)
        conn.commit()
