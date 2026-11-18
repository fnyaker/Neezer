# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""LAN nodes: the enrolled devices, the keys they held, and trusted friends.

Idempotent: the roster counter is added only when missing, the four tables use
``IF NOT EXISTS`` and the indexes follow. Replayed by
``_repair_skipped_migrations`` whenever a column or a table is missing, which is
also how a server whose database was moved past this migration gets it late.

``user.lan_serial`` is a COLUMN rather than ``Meta lan_roster_serial:<uuid>``:
``Meta.key`` is 32 characters and that key is 54 (docs/agents/lan-nodes.md
leaves the choice to LN1).
"""

import sqlite3

COLUMNS = (("user", "lan_serial", "INTEGER NOT NULL DEFAULT 0"),)

TABLES = (
    """
CREATE TABLE IF NOT EXISTS lan_device (
    id CHAR(24) PRIMARY KEY,
    user_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    pubkey CHAR(64) NOT NULL UNIQUE,
    name VARCHAR(64) NOT NULL,
    platform VARCHAR(16) NOT NULL,
    created DATETIME NOT NULL,
    last_seen DATETIME NOT NULL,
    not_after DATETIME NOT NULL,
    revoked DATETIME
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_device_key (
    pubkey CHAR(64) PRIMARY KEY,
    device_id CHAR(24) NOT NULL REFERENCES lan_device ON DELETE CASCADE,
    not_after DATETIME NOT NULL
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_friend (
    owner_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    friend_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    share_all BOOLEAN NOT NULL DEFAULT FALSE,
    created DATETIME NOT NULL,
    PRIMARY KEY (owner_id, friend_id)
)
""",
    """
CREATE TABLE IF NOT EXISTS lan_friend_playlist (
    owner_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    friend_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    playlist_id CHAR(36) NOT NULL REFERENCES playlist ON DELETE CASCADE,
    PRIMARY KEY (owner_id, friend_id, playlist_id)
)
""",
)

INDEXES = (
    "CREATE INDEX IF NOT EXISTS index_lan_device_user ON lan_device(user_id)",
    "CREATE INDEX IF NOT EXISTS index_lan_device_key_device"
    " ON lan_device_key(device_id)",
)


def _columns(cursor, table):
    return {row[1] for row in cursor.execute(f"PRAGMA table_info({table})")}


def apply(args):
    file = args.pop("database")
    with sqlite3.connect(file, **args) as conn:
        c = conn.cursor()
        for table, column, ddl in COLUMNS:
            if column not in _columns(c, table):
                c.execute(f"ALTER TABLE {table} ADD COLUMN {column} {ddl}")
        for statement in TABLES:
            c.execute(statement)
        for statement in INDEXES:
            c.execute(statement)
        conn.commit()
