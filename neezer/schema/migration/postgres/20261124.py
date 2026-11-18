# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""LAN nodes: the enrolled devices, the keys they held, and trusted friends.

Idempotent: the roster counter is added only when missing, the four tables use
``IF NOT EXISTS`` and the indexes follow. Replayed by
``_repair_skipped_migrations`` whenever a column or a table is missing.

``"user"`` is quoted: it is a reserved word on PostgreSQL, and an unquoted
identifier there resolves to ``USER`` (the session user function), not the table.
"""

import psycopg2

COLUMNS = (
    (
        "user",
        "lan_serial",
        "INTEGER NOT NULL DEFAULT 0",
    ),
)

TABLES = (
    """
CREATE TABLE IF NOT EXISTS lan_device (
    id CHAR(24) PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    pubkey CHAR(64) NOT NULL UNIQUE,
    name VARCHAR(64) NOT NULL,
    platform VARCHAR(16) NOT NULL,
    created TIMESTAMP NOT NULL,
    last_seen TIMESTAMP NOT NULL,
    not_after TIMESTAMP NOT NULL,
    revoked TIMESTAMP
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_device_key (
    pubkey CHAR(64) PRIMARY KEY,
    device_id CHAR(24) NOT NULL REFERENCES lan_device ON DELETE CASCADE,
    not_after TIMESTAMP NOT NULL
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_friend (
    owner_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    friend_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    share_all BOOLEAN NOT NULL DEFAULT FALSE,
    created TIMESTAMP NOT NULL,
    PRIMARY KEY (owner_id, friend_id)
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_friend_playlist (
    owner_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    friend_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    playlist_id UUID NOT NULL REFERENCES playlist ON DELETE CASCADE,
    PRIMARY KEY (owner_id, friend_id, playlist_id)
)
""",
)

INDEXES = (
    "CREATE INDEX IF NOT EXISTS index_lan_device_user ON lan_device(user_id)",
    "CREATE INDEX IF NOT EXISTS index_lan_device_key_device"
    " ON lan_device_key(device_id)",
)


def apply(args):
    with psycopg2.connect(**args) as conn:
        c = conn.cursor()
        for table, column, ddl in COLUMNS:
            c.execute(
                f'ALTER TABLE "{table}" ADD COLUMN IF NOT EXISTS {column} {ddl}'
            )
        for statement in TABLES:
            c.execute(statement)
        for statement in INDEXES:
            c.execute(statement)
        conn.commit()
