-- A public read-only link to ONE track (neezer/webui/trackshare.py): the row
-- names the capability, and the token handed out is id + an HMAC of it under
-- the app's secret key, never stored. One live link per track; `revoked`
-- stamps the owner's cut, which the next public request honours.
-- Idempotent: CREATE ... IF NOT EXISTS; replayed by _repair_skipped_migrations.
CREATE TABLE IF NOT EXISTS track_share (
    id VARCHAR(24) PRIMARY KEY,
    track_id CHAR(36) NOT NULL REFERENCES track ON DELETE CASCADE,
    user_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    created DATETIME NOT NULL,
    expires DATETIME,
    revoked DATETIME,
    last_used DATETIME
);
CREATE INDEX IF NOT EXISTS index_track_share_user_revoked ON track_share(user_id, revoked);
CREATE INDEX IF NOT EXISTS index_track_share_track_id_fk ON track_share(track_id);
