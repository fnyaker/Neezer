-- A public read-only link to one playlist (neezer/webui/playlist_share.py): the
-- row names the capability, and the token handed out is id + an HMAC of it under
-- the app's secret key, never stored. One live link per playlist; `revoked`
-- stamps the owner's cut, which the next public request honours.
CREATE TABLE IF NOT EXISTS playlist_share (
    id VARCHAR(24) PRIMARY KEY,
    playlist_id UUID NOT NULL REFERENCES playlist ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES "user" ON DELETE CASCADE,
    created TIMESTAMP NOT NULL,
    expires TIMESTAMP,
    revoked TIMESTAMP,
    last_used TIMESTAMP
);
CREATE INDEX IF NOT EXISTS index_playlist_share_playlist_revoked ON playlist_share(playlist_id, revoked);
CREATE INDEX IF NOT EXISTS index_playlist_share_user_id_fk ON playlist_share(user_id);
