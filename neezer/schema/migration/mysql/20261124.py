# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""LAN nodes: the enrolled devices, the keys they held, and trusted friends.

Idempotent: the roster counter is added only when missing, the four tables use
``IF NOT EXISTS`` and the indexes are added only when the catalog lacks them
(MySQL has no ``CREATE INDEX IF NOT EXISTS``). Replayed by
``_repair_skipped_migrations`` whenever a column or a table is missing.
"""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CHARSET = " DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"

COLUMNS = (("user", "lan_serial", "INTEGER NOT NULL DEFAULT 0"),)

TABLES = (
    """CREATE TABLE IF NOT EXISTS lan_device (
    id CHAR(24) PRIMARY KEY,
    user_id CHAR(32) NOT NULL,
    pubkey CHAR(64) NOT NULL,
    name VARCHAR(64) NOT NULL,
    platform VARCHAR(16) NOT NULL,
    created DATETIME NOT NULL,
    last_seen DATETIME NOT NULL,
    not_after DATETIME NOT NULL,
    revoked DATETIME,
    UNIQUE INDEX index_lan_device_pubkey (pubkey),
    INDEX index_lan_device_user (user_id),
    CONSTRAINT lan_device_user_fk FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
)""",
    """CREATE TABLE IF NOT EXISTS lan_device_key (
    pubkey CHAR(64) PRIMARY KEY,
    device_id CHAR(24) NOT NULL,
    not_after DATETIME NOT NULL,
    INDEX index_lan_device_key_device (device_id),
    CONSTRAINT lan_device_key_device_fk FOREIGN KEY (device_id) REFERENCES lan_device(id) ON DELETE CASCADE
)""",
    """CREATE TABLE IF NOT EXISTS lan_friend (
    owner_id CHAR(32) NOT NULL,
    friend_id CHAR(32) NOT NULL,
    share_all BOOLEAN NOT NULL DEFAULT FALSE,
    created DATETIME NOT NULL,
    PRIMARY KEY (owner_id, friend_id),
    CONSTRAINT lan_friend_owner_fk FOREIGN KEY (owner_id) REFERENCES user(id) ON DELETE CASCADE,
    CONSTRAINT lan_friend_friend_fk FOREIGN KEY (friend_id) REFERENCES user(id) ON DELETE CASCADE
)""",
    """CREATE TABLE IF NOT EXISTS lan_friend_playlist (
    owner_id CHAR(32) NOT NULL,
    friend_id CHAR(32) NOT NULL,
    playlist_id CHAR(32) NOT NULL,
    PRIMARY KEY (owner_id, friend_id, playlist_id),
    CONSTRAINT lan_friend_playlist_owner_fk FOREIGN KEY (owner_id) REFERENCES user(id) ON DELETE CASCADE,
    CONSTRAINT lan_friend_playlist_friend_fk FOREIGN KEY (friend_id) REFERENCES user(id) ON DELETE CASCADE,
    CONSTRAINT lan_friend_playlist_playlist_fk FOREIGN KEY (playlist_id) REFERENCES playlist(id) ON DELETE CASCADE
)""",
)

INDEXES = (
    (
        "lan_device",
        "index_lan_device_user",
        "ALTER TABLE lan_device ADD INDEX index_lan_device_user (user_id)",
    ),
    (
        "lan_device_key",
        "index_lan_device_key_device",
        "ALTER TABLE lan_device_key ADD INDEX index_lan_device_key_device (device_id)",
    ),
)


def _has_index(cursor, table, name):
    cursor.execute(
        "SELECT COUNT(*) FROM information_schema.statistics"
        " WHERE table_schema = DATABASE() AND table_name = %s AND index_name = %s",
        (table, name),
    )
    return bool(cursor.fetchone()[0])


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            for table, column, ddl in COLUMNS:
                c.execute(
                    "SELECT COUNT(*) FROM information_schema.columns"
                    " WHERE table_schema = DATABASE()"
                    " AND table_name = %s AND column_name = %s",
                    (table, column),
                )
                if not c.fetchone()[0]:
                    c.execute(f"ALTER TABLE {table} ADD COLUMN {column} {ddl}")
            for statement in TABLES:
                c.execute(statement + CHARSET)
            for table, name, statement in INDEXES:
                if not _has_index(c, table, name):
                    c.execute(statement)
        conn.commit()
    finally:
        conn.close()
