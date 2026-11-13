-- Playlist folders (neezer/webui/playlist_folders.py): a private per-person
-- folder, and where one person filed one playlist. Strictly additive and
-- idempotent, so _repair_skipped_migrations may replay it. No backfill: a
-- playlist without a placement row is unfiled and keeps sorting by creation
-- date, which is exactly today's list.
CREATE TABLE IF NOT EXISTS playlist_folder (
    id CHAR(36) PRIMARY KEY,
    user_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created DATETIME NOT NULL
);
CREATE INDEX IF NOT EXISTS index_playlist_folder_user_position ON playlist_folder(user_id, position);
CREATE INDEX IF NOT EXISTS index_playlist_folder_user_id_fk ON playlist_folder(user_id);

CREATE TABLE IF NOT EXISTS playlist_placement (
    user_id CHAR(36) NOT NULL REFERENCES user ON DELETE CASCADE,
    playlist_id CHAR(36) NOT NULL REFERENCES playlist ON DELETE CASCADE,
    folder_id CHAR(36) REFERENCES playlist_folder ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (user_id, playlist_id)
);
CREATE INDEX IF NOT EXISTS index_playlist_placement_user_folder_position
    ON playlist_placement(user_id, folder_id, position);
CREATE INDEX IF NOT EXISTS index_playlist_placement_playlist_id_fk ON playlist_placement(playlist_id);
CREATE INDEX IF NOT EXISTS index_playlist_placement_folder_id_fk ON playlist_placement(folder_id);
