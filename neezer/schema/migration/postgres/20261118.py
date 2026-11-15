# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Invitations, folded account names and the account state columns.

Idempotent: every column and the table are created only when missing, the name
backfill only fills NULL keys, and the indexes are created last. Replayed by
``_repair_skipped_migrations`` whenever a column or the table is missing.
"""

import unicodedata

import psycopg2

COLUMNS = (
    ("name_key", "VARCHAR(64)"),
    ("disabled", "BOOLEAN NOT NULL DEFAULT false"),
    ("last_login", "TIMESTAMP"),
    ("onboarding_step", "VARCHAR(16) NOT NULL DEFAULT 'done'"),
)

TABLE = """
CREATE TABLE IF NOT EXISTS invite (
    id VARCHAR(24) PRIMARY KEY,
    token_hash CHAR(64) NOT NULL,
    created_by UUID REFERENCES "user" ON DELETE SET NULL,
    created TIMESTAMP NOT NULL,
    expires TIMESTAMP NOT NULL,
    used_by UUID REFERENCES "user" ON DELETE SET NULL,
    used_at TIMESTAMP,
    revoked_at TIMESTAMP,
    note VARCHAR(64)
)
"""

INDEXES = (
    'CREATE UNIQUE INDEX IF NOT EXISTS index_user_name_key ON "user"(name_key)',
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
    cursor.execute('SELECT id, name, name_key FROM "user" ORDER BY id')
    rows = cursor.fetchall()
    used = {key for _, _, key in rows if key}
    for uid, name, key in rows:
        if key is not None:
            continue
        folded = _fold(name)
        if folded and len(folded) <= 64 and folded not in used:
            cursor.execute(
                'UPDATE "user" SET name_key = %s'
                " WHERE id = %s AND name_key IS NULL",
                (folded, uid),
            )
            used.add(folded)
    cursor.execute(
        'SELECT id, name_key FROM "user" WHERE name_key IS NOT NULL ORDER BY id'
    )
    seen = set()
    for uid, key in cursor.fetchall():
        if key in seen:
            cursor.execute('UPDATE "user" SET name_key = NULL WHERE id = %s', (uid,))
        else:
            seen.add(key)


def apply(args):
    with psycopg2.connect(**args) as conn:
        c = conn.cursor()
        for name, ddl in COLUMNS:
            c.execute(f'ALTER TABLE "user" ADD COLUMN IF NOT EXISTS {name} {ddl}')
        c.execute(TABLE)
        c.execute('UPDATE "user" SET disabled = false WHERE disabled IS NULL')
        _backfill_names(c)
        c.execute(
            'UPDATE "user" SET onboarding_step = \'done\''
            " WHERE onboarding_step IS NULL"
            " OR onboarding_step NOT IN ('artists', 'account', 'done')"
        )
        for statement in INDEXES:
            c.execute(statement)
        conn.commit()
