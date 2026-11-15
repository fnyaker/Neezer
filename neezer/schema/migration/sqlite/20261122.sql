-- Public share links whose playlist or owner is already gone. SQLite declared
-- the two foreign keys with ON DELETE CASCADE since 20261113, so this normally
-- removes nothing; it is here because the same date is a real migration on
-- MySQL, and every engine ships the same shape at the same version.
-- Idempotent: a DELETE of what no longer exists.
DELETE FROM playlist_share
WHERE playlist_id NOT IN (SELECT id FROM playlist)
   OR user_id NOT IN (SELECT id FROM user);
