# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Collaborative playlists (neezer/playlist_access.py): a version pair for
optimistic concurrency, a "last edited" snapshot, and one role row per person.
See the SQLite twin; MySQL has no portable ADD COLUMN IF NOT EXISTS, so
information_schema is read first.

The backfill inserts ONE owner row per existing playlist and nothing else,
through INSERT IGNORE: replaying it (the _repair_skipped_migrations path) can
never duplicate a row nor resurrect a non-owner membership somebody removed. It
is not a DATA_MIGRATIONS entry: it never undoes a person's choice.
"""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

_COLUMNS = (
    ("version", "INT NOT NULL DEFAULT 1"),
    ("members_version", "INT NOT NULL DEFAULT 1"),
    ("last_edited_by", "VARCHAR(64)"),
    ("last_edited_at", "DATETIME"),
)


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            for name, ddl in _COLUMNS:
                c.execute(
                    "SELECT COUNT(*) FROM information_schema.columns "
                    "WHERE table_schema = DATABASE() "
                    "AND table_name = 'playlist' AND column_name = %s",
                    (name,),
                )
                if not c.fetchone()[0]:
                    c.execute(f"ALTER TABLE playlist ADD COLUMN {name} {ddl}")
            c.execute(
                "CREATE TABLE IF NOT EXISTS playlist_member ("
                " playlist_id CHAR(32) NOT NULL,"
                " user_id CHAR(32) NOT NULL,"
                " role VARCHAR(8) NOT NULL,"
                " added DATETIME NOT NULL,"
                " CONSTRAINT playlist_member_playlist_fk FOREIGN KEY (playlist_id)"
                "  REFERENCES playlist(id) ON DELETE CASCADE,"
                " CONSTRAINT playlist_member_user_fk FOREIGN KEY (user_id)"
                "  REFERENCES user(id) ON DELETE CASCADE,"
                " PRIMARY KEY (playlist_id, user_id),"
                " INDEX index_playlist_member_user_id_fk (user_id)"
                ") DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
            )
            c.execute(
                "INSERT IGNORE INTO playlist_member (playlist_id, user_id, role, added)"
                " SELECT id, user_id, 'owner', created FROM playlist"
            )
        conn.commit()
    finally:
        conn.close()
