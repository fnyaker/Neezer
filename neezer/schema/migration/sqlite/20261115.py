# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Collaborative playlists (neezer/playlist_access.py): a version pair for
optimistic concurrency, a "last edited" snapshot, and one role row per person.

SQLite has no ADD COLUMN IF NOT EXISTS, so the shape is read first. The backfill
inserts ONE owner row per existing playlist and nothing else, through
INSERT OR IGNORE: replaying it (the _repair_skipped_migrations path) can never
duplicate a row nor resurrect a non-owner membership somebody removed. It is not
a DATA_MIGRATIONS entry: it never undoes a person's choice.
"""

import sqlite3

_COLUMNS = (
    ("version", "INTEGER NOT NULL DEFAULT 1"),
    ("members_version", "INTEGER NOT NULL DEFAULT 1"),
    ("last_edited_by", "VARCHAR(64)"),
    ("last_edited_at", "DATETIME"),
)


def apply(args):
    file = args.pop("database")
    with sqlite3.connect(file, **args) as conn:
        c = conn.cursor()
        c.execute("PRAGMA table_info(playlist)")
        have = {row[1] for row in c.fetchall()}
        for name, ddl in _COLUMNS:
            if name not in have:
                c.execute(f"ALTER TABLE playlist ADD COLUMN {name} {ddl}")
        c.execute(
            "CREATE TABLE IF NOT EXISTS playlist_member ("
            " playlist_id CHAR(36) NOT NULL REFERENCES playlist ON DELETE CASCADE,"
            " user_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,"
            " role VARCHAR(8) NOT NULL,"
            " added DATETIME NOT NULL,"
            " PRIMARY KEY (playlist_id, user_id))"
        )
        c.execute(
            "CREATE INDEX IF NOT EXISTS index_playlist_member_user_id_fk"
            " ON playlist_member(user_id)"
        )
        c.execute(
            "INSERT OR IGNORE INTO playlist_member (playlist_id, user_id, role, added)"
            " SELECT id, user_id, 'owner', created FROM playlist"
        )
        conn.commit()
