-- A public read-only link to ONE track (neezer/webui/trackshare.py): the row
-- names the capability, and the token handed out is id + an HMAC of it under
-- the app's secret key, never stored. One live link per track; `revoked`
-- stamps the owner's cut, which the next public request honours.
--
-- The foreign keys are written as table-level CONSTRAINTs: MySQL silently
-- drops the inline `REFERENCES` form inside a column list (see 20261122).
-- Idempotent: CREATE ... IF NOT EXISTS; replayed by _repair_skipped_migrations.
CREATE TABLE IF NOT EXISTS track_share (
    id VARCHAR(24) PRIMARY KEY,
    track_id CHAR(32) NOT NULL,
    user_id CHAR(32) NOT NULL,
    created DATETIME NOT NULL,
    expires DATETIME,
    revoked DATETIME,
    last_used DATETIME,
    CONSTRAINT track_share_track_fk FOREIGN KEY (track_id) REFERENCES track(id) ON DELETE CASCADE,
    CONSTRAINT track_share_user_fk FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE,
    INDEX index_track_share_user_revoked (user_id, revoked),
    INDEX index_track_share_track_id_fk (track_id)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
