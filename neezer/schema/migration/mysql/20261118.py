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

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CHARSET = " DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"

COLUMNS = (
    ("name_key", "VARCHAR(64)"),
    ("disabled", "BOOLEAN NOT NULL DEFAULT FALSE"),
    ("last_login", "DATETIME"),
    ("onboarding_step", "VARCHAR(16) NOT NULL DEFAULT 'done'"),
)

TABLE = """CREATE TABLE IF NOT EXISTS invite (
    id VARCHAR(24) PRIMARY KEY,
    token_hash CHAR(64) NOT NULL,
    created_by CHAR(32),
    created DATETIME NOT NULL,
    expires DATETIME NOT NULL,
    used_by CHAR(32),
    used_at DATETIME,
    revoked_at DATETIME,
    note VARCHAR(64),
    CONSTRAINT invite_created_by_fk FOREIGN KEY (created_by) REFERENCES user(id) ON DELETE SET NULL,
    CONSTRAINT invite_used_by_fk FOREIGN KEY (used_by) REFERENCES user(id) ON DELETE SET NULL,
    UNIQUE INDEX index_invite_token_hash (token_hash),
    INDEX index_invite_expires (expires),
    INDEX index_invite_created_by (created_by),
    INDEX index_invite_used_by (used_by)
)"""

INDEXES = (
    (
        "user",
        "index_user_name_key",
        "ALTER TABLE `user` ADD UNIQUE INDEX index_user_name_key (name_key)",
    ),
    (
        "invite",
        "index_invite_token_hash",
        "ALTER TABLE invite ADD UNIQUE INDEX index_invite_token_hash (token_hash)",
    ),
    (
        "invite",
        "index_invite_expires",
        "ALTER TABLE invite ADD INDEX index_invite_expires (expires)",
    ),
    (
        "invite",
        "index_invite_created_by",
        "ALTER TABLE invite ADD INDEX index_invite_created_by (created_by)",
    ),
    (
        "invite",
        "index_invite_used_by",
        "ALTER TABLE invite ADD INDEX index_invite_used_by (used_by)",
    ),
)


def _fold(name):
    decomposed = unicodedata.normalize("NFKD", name or "")
    stripped = "".join(
        char for char in decomposed if not unicodedata.combining(char)
    )
    return stripped.strip().lower()


def _has_index(cursor, table, name):
    cursor.execute(
        "SELECT COUNT(*) FROM information_schema.statistics"
        " WHERE table_schema = DATABASE() AND table_name = %s AND index_name = %s",
        (table, name),
    )
    return bool(cursor.fetchone()[0])


def _backfill_names(cursor):
    cursor.execute("SELECT id, name, name_key FROM `user` ORDER BY id")
    rows = cursor.fetchall()
    used = {key for _, _, key in rows if key}
    for uid, name, key in rows:
        if key is not None:
            continue
        folded = _fold(name)
        if folded and len(folded) <= 64 and folded not in used:
            cursor.execute(
                "UPDATE `user` SET name_key = %s"
                " WHERE id = %s AND name_key IS NULL",
                (folded, uid),
            )
            used.add(folded)
    cursor.execute(
        "SELECT id, name_key FROM `user` WHERE name_key IS NOT NULL ORDER BY id"
    )
    seen = set()
    for uid, key in cursor.fetchall():
        if key in seen:
            cursor.execute(
                "UPDATE `user` SET name_key = NULL WHERE id = %s", (uid,)
            )
        else:
            seen.add(key)


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            for column, ddl in COLUMNS:
                c.execute(
                    "SELECT COUNT(*) FROM information_schema.columns"
                    " WHERE table_schema = DATABASE()"
                    " AND table_name = %s AND column_name = %s",
                    ("user", column),
                )
                if not c.fetchone()[0]:
                    c.execute(f"ALTER TABLE `user` ADD COLUMN {column} {ddl}")
            c.execute(TABLE + CHARSET)
            c.execute("UPDATE `user` SET disabled = 0 WHERE disabled IS NULL")
            _backfill_names(c)
            c.execute(
                "UPDATE `user` SET onboarding_step = 'done'"
                " WHERE onboarding_step IS NULL"
                " OR onboarding_step NOT IN ('artists', 'account', 'done')"
            )
            for table, name, statement in INDEXES:
                if not _has_index(c, table, name):
                    c.execute(statement)
        conn.commit()
    finally:
        conn.close()
