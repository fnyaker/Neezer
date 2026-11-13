-- A public read-only link to one playlist (neezer/webui/playlist_share.py): the
-- row names the capability, and the token handed out is id + an HMAC of it under
-- the app's secret key, never stored. One live link per playlist; `revoked`
-- stamps the owner's cut, which the next public request honours.
CREATE TABLE IF NOT EXISTS playlist_share (
    id VARCHAR(24) PRIMARY KEY,
    playlist_id CHAR(32) NOT NULL REFERENCES playlist(id) ON DELETE CASCADE,
    user_id CHAR(32) NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    created DATETIME NOT NULL,
    expires DATETIME,
    revoked DATETIME,
    last_used DATETIME,
    INDEX index_playlist_share_playlist_revoked (playlist_id, revoked),
    INDEX index_playlist_share_user_id_fk (user_id)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
