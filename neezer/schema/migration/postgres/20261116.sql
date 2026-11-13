-- Chapters an episode carries in its own file (neezer/deezer/chapters.py). Two
-- tables, no ALTER on podcast_episode: the upgrade is trivial and idempotent.
-- podcast_chapter_scan is what tells "never scanned" from "scanned, nothing
-- there", so a show open never re-reads a file for nothing.
CREATE TABLE IF NOT EXISTS podcast_chapter (
    id UUID PRIMARY KEY,
    episode_id UUID NOT NULL REFERENCES podcast_episode ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER,
    title VARCHAR(300),
    source VARCHAR(8) NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS index_podcast_chapter_episode_idx
    ON podcast_chapter(episode_id, idx);
CREATE UNIQUE INDEX IF NOT EXISTS index_podcast_chapter_episode_start
    ON podcast_chapter(episode_id, start_ms);

CREATE TABLE IF NOT EXISTS podcast_chapter_scan (
    episode_id UUID PRIMARY KEY REFERENCES podcast_episode ON DELETE CASCADE,
    scanned_at TIMESTAMP NOT NULL,
    source VARCHAR(8) NOT NULL,
    n_chapters INTEGER NOT NULL DEFAULT 0,
    error VARCHAR(200)
);
