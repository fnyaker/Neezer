//! The session: one cookie jar, kept in a file the person owns.
//!
//! `POST /api/login` sets a signed session cookie; every later request carries
//! it (`docs/agents/desktop.md`, *Auth and security*). The jar is a file —
//! `$XDG_STATE_HOME/neezer/session.json`, mode `0600`, one entry per server URL
//! — so restarting the app does not ask for the password again, while a second
//! server's session can never be sent to the first.
//!
//! Two rules this file exists to keep:
//!
//! - **Never log a cookie.** [`SessionJar`]'s `Debug` prints the origin and how
//!   many cookies are held, never a value.
//! - **Hostile disk input cannot crash the app.** A truncated, hand-edited or
//!   foreign `session.json` loads as "no session" instead of panicking; the
//!   person simply signs in again.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use reqwest::Url;
use reqwest::cookie::CookieStore;
use reqwest::header::HeaderValue;

/// The most cookies kept for one origin. The server sends exactly one.
const MAX_COOKIES_PER_ORIGIN: usize = 8;
/// The longest cookie value kept, in bytes. A session cookie is ~200 bytes.
const MAX_COOKIE_BYTES: usize = 4 * 1024;

/// The cookie jar of one [`crate::Api`]: in memory, and mirrored to a file when
/// it has one.
pub struct SessionJar {
    /// The file the jar is mirrored to. `None` keeps it in memory (tests, and a
    /// server URL whose state directory cannot be created).
    path: Option<PathBuf>,
    /// `origin` → `[(name, value)]`. Never printed (see the `Debug` impl).
    cookies: Mutex<BTreeMap<String, Vec<(String, String)>>>,
}

impl fmt::Debug for SessionJar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let origins = self
            .cookies
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .iter()
            .map(|(origin, cookies)| format!("{origin}: {} cookie(s)", cookies.len()))
            .collect::<Vec<_>>();
        f.debug_struct("SessionJar")
            .field("path", &self.path)
            .field("origins", &origins)
            .finish()
    }
}

