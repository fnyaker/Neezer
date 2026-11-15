# This file is part of Neezer.
# Neezer is a Python implementation of the Subsonic server API.
#
# Distributed under terms of the GNU AGPLv3 license.

"""Speech-to-text scheduling: the per-show switches, the sidecar path and the
worker/lease tables (``neezer/transcribe``).

Idempotent: every column is added only when missing, both tables use
``IF NOT EXISTS`` and the indexes follow. Replayed by
``_repair_skipped_migrations`` whenever a column or a table is missing.
"""

try:
    import MySQLdb as provider
except ImportError:
    import pymysql as provider

CHARSET = " DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"

COLUMNS = (
    ("podcast_channel", "auto_transcribe", "BOOLEAN NOT NULL DEFAULT FALSE"),
    ("podcast_channel", "transcribe_language", "VARCHAR(8)"),
    ("podcast_transcript", "source", "VARCHAR(8) NOT NULL DEFAULT 'server'"),
    ("podcast_transcript", "worker_name", "VARCHAR(64)"),
    ("podcast_transcript", "transcript_path", "VARCHAR(4096)"),
)

TABLES = (
    """CREATE TABLE IF NOT EXISTS transcribe_worker (
    id CHAR(32) PRIMARY KEY,
    name VARCHAR(64) NOT NULL,
    kind VARCHAR(8) NOT NULL,
    token TEXT NOT NULL,
    scopes VARCHAR(32) NOT NULL DEFAULT 'transcribe',
    created DATETIME NOT NULL,
    last_seen DATETIME,
    enabled BOOLEAN NOT NULL DEFAULT true,
    revoked DATETIME
)""",
    """CREATE TABLE IF NOT EXISTS podcast_transcript_lease (
    episode_id CHAR(32) PRIMARY KEY,
    worker_id CHAR(32) NOT NULL,
    lease_token_hash CHAR(64) NOT NULL,
    acquired_at DATETIME NOT NULL,
    expires_at DATETIME NOT NULL,
    heartbeat_at DATETIME NOT NULL,
    chunk_start_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error VARCHAR(200),
    CONSTRAINT podcast_transcript_lease_episode_fk FOREIGN KEY (episode_id) REFERENCES podcast_transcript(episode_id) ON DELETE CASCADE,
    CONSTRAINT podcast_transcript_lease_worker_fk FOREIGN KEY (worker_id) REFERENCES transcribe_worker(id) ON DELETE CASCADE,
    INDEX index_transcribe_lease_expires (expires_at),
    INDEX index_transcribe_lease_worker (worker_id)
)""",
)

INDEXES = (
    (
        "podcast_transcript_lease",
        "index_transcribe_lease_expires",
        "ALTER TABLE podcast_transcript_lease ADD INDEX index_transcribe_lease_expires (expires_at)",
    ),
    (
        "podcast_transcript_lease",
        "index_transcribe_lease_worker",
        "ALTER TABLE podcast_transcript_lease ADD INDEX index_transcribe_lease_worker (worker_id)",
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
