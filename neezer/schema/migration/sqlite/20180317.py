import hashlib
import sqlite3


def process_table(connection, table):
    c = connection.cursor()

    c.execute(f"ALTER TABLE {table} ADD COLUMN path_hash BLOB NOT NULL DEFAULT ROWID")

    hashes = dict()
    for row in c.execute(f"SELECT path FROM {table}"):
        hashes[row[0]] = hashlib.sha1(row[0].encode("utf-8"), usedforsecurity=False).digest()
    c.executemany(
        f"UPDATE {table} SET path_hash=? WHERE path=?",
        [(h, p) for p, h in hashes.items()],
    )

    c.execute("CREATE UNIQUE INDEX index_{0}_path ON {0}(path_hash)".format(table))


def apply(args):
    with sqlite3.connect(args["database"]) as conn:
        process_table(conn, "folder")
        process_table(conn, "track")
        conn.commit()
        conn.cursor().execute("VACUUM")