impl SessionJar {
    /// A jar that lives only in memory.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            cookies: Mutex::new(BTreeMap::new()),
        }
    }

    /// A jar mirrored to `path`, loaded from it when the file exists.
    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let cookies = load(&path).unwrap_or_default();
        Self {
            path: Some(path),
            cookies: Mutex::new(cookies),
        }
    }

    /// `$XDG_STATE_HOME/neezer/session.json`, the app's own jar. `None` when the
    /// platform has no state directory (the jar then stays in memory).
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("fr", "Neezer", "neezer").map(|dirs| {
            dirs.state_dir()
                .unwrap_or_else(|| dirs.data_dir())
                .join("session.json")
        })
    }

    /// The jar [`crate::Api`] uses by default: the app's file, or memory.
    pub fn default_jar() -> Self {
        match Self::default_path() {
            Some(path) => Self::at(path),
            None => Self::in_memory(),
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<(String, String)>>> {
        self.cookies.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Forget every cookie of one origin, and write that to the file. Called on
    /// logout and on a plain `401`.
    pub fn clear(&self, base: &Url) {
        let origin = origin_of(base);
        if let Some(held) = self.held().get_mut(&origin) {
            held.clear();
        }
        self.persist();
    }

    /// Takes a cookie the person's browser already holds (lot MB4's migration
    /// from the old WebView app, `CookieManager.getCookie(origin)`).
    ///
    /// `url` must be the CONFIGURED server's own origin — the caller checks
    /// that, and this crate cannot know it — so a cookie lifted from one host
    /// can never be handed to another. `cookie_header` is what
    /// `CookieManager.getCookie` answers, `a=1; b=2`; each pair goes through the
    /// very validation a `Set-Cookie` does, the file is written like any other
    /// change, and NOTHING is logged: it is a session credential.
    ///
    /// Returns how many cookies were kept (0 for an unreadable URL or header).
    pub fn seed(&self, url: &str, cookie_header: &str) -> usize {
        let Ok(base) = Url::parse(url) else {
            return 0;
        };
        let origin = origin_of(&base);
        // Bound what a caller (or a hostile `CookieManager` answer) can inject:
        // `store`'s own per-cookie rules, applied to a whole header at once.
        let mut kept = 0usize;
        for part in cookie_header.split(';') {
            let pair = part.trim();
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if name.is_empty()
                || name.len() > 64
                || !name.bytes().all(is_cookie_name_byte)
                || value.is_empty()
                || value.len() > MAX_COOKIE_BYTES
            {
                continue;
            }
            let mut cookies = self.held();
            let held = cookies.entry(origin.clone()).or_default();
            if held.len() >= MAX_COOKIES_PER_ORIGIN && !held.iter().any(|(held, _)| held == name) {
                continue;
            }
            held.retain(|(held_name, _)| held_name != name);
            held.push((name.to_owned(), value.to_owned()));
            drop(cookies);
            kept += 1;
        }
        if kept > 0 {
            self.persist();
        }
        kept
    }

    /// How many cookies are held for this origin (a test and a diagnostic read
    /// this; nothing else needs to know).
    pub fn len(&self, base: &Url) -> usize {
        self.held().get(&origin_of(base)).map(Vec::len).unwrap_or(0)
    }

    pub fn is_empty(&self, base: &Url) -> bool {
        self.len(base) == 0
    }

    /// Write the jar to its file, atomically and mode `0600`. A failure is
    /// logged without any cookie content: losing the persisted jar costs one
    /// sign-in, never a request.
    fn persist(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let body = {
            let cookies = self.held();
            serde_json::to_vec(&*cookies).unwrap_or_default()
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // The temporary file is created 0600 BEFORE anything is written, so a
        // crash mid-write never leaves a world-readable session behind.
        let tmp = path.with_extension("json.tmp");
        let written = (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            use std::io::Write;
            file.write_all(&body)?;
            file.sync_all()?;
            std::fs::rename(&tmp, path)
        })();
        if let Err(err) = written {
            tracing::warn!(path = %path.display(), error = %err, "the session file could not be written");
        }
    }

    /// Store one `Set-Cookie` header for `url`.
    fn store(&self, header: &HeaderValue, url: &Url) {
        let Ok(text) = header.to_str() else {
            return;
        };
        let mut parts = text.split(';');
        let Some(pair) = parts.next() else {
            return;
        };
        let Some((name, value)) = pair.split_once('=') else {
            return;
        };
        let name = name.trim();
        if name.is_empty() || name.len() > 64 || !name.bytes().all(is_cookie_name_byte) {
            return;
        }
        let value = value.trim();
        // `Expires=<the epoch>; Max-Age=0` is how the server logs a browser out;
        // an empty value means the same thing.
        let erased = value.is_empty()
            || value.len() > MAX_COOKIE_BYTES
            || parts.any(|attr| {
                let attr = attr.trim();
                attr.eq_ignore_ascii_case("max-age=0")
                    || attr.eq_ignore_ascii_case("expires=thu, 01 jan 1970 00:00:00 gmt")
            });

        let origin = origin_of(url);
        let mut cookies = self.held();
        let held = cookies.entry(origin).or_default();
        held.retain(|(held_name, _)| held_name != name);
        if !erased && held.len() < MAX_COOKIES_PER_ORIGIN {
            held.push((name.to_owned(), value.to_owned()));
        }
        drop(cookies);
        self.persist();
    }
}

impl CookieStore for SessionJar {
    fn set_cookies(&self, cookie_headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        for header in cookie_headers {
            self.store(header, url);
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let origin = origin_of(url);
        let cookies = self.held();
        let held = cookies.get(&origin)?;
        if held.is_empty() {
            return None;
        }
        let joined = held
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        HeaderValue::from_str(&joined).ok()
    }
}

/// `scheme://host:port` — what a cookie is scoped to here. The path is dropped
/// on purpose: the server's cookie is `Path=/`, and a jar that honoured a
/// narrower path would silently stop sending it.
fn origin_of(url: &Url) -> String {
    let mut origin = url.origin().ascii_serialization();
    if origin == "null" {
        // An opaque origin has no cookies to share; key it by the full URL.
        origin = url.as_str().to_owned();
    }
    origin
}

fn is_cookie_name_byte(byte: u8) -> bool {
    byte.is_ascii_graphic() && !b"()<>@,;:\\\"/[]?={}".contains(&byte)
}

/// Read a jar file. Anything unreadable or unexpected is "no session": the
/// person signs in again instead of the app refusing to start.
fn load(path: &Path) -> Option<BTreeMap<String, Vec<(String, String)>>> {
    let raw = std::fs::read(path).ok()?;
    if raw.len() > 64 * 1024 {
        return None;
    }
    let parsed: BTreeMap<String, Vec<(String, String)>> = serde_json::from_slice(&raw).ok()?;
    // Bound what a hand-written file can put in memory.
    let mut bounded = BTreeMap::new();
    for (origin, cookies) in parsed {
        let origin = origin.trim().to_owned();
        if origin.is_empty() || origin.len() > 512 {
            continue;
        }
        let cookies: Vec<(String, String)> = cookies
            .into_iter()
            .filter(|(name, value)| {
                !name.is_empty() && name.len() <= 64 && value.len() <= MAX_COOKIE_BYTES
            })
            .take(MAX_COOKIES_PER_ORIGIN)
            .collect();
        if !cookies.is_empty() {
            bounded.insert(origin, cookies);
        }
    }
    Some(bounded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn temp_path() -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "neezer-session-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a test URL")
    }

    fn header(text: &str) -> HeaderValue {
        HeaderValue::from_str(text).expect("a test header")
    }

    /// The session survives a restart, and only for the server it belongs to:
    /// one file holds several servers' sessions side by side.
    #[test]
    fn a_cookie_is_persisted_per_server_and_never_sent_to_another() {
        let path = temp_path();
        let base = url("http://127.0.0.1:5000");
        {
            let jar = SessionJar::at(&path);
            let value = header("session=abc123; Path=/; HttpOnly");
            let mut headers = std::iter::once(&value);
            jar.set_cookies(&mut headers, &base);
            assert_eq!(jar.len(&base), 1);
        }
        // A second server's session lands in the same file.
        let other = url("https://music.example.org");
        {
            let jar = SessionJar::at(&path);
            assert_eq!(
                jar.cookies(&base).expect("reloaded").to_str().unwrap(),
                "session=abc123"
            );
            let value = header("session=zzz; Path=/");
            let mut headers = std::iter::once(&value);
            jar.set_cookies(&mut headers, &other);
        }
        let jar = SessionJar::at(&path);
        assert_eq!(
            CookieStore::cookies(&jar, &base).unwrap().to_str().unwrap(),
            "session=abc123"
        );
        assert_eq!(
            CookieStore::cookies(&jar, &other)
                .unwrap()
                .to_str()
                .unwrap(),
            "session=zzz"
        );
        // And never to a third host.
        assert!(CookieStore::cookies(&jar, &url("http://127.0.0.1:5001")).is_none());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the session file is private to the person");
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Logout, or a plain `401`: the cookie is gone from memory AND from disk.
    #[test]
    fn a_max_age_zero_set_cookie_erases_the_session_and_logout_clears_the_file() {
        let path = temp_path();
        let base = url("http://127.0.0.1:5000");
        let jar = SessionJar::at(&path);
        let value = header("session=abc123; Path=/");
        let mut headers = std::iter::once(&value);
        jar.set_cookies(&mut headers, &base);
        assert!(CookieStore::cookies(&jar, &base).is_some());

        // What Werkzeug sends on `session.clear()`.
        let erased = header("session=; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Max-Age=0; Path=/");
        let mut headers = std::iter::once(&erased);
        jar.set_cookies(&mut headers, &base);
        assert!(CookieStore::cookies(&jar, &base).is_none());

        // And a fresh jar reads the erasure back.
        let value = header("session=abc123; Path=/");
        let mut headers = std::iter::once(&value);
        jar.set_cookies(&mut headers, &base);
        jar.clear(&base);
        assert!(CookieStore::cookies(&SessionJar::at(&path), &base).is_none());
        let _ = std::fs::remove_file(&path);
    }

    /// Hostile or stale disk input is "no session", never a panic and never a
    /// cookie sent to the wrong host.
    #[test]
    fn a_broken_or_hostile_file_loads_as_no_session() {
        let path = temp_path();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(CookieStore::cookies(&SessionJar::at(&path), &url("http://x")).is_none());

        std::fs::write(
            &path,
            br#"{"http://127.0.0.1:5000":[["session","ok"]],"":[["a","b"]],"http://h":[["","x"],["n","v"]]}"#,
        )
        .unwrap();
        let jar = SessionJar::at(&path);
        assert_eq!(
            CookieStore::cookies(&jar, &url("http://127.0.0.1:5000"))
                .unwrap()
                .to_str()
                .unwrap(),
            "session=ok"
        );
        // An empty cookie name was dropped, the well-formed one kept.
        assert_eq!(
            CookieStore::cookies(&jar, &url("http://h"))
                .unwrap()
                .to_str()
                .unwrap(),
            "n=v"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn debug_never_prints_a_cookie_value() {
        let jar = SessionJar::in_memory();
        let base = url("http://127.0.0.1:5000");
        let value = header("session=super-secret");
        let mut headers = std::iter::once(&value);
        jar.set_cookies(&mut headers, &base);
        let printed = format!("{jar:?}");
        assert!(printed.contains("1 cookie(s)"), "{printed}");
        assert!(!printed.contains("super-secret"), "{printed}");
    }

    /// The migration's graine (lot MB4): a cookie lifted from the old app's
    /// WebView jar is usable by the NEXT request, without a relaunch, and it is
    /// scoped to the URL it was lifted for. A hostile or empty header stores
    /// nothing, and nothing is ever printed.
    #[test]
    fn a_seeded_cookie_is_used_at_once_and_stays_on_its_own_origin() {
        let path = temp_path();
        let base = url("https://neezer.example");
        {
            let jar = SessionJar::at(&path);
            assert_eq!(
                CookieStore::cookies(&jar, &base),
                None,
                "a fresh jar holds nothing"
            );
            assert_eq!(jar.seed(base.as_str(), "session=abc123; other=1"), 2);
            // Read back through the very interface a request goes through: no
            // relaunch, no second load.
            assert_eq!(
                CookieStore::cookies(&jar, &base).unwrap().to_str().unwrap(),
                "session=abc123; other=1"
            );
            // Never to another host, and never to another port of this one.
            assert!(CookieStore::cookies(&jar, &url("https://other.example")).is_none());
            assert!(CookieStore::cookies(&jar, &url("https://neezer.example:8443")).is_none());
        }
        // Persisted like every other change.
        let jar = SessionJar::at(&path);
        assert_eq!(
            CookieStore::cookies(&jar, &base).unwrap().to_str().unwrap(),
            "session=abc123; other=1"
        );

        // Hostile input: no name, an empty value, a name outside the cookie
        // alphabet, an unreadable URL — nothing is stored, nothing panics.
        assert_eq!(jar.seed(base.as_str(), ""), 0);
        assert_eq!(jar.seed(base.as_str(), "=v; a=; b@d=1"), 0);
        assert_eq!(jar.seed("pas une adresse", "session=x"), 0);
        assert_eq!(
            CookieStore::cookies(&jar, &base).unwrap().to_str().unwrap(),
            "session=abc123; other=1",
            "a refused header changed nothing"
        );

        // The same name overwrites, as a browser's cookie jar does.
        assert_eq!(jar.seed(base.as_str(), "session=zzz"), 1);
        assert_eq!(
            CookieStore::cookies(&jar, &base).unwrap().to_str().unwrap(),
            "other=1; session=zzz"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A seeded value never reaches the diagnostic journal.
    #[test]
    fn a_seeded_cookie_is_never_printed() {
        let jar = SessionJar::in_memory();
        let base = url("https://neezer.example");
        jar.seed(base.as_str(), "session=super-secret");
        let printed = format!("{jar:?}");
        assert!(!printed.contains("super-secret"), "{printed}");
    }
}
