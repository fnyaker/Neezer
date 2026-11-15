# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The user's genre vocabulary, the tracks labelled with it and the trained head.

The index is added only when `information_schema` does not list it: MySQL has
no `CREATE INDEX IF NOT EXISTS`, and this file is replayed by the boot repair.
"""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CHARSET = " DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"

TABLES = (
    """CREATE TABLE IF NOT EXISTS genre_tag (
    id INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(48) NOT NULL UNIQUE,
    color VARCHAR(16),
    archetype VARCHAR(16),
    created DATETIME NOT NULL
)""",
    """CREATE TABLE IF NOT EXISTS track_tag (
    track_id CHAR(32) NOT NULL REFERENCES track(id),
    tag_id INTEGER NOT NULL REFERENCES genre_tag(id),
    created DATETIME NOT NULL,
    PRIMARY KEY (track_id, tag_id)
)""",
    """CREATE TABLE IF NOT EXISTS genre_model (
    id INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY,
    version INTEGER NOT NULL DEFAULT 1,
    created DATETIME NOT NULL,
    active BOOLEAN NOT NULL DEFAULT true,
    kind VARCHAR(16) NOT NULL DEFAULT 'linear',
    hidden INTEGER NOT NULL DEFAULT 0,
    labels TEXT NOT NULL,
    weights TEXT NOT NULL,
    metrics TEXT,
    dim INTEGER NOT NULL DEFAULT 1280
)""",
)


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            for statement in TABLES:
                c.execute(statement + CHARSET)
            c.execute(
                "SELECT COUNT(*) FROM information_schema.statistics"
                " WHERE table_schema = DATABASE() AND table_name = 'track_tag'"
                " AND index_name = 'index_track_tag_tag_id_fk'"
            )
            if not c.fetchone()[0]:
                c.execute("CREATE INDEX index_track_tag_tag_id_fk ON track_tag(tag_id)")
        conn.commit()
    finally:
        conn.close()
