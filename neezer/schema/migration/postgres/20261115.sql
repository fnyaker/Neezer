-- Collaborative playlists (neezer/playlist_access.py): a version pair for
-- optimistic concurrency, a "last edited" snapshot, and one role row per person.
-- Idempotent by construction, so _repair_skipped_migrations may replay it.
-- The backfill inserts ONE owner row per existing playlist and nothing else:
-- ON CONFLICT DO NOTHING makes a replay harmless and never resurrects a
-- non-owner membership somebody removed. Not a DATA_MIGRATIONS entry: it never
-- undoes a person's choice.
ALTER TABLE playlist ADD COLUMN IF NOT EXISTS version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE playlist ADD COLUMN IF NOT EXISTS members_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE playlist ADD COLUMN IF NOT EXISTS last_edited_by VARCHAR(64);
ALTER TABLE playlist ADD COLUMN IF NOT EXISTS last_edited_at TIMESTAMP;

CREATE TABLE IF NOT EXISTS playlist_member (
    playlist_id UUID NOT NULL REFERENCES playlist ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    role VARCHAR(8) NOT NULL,
    added TIMESTAMP NOT NULL,
    PRIMARY KEY (playlist_id, user_id)
);
CREATE INDEX IF NOT EXISTS index_playlist_member_user_id_fk ON playlist_member(user_id);

INSERT INTO playlist_member (playlist_id, user_id, role, added)
SELECT id, user_id, 'owner', created FROM playlist
ON CONFLICT DO NOTHING;
