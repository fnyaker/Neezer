-- Chapters an episode carries in its own file (neezer/deezer/chapters.py). Two
-- tables, no ALTER on podcast_episode: the upgrade is trivial and idempotent.
-- Indexes are inline, as MySQL 5.7 has no CREATE INDEX IF NOT EXISTS.
-- podcast_chapter_scan is what tells "never scanned" from "scanned, nothing
-- there", so a show open never re-reads a file for nothing.
CREATE TABLE IF NOT EXISTS podcast_chapter (
    id CHAR(32) PRIMARY KEY,
    episode_id CHAR(32) NOT NULL,
    idx INTEGER NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER,
    title VARCHAR(300),
    source VARCHAR(8) NOT NULL,
    CONSTRAINT podcast_chapter_episode_fk FOREIGN KEY (episode_id) REFERENCES podcast_episode(id) ON DELETE CASCADE,
    UNIQUE INDEX index_podcast_chapter_episode_idx (episode_id, idx),
    UNIQUE INDEX index_podcast_chapter_episode_start (episode_id, start_ms)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS podcast_chapter_scan (
    episode_id CHAR(32) PRIMARY KEY,
    scanned_at DATETIME NOT NULL,
    source VARCHAR(8) NOT NULL,
    n_chapters INTEGER NOT NULL DEFAULT 0,
    error VARCHAR(200),
    CONSTRAINT podcast_chapter_scan_episode_fk FOREIGN KEY (episode_id) REFERENCES podcast_episode(id) ON DELETE CASCADE
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
