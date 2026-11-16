-- The local SQLite file, schema version 1 (docs/agents/desktop.md, *Caches*).
--
-- Every object is `IF NOT EXISTS` and every statement is replayable, because
-- this file is BOTH the first migration and the repair path: a boot that finds
-- `PRAGMA user_version` below 1 (a fresh or a wiped file, an older build) runs
-- it again, and a second run must be a no-op (tests/schema: "run v1 twice").
-- Nothing here deletes a row: a migration only ever adds.
--
-- The file lives in the DATA directory ($XDG_DATA_HOME/neezer/desktop.sqlite,
-- WAL); the files it points at live in two: the evictable ones under
-- $XDG_CACHE_HOME/neezer/ (covers, play cache) and the permanent ones under
-- $XDG_DATA_HOME/neezer/offline/ (downloads, never evicted).

-- The API responses, exactly as the server sent them (TEXT, not parsed: one
-- copy of the bytes, and a re-read costs one JSON parse). Keyed by the account
-- so two people sharing a machine never see each other's library.
CREATE TABLE IF NOT EXISTS api_cache(
  owner TEXT NOT NULL,
  path TEXT NOT NULL,
  body TEXT NOT NULL,
  fetched_at INTEGER NOT NULL,
  PRIMARY KEY(owner, path)
);

-- Cover art on disk: one row per remote URL the UI renders, `file` the name
-- under the covers directory. `pinned = 1` is a DOWNLOAD's art, which the LRU
-- must never take: an album downloaded for a flight keeps its pochette. Without
-- it the 300 MB cap would silently strip the art of exactly the albums someone
-- prepared for a plane (the SPA's lib/offline.js keeps its own copy for the
-- same reason).
CREATE TABLE IF NOT EXISTS covers(
  url TEXT PRIMARY KEY,
  file TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  used_at INTEGER NOT NULL,
  pinned INTEGER NOT NULL DEFAULT 0
);

-- Streamed audio of played and prefetched tracks, LRU-evicted. `complete = 0`
-- is a reservation whose bytes are still arriving: never replayed from disk.
CREATE TABLE IF NOT EXISTS play_cache(
  track TEXT NOT NULL,
  quality TEXT NOT NULL,
  file TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  complete INTEGER NOT NULL,
  used_at INTEGER NOT NULL,
  PRIMARY KEY(track, quality)
);

-- The person's downloads ("hors ligne"). One row per track per account; `file`
-- is the name under the offline directory, published only once complete.
CREATE TABLE IF NOT EXISTS downloads(
  owner TEXT NOT NULL,
  track TEXT NOT NULL,
  quality TEXT NOT NULL,
  file TEXT,
  bytes INTEGER,
  state TEXT NOT NULL,
  error TEXT,
  added_at INTEGER NOT NULL,
  PRIMARY KEY(owner, track)
);

-- What was chosen for offline: a playlist, an album, a show. The title is what
-- the UI lists, kept here so the list opens offline too.
CREATE TABLE IF NOT EXISTS download_sets(
  owner TEXT NOT NULL,
  kind TEXT NOT NULL,
  ident TEXT NOT NULL,
  title TEXT NOT NULL,
  added_at INTEGER NOT NULL,
  PRIMARY KEY(owner, kind, ident)
);

-- The set -> track link, which the set row alone cannot express: the same track
-- is in several playlists and in its album, it is downloaded ONCE, and removing
-- one set must take back only the tracks no other set still wants.
CREATE TABLE IF NOT EXISTS download_members(
  owner TEXT NOT NULL,
  kind TEXT NOT NULL,
  ident TEXT NOT NULL,
  track TEXT NOT NULL,
  PRIMARY KEY(owner, kind, ident, track)
);

-- Settings and small documents the SPA keeps in localStorage: a typed key/value
-- pair, the value being its JSON text.
CREATE TABLE IF NOT EXISTS kv(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Podcast positions written while offline, sent on reconnection (last write
-- wins). One row per episode: the newest position replaces the oldest.
CREATE TABLE IF NOT EXISTS pending_progress(
  owner TEXT NOT NULL,
  episode TEXT NOT NULL,
  body TEXT NOT NULL,
  PRIMARY KEY(owner, episode)
);

-- The LRU scans are the only query the startup path and every eviction make.
CREATE INDEX IF NOT EXISTS covers_used_at ON covers(used_at);
CREATE INDEX IF NOT EXISTS play_cache_used_at ON play_cache(used_at);
CREATE INDEX IF NOT EXISTS api_cache_fetched_at ON api_cache(fetched_at);
-- Work is served oldest-first, and a person's whole queue is one owner.
CREATE INDEX IF NOT EXISTS downloads_state ON downloads(owner, state, added_at);
