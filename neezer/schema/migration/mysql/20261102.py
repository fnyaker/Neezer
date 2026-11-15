# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Tracks the background explorer found and verified, kept un-archived."""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CHARSET = " DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            c.execute(
                """CREATE TABLE IF NOT EXISTS discovery (
    id INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY,
    deezer_id VARCHAR(20) NOT NULL UNIQUE,
    state VARCHAR(10) NOT NULL DEFAULT 'new',
    target VARCHAR(48) NOT NULL,
    style VARCHAR(48),
    confidence DOUBLE NOT NULL DEFAULT 0,
    bpm DOUBLE,
    found DATETIME NOT NULL,
    payload TEXT
)"""
                + CHARSET
            )
            c.execute(
                "SELECT COUNT(*) FROM information_schema.statistics"
                " WHERE table_schema = DATABASE() AND table_name = 'discovery'"
                " AND index_name = 'index_discovery_state'"
            )
            if not c.fetchone()[0]:
                c.execute("CREATE INDEX index_discovery_state ON discovery(state)")
        conn.commit()
    finally:
        conn.close()
