-- Speech-to-text scheduling (neezer/transcribe). Additive only: the per-show
-- switches, the sidecar path on the transcript, and the worker/lease tables.
-- Idempotent, replayed by the boot repair when a column or a table is missing.
ALTER TABLE podcast_channel ADD COLUMN IF NOT EXISTS auto_transcribe BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE podcast_channel ADD COLUMN IF NOT EXISTS transcribe_language VARCHAR(8);
ALTER TABLE podcast_transcript ADD COLUMN IF NOT EXISTS source VARCHAR(8) NOT NULL DEFAULT 'server';
ALTER TABLE podcast_transcript ADD COLUMN IF NOT EXISTS worker_name VARCHAR(64);
ALTER TABLE podcast_transcript ADD COLUMN IF NOT EXISTS transcript_path VARCHAR(4096);

CREATE TABLE IF NOT EXISTS transcribe_worker (
    id UUID PRIMARY KEY,
    name VARCHAR(64) NOT NULL,
    kind VARCHAR(8) NOT NULL,
    token TEXT NOT NULL,
    scopes VARCHAR(32) NOT NULL DEFAULT 'transcribe',
    created TIMESTAMP NOT NULL,
    last_seen TIMESTAMP,
    enabled BOOLEAN NOT NULL DEFAULT true,
    revoked TIMESTAMP
);

CREATE TABLE IF NOT EXISTS podcast_transcript_lease (
    episode_id UUID PRIMARY KEY REFERENCES podcast_transcript(episode_id) ON DELETE CASCADE,
    worker_id UUID NOT NULL REFERENCES transcribe_worker(id) ON DELETE CASCADE,
    lease_token_hash CHAR(64) NOT NULL,
    acquired_at TIMESTAMP NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    heartbeat_at TIMESTAMP NOT NULL,
    chunk_start_ms INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error VARCHAR(200)
);
CREATE INDEX IF NOT EXISTS index_transcribe_lease_expires
    ON podcast_transcript_lease(expires_at);
CREATE INDEX IF NOT EXISTS index_transcribe_lease_worker
    ON podcast_transcript_lease(worker_id);
