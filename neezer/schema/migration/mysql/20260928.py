# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Links that let whoever holds one drive one of the owner's players."""

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
                """CREATE TABLE IF NOT EXISTS remote_link (
    id VARCHAR(24) PRIMARY KEY,
    user_id CHAR(32) NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    level VARCHAR(8) NOT NULL,
    device VARCHAR(40) NOT NULL,
    device_name VARCHAR(64),
    label VARCHAR(64),
    created DATETIME NOT NULL,
    expires DATETIME,
    revoked DATETIME,
    last_used DATETIME
)"""
                + CHARSET
            )
            c.execute(
                "SELECT COUNT(*) FROM information_schema.statistics"
                " WHERE table_schema = DATABASE() AND table_name = 'remote_link'"
                " AND index_name = 'index_remote_link_user_id_fk'"
            )
            if not c.fetchone()[0]:
                c.execute(
                    "CREATE INDEX index_remote_link_user_id_fk ON remote_link(user_id)"
                )
        conn.commit()
    finally:
        conn.close()
