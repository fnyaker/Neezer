-- Playlist folders (neezer/webui/playlist_folders.py): a private per-person
-- folder, and where one person filed one playlist. MySQL 5.7 has no
-- CREATE INDEX IF NOT EXISTS, so every index is declared inline: the
-- CREATE TABLE IF NOT EXISTS is the whole idempotence story. No backfill: a
-- playlist without a placement row is unfiled and keeps sorting by creation
-- date, which is exactly today's list.
CREATE TABLE IF NOT EXISTS playlist_folder (
    id CHAR(32) PRIMARY KEY,
    user_id CHAR(32) NOT NULL,
    name VARCHAR(255) NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    created DATETIME NOT NULL,
    CONSTRAINT playlist_folder_user_fk FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE,
    INDEX index_playlist_folder_user_position (user_id, position),
    INDEX index_playlist_folder_user_id_fk (user_id)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS playlist_placement (
    user_id CHAR(32) NOT NULL,
    playlist_id CHAR(32) NOT NULL,
    folder_id CHAR(32),
    position INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT playlist_placement_user_fk FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE,
    CONSTRAINT playlist_placement_playlist_fk FOREIGN KEY (playlist_id) REFERENCES playlist(id) ON DELETE CASCADE,
    CONSTRAINT playlist_placement_folder_fk FOREIGN KEY (folder_id) REFERENCES playlist_folder(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, playlist_id),
    INDEX index_playlist_placement_user_folder_position (user_id, folder_id, position),
    INDEX index_playlist_placement_playlist_id_fk (playlist_id),
    INDEX index_playlist_placement_folder_id_fk (folder_id)
) DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
