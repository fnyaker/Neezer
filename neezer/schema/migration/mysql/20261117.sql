-- Speech-to-text of an archived episode (neezer/transcribe). Derived text only:
-- deleting a transcript never touches the audio. n_segments is the page count's
-- total, maintained per chunk, so a page never counts the segment rows.
-- Indexes are inline, as MySQL 5.7 has no CREATE INDEX IF NOT EXISTS.
CREATE TABLE IF NOT EXISTS podcast_transcript (
    episode_id CHAR(32) PRIMARY KEY,
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
    cancel_requested BOOLEAN NOT NULL DEFAULT FALSE,
    user_id CHAR(32),
    queued_at DATETIME,
    started_at DATETIME,
    finished_at DATETIME,
    updated DATETIME NOT NULL,
    CONSTRAINT podcast_transcript_episode_fk FOREIGN KEY (episode_id) REFERENCES podcast_episode(id) ON DELETE CASCADE,
    CONSTRAINT podcast_transcript_user_fk FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE SET NULL,
    INDEX index_podcast_transcript_status (status)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS podcast_transcript_segment (
    id INTEGER NOT NULL AUTO_INCREMENT PRIMARY KEY,
    transcript_id CHAR(32) NOT NULL,
    idx INTEGER NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER NOT NULL,
    text TEXT NOT NULL,
    search_text TEXT NOT NULL,
    CONSTRAINT podcast_transcript_segment_transcript_fk FOREIGN KEY (transcript_id) REFERENCES podcast_transcript(episode_id) ON DELETE CASCADE,
    UNIQUE INDEX index_podcast_transcript_segment_idx (transcript_id, idx),
    INDEX index_podcast_transcript_segment_start (transcript_id, start_ms)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
