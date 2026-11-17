import hashlib
import psycopg2


def process_table(connection, table):
    c = connection.cursor()

    c.execute(
        r"ALTER TABLE {} ADD COLUMN path_hash BYTEA NOT NULL DEFAULT E'\\0000'".format(
            table
        )
    )

    hashes = dict()
    c.execute(f"SELECT path FROM {table}")  # nosec B608 - table is a literal at each call site
    for row in c.fetchall():
        # The digest every row of an already-migrated database carries: it must
        # stay SHA-1 or this migration recomputes nothing (nosemgrep below).
        hashes[row[0]] = hashlib.sha1(  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- a path's index key, not a signature; the stored rows already hold SHA-1
            row[0].encode("utf-8"), usedforsecurity=False
        ).digest()
    c.executemany(
        f"UPDATE {table} SET path_hash=%s WHERE path=%s",  # nosec B608 - table is a literal at each call site
        [(h, p) for p, h in hashes.items()],
    )

    c.execute("CREATE UNIQUE INDEX index_{0}_path ON {0}(path_hash)".format(table))


def apply(args):
    with psycopg2.connect(**args) as conn:
        process_table(conn, "folder")
        process_table(conn, "track")
