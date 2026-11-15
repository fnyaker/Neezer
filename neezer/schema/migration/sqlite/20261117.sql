-- Speech-to-text of an archived episode (neezer/transcribe). Derived text only:
-- deleting a transcript never touches the audio. n_segments is the page count's
-- total, maintained per chunk, so a page never counts the segment rows.
CREATE TABLE IF NOT EXISTS podcast_transcript (
    episode_id CHAR(36) PRIMARY KEY REFERENCES podcast_episode ON DELETE CASCADE,
    status VARCHAR(10) NOT NULL,
    language VARCHAR(8),
    engine VARCHAR(16),
    model VARCHAR(64),
    engine_version VARCHAR(32),
    duration_ms INTEGER NOT NULL DEFAULT 0,
    done_ms INTEGER NOT NULL DEFAULT 0,
    n_segments INTEGER NOT NULL DEFAULT 0,
    error_code VARCHAR(24),
    error_message VARCHAR(300),
    cancel_requested BOOLEAN NOT NULL DEFAULT false,
    user_id CHAR(36) REFERENCES user ON DELETE SET NULL,
    queued_at DATETIME,
    started_at DATETIME,
    finished_at DATETIME,
    updated DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS index_podcast_transcript_status ON podcast_transcript(status);

CREATE TABLE IF NOT EXISTS podcast_transcript_segment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_id CHAR(36) NOT NULL REFERENCES podcast_transcript(episode_id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    text TEXT NOT NULL,
    search_text TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS index_podcast_transcript_segment_idx
    ON podcast_transcript_segment(transcript_id, idx);
CREATE INDEX IF NOT EXISTS index_podcast_transcript_segment_start
    ON podcast_transcript_segment(transcript_id, start_ms);
