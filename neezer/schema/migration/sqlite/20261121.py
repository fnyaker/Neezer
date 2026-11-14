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

import sqlite3

COLUMNS = (
    ("podcast_channel", "auto_transcribe", "BOOLEAN NOT NULL DEFAULT 0"),
    ("podcast_channel", "transcribe_language", "VARCHAR(8)"),
    ("podcast_transcript", "source", "VARCHAR(8) NOT NULL DEFAULT 'server'"),
    ("podcast_transcript", "worker_name", "VARCHAR(64)"),
    ("podcast_transcript", "transcript_path", "VARCHAR(4096)"),
)

TABLES = (
    """
CREATE TABLE IF NOT EXISTS transcribe_worker (
    id CHAR(36) PRIMARY KEY,
    name VARCHAR(64) NOT NULL,
    kind VARCHAR(8) NOT NULL,
    token TEXT NOT NULL,
    scopes VARCHAR(32) NOT NULL DEFAULT 'transcribe',
    created DATETIME NOT NULL,
    last_seen DATETIME,
    enabled BOOLEAN NOT NULL DEFAULT true,
    revoked DATETIME
)
""",
    """
CREATE TABLE IF NOT EXISTS podcast_transcript_lease (
    episode_id CHAR(36) PRIMARY KEY REFERENCES podcast_transcript(episode_id) ON DELETE CASCADE,
    worker_id CHAR(36) NOT NULL REFERENCES transcribe_worker(id) ON DELETE CASCADE,
    lease_token_hash CHAR(64) NOT NULL,
    acquired_at DATETIME NOT NULL,
    expires_at DATETIME NOT NULL,
    heartbeat_at DATETIME NOT NULL,
    chunk_start_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error VARCHAR(200)
)
""",
)

INDEXES = (
    "CREATE INDEX IF NOT EXISTS index_transcribe_lease_expires"
    " ON podcast_transcript_lease(expires_at)",
    "CREATE INDEX IF NOT EXISTS index_transcribe_lease_worker"
    " ON podcast_transcript_lease(worker_id)",
)


def apply(args):
    file = args.pop("database")
    with sqlite3.connect(file, **args) as conn:
        c = conn.cursor()
        for table, column, ddl in COLUMNS:
            c.execute(f"PRAGMA table_info({table})")
            if column not in {row[1] for row in c.fetchall()}:
                c.execute(f"ALTER TABLE {table} ADD COLUMN {column} {ddl}")
        for statement in TABLES:
            c.execute(statement)
        for statement in INDEXES:
            c.execute(statement)
        conn.commit()
