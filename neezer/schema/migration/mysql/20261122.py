# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""The cascade `playlist_share` always assumed, plus the orphans in its way.

20261113 wrote ``playlist_id CHAR(32) NOT NULL REFERENCES playlist(id) ON
DELETE CASCADE`` inside the column list. MySQL parses that and keeps nothing:
``SHOW CREATE TABLE playlist_share`` had no FOREIGN KEY at all, so deleting a
playlist or an account left its public links behind for ever, and a link could
look alive while the playlist it names was gone.

The orphan sweep runs FIRST: the rows it removes are exactly what would make
``ALTER TABLE … ADD CONSTRAINT`` fail (error 1452). Idempotent — the DELETE
removes what is already gone on a healthy database, and each constraint is
added only when ``information_schema`` does not list it. Replayed by
``_repair_skipped_migrations`` whenever a table or column is missing.
"""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CONSTRAINTS = (
    (
        "playlist_share_playlist_fk",
        "FOREIGN KEY (playlist_id) REFERENCES playlist(id) ON DELETE CASCADE",
    ),
    (
        "playlist_share_user_fk",
        "FOREIGN KEY (user_id) REFERENCES `user`(id) ON DELETE CASCADE",
    ),
)


def _has_constraint(cursor, name):
    cursor.execute(
        "SELECT COUNT(*) FROM information_schema.table_constraints"
        " WHERE table_schema = DATABASE() AND table_name = 'playlist_share'"
        " AND constraint_name = %s",
        (name,),
    )
    return bool(cursor.fetchone()[0])


def apply(args):
    conn = provider.connect(**args)
    try:
        with conn.cursor() as c:
            c.execute(
                "DELETE FROM playlist_share WHERE playlist_id NOT IN"
                " (SELECT id FROM playlist)"
            )
            c.execute(
                "DELETE FROM playlist_share WHERE user_id NOT IN"
                " (SELECT id FROM `user`)"
            )
            for name, ddl in CONSTRAINTS:
                if not _has_constraint(c, name):
                    c.execute(
                        "ALTER TABLE playlist_share ADD CONSTRAINT %s %s" % (name, ddl)
                    )
        conn.commit()
    finally:
        conn.close()
