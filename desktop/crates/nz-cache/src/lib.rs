//! The local SQLite file: API cache, covers, play cache, downloads, settings,
//! queue snapshot.
//!
//! Owned by lot D3 (`docs/agents/desktop.md`, *Caches*). This is D0's skeleton:
//! it proves the pinned rusqlite with its bundled SQLite (no system libsqlite3
//! on a fresh machine or in the package's `depends`) builds and answers.

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_sqlite_answers_and_keeps_user_version() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory database");
        // WAL is only meaningful on a file, but the pragma must be accepted:
        // D3 turns it on for the real file and relies on it for the
        // "read while a write runs" promise.
        let _: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");

        let before: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(before, 0);

        conn.pragma_update(None, "user_version", 1)
            .expect("migrations bump user_version");
        let after: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(after, 1);
    }
}
