//! The app's own update, fetched from ITS server (lot MB4; `Updater.kt` and
//! `android.md`'s *Updating itself from the server*).
//!
//! `GET /api/android/apk` serves the APK, or answers `202` + `Retry-After` while
//! its one worker fetches the release (a 10 MB download never happens in a
//! request thread). That `202` body is `{"state":"fetching"}` — a JSON object,
//! not an application — and saving it as an APK is the reported "Le fichier reçu
//! n'est pas une application valide" the first tap after a release used to get.
//!
//! So this file fetches it the only way that can be trusted:
//!
//! - **the transfer is the app's own client**, on the `Bulk` lane: an update is
//!   never what the person just asked for, and rule 1 says it yields (one
//!   transfer, paced at a quarter of a playing stream);
//! - **a body shorter than its own `Content-Length` is refused**, on both ends —
//!   a truncated ZIP is exactly the "preparing / truncated archive" cause;
//! - **a non-APK body is a WAIT, not a failure**: it is recognised (a ZIP starts
//!   with `PK\x03\x04`, the JSON does not) and the fetch retries after the
//!   server's own wait, bounded to three attempts;
//! - **nothing here decides whether the APK may be installed.** Kotlin's
//!   `Updater.kt` checks package, `versionCode` and signing lineage BEFORE the
//!   installer, so a different signature is a sentence rather than an installer
//!   error (`ApkArchive.kt`, its JVM tests, ported as they are).
//!
//! `nz-api`'s download answer carries no header, so the `Retry-After` of a `202`
//! is not read: the wait is the server's own default for that route
//! (`neezer/webui/androidapk.py`, `APK_WAIT = 5`), which is what it sends while a
//! fetch is in flight. A `202` that keeps answering after three attempts is
//! `NotReady` — the UI shows "le serveur prépare la mise à jour" and offers
//! "Réessayer", never a spinner.

use std::path::{Path, PathBuf};

use crate::core::{Core, CoreError};

/// The APK a real build cannot be smaller than (Android's binary manifest alone
/// is bigger than a truncated file); `ApkArchive.MIN_BYTES` on the Kotlin side.
pub const MIN_APK_BYTES: u64 = 64 * 1024;
/// A release APK is ~10 MB; anything past this is not one (the server's own cap,
/// `androidapk.py`'s `MAX_APK_BYTES`).
pub const MAX_APK_BYTES: u64 = 200 * 1024 * 1024;
/// How long a "the server is preparing it" answer is waited, in seconds — the
/// value `androidapk.py` sends in `Retry-After` while its worker runs.
pub const PREPARING_WAIT_S: u64 = 5;
/// Three attempts of the whole fetch, as the old app did.
const ATTEMPTS: u32 = 3;

/// What landed on the disk, for the installer's `FileProvider` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFile {
    /// `cacheDir/update/neezer.apk`, an absolute path the app owns.
    pub path: PathBuf,
    /// Its length, counted as it arrived.
    pub bytes: u64,
}

impl Core {
    /// Fetches the server's APK into `destination` (a directory this app owns,
    /// `cacheDir/update`), on the `Bulk` lane.
    pub async fn fetch_update(&self, destination: &Path) -> Result<UpdateFile, CoreError> {
        let api = self.api()?;
        std::fs::create_dir_all(destination).map_err(|error| CoreError::Internal {
            detail: format!("{} : {error}", destination.display()),
        })?;
        let path = destination.join("neezer.apk");
        let mut preparing = 0u32;

        for attempt in 0..ATTEMPTS {
            // One `.part` at a time and no leftovers: the old app cleared the
            // directory before each try, and a stale APK must never be handed
            // to an installer.
            let _ = std::fs::remove_file(&path);
            let cancel = std::sync::atomic::AtomicBool::new(false);
            let mut announced: Option<u64> = None;
            let report = api
                .download_to(
                    nz_api::Lane::Bulk,
                    "/android/apk",
                    &path,
                    |_written, total| {
                        if total.is_some() {
                            announced = total;
                        }
                    },
                    &cancel,
                )
                .await?;

            if report.bytes > MAX_APK_BYTES {
                let _ = std::fs::remove_file(&path);
                return Err(CoreError::Invalid {
                    detail: "Le fichier annoncé par le serveur est trop volumineux".to_owned(),
                });
            }
            // A body cut short is not a verdict about the update: it is a
            // truncated archive, and the cause says so.
            if let Some(total) = announced
                && total > 0
                && report.bytes != total
            {
                let _ = std::fs::remove_file(&path);
                return Err(CoreError::Invalid {
                    detail: format!(
                        "Téléchargement incomplet ({} octets sur {total})",
                        report.bytes
                    ),
                });
            }

            match std::fs::read(&path) {
                Ok(head) if looks_like_apk(&head) && report.bytes >= MIN_APK_BYTES => {
                    return Ok(UpdateFile {
                        path,
                        bytes: report.bytes,
                    });
                }
                Ok(_) => {
                    // The 202 answer, a short body, or something else the
                    // server sent: wait for its worker and ask again.
                    let _ = std::fs::remove_file(&path);
                    preparing += 1;
                    if attempt + 1 < ATTEMPTS {
                        tokio::time::sleep(std::time::Duration::from_secs(PREPARING_WAIT_S)).await;
                    }
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&path);
                    return Err(CoreError::Internal {
                        detail: format!("{} : {error}", path.display()),
                    });
                }
            }
        }
        if preparing > 0 {
            return Err(CoreError::NotReady {
                retry_after_s: PREPARING_WAIT_S as u32,
            });
        }
        Err(CoreError::Invalid {
            detail: "Le fichier reçu n'est pas une application valide".to_owned(),
        })
    }
}

/// A ZIP carrying Android's BINARY `AndroidManifest.xml` — the check
/// `ApkArchive.looksLikeApk` makes on the JVM, done first here so a JSON body
/// never reaches an installer. The entry itself is Kotlin's to look inside.
fn looks_like_apk(body: &[u8]) -> bool {
    if body.len() as u64 > MAX_APK_BYTES {
        return false;
    }
    let head: &[u8] = &body[..body.len().min(4096)];
    // `PK\x03\x04`: a local file header at offset 0 is what a build produces.
    head.starts_with(b"PK\x03\x04")
}

/// The directory the installer's `FileProvider` serves (`file_paths.xml`'s
/// `update`), for a caller that has the app's cache root.
pub fn update_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("update")
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;
    use std::sync::mpsc::{Receiver, channel};

    use super::*;
    use crate::setup::testing::TempDir;

    /// One answer the test server gives, in the order it is asked for.
    #[derive(Clone)]
    struct Hop {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
        /// Cut the connection after this many body bytes: a truncated archive.
        cut_after: Option<usize>,
    }

    fn apk(bytes: usize) -> Hop {
        let mut body = Vec::with_capacity(bytes);
        body.extend_from_slice(b"PK\x03\x04");
        body.resize(bytes, 0x41);
        Hop {
            status: 200,
            content_type: "application/vnd.android.package-archive",
            body,
            cut_after: None,
        }
    }

    fn preparing() -> Hop {
        Hop {
            status: 202,
            content_type: "application/json",
            body: br#"{"state":"fetching"}"#.to_vec(),
            cut_after: None,
        }
    }

    /// `GET /api/version`, which `set_server` reads before `fetch_update` is
    /// asked for anything: the script's first answer in every test.
    fn version() -> Hop {
        Hop {
            status: 200,
            content_type: "application/json",
            body: br#"{"version":"0.1.0"}"#.to_vec(),
            cut_after: None,
        }
    }

    /// A loopback HTTP server that answers `script` in order and reports what it
    /// saw. `10.0.2.2`-free: nothing leaves the machine and no port is assumed.
    fn server(script: Vec<Hop>) -> (String, Receiver<u64>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let base = format!("http://{}", listener.local_addr().expect("its address"));
        let (sender, seen) = channel();
        let handle = std::thread::spawn(move || {
            for hop in script {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                let _ = socket.set_read_timeout(Some(std::time::Duration::from_secs(20)));
                let _ = read_head(&mut socket);
                let mut response = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    hop.status,
                    hop.content_type,
                    hop.body.len()
                )
                .into_bytes();
                let body = match hop.cut_after {
                    Some(at) => &hop.body[..hop.body.len().min(at)],
                    None => &hop.body[..],
                };
                response.extend_from_slice(body);
                let _ = socket.write_all(&response);
                let _ = sender.send(hop.body.len() as u64);
            }
        });
        (base, seen, handle)
    }

    fn read_head(socket: &mut std::net::TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while socket.read(&mut byte).unwrap_or(0) == 1 {
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    async fn fetch(base: &str, dir: &TempDir) -> Result<UpdateFile, CoreError> {
        let core = dir.core("test");
        // One `GET /api/version` (the script's first hop) and nothing else: the
        // origin is what the fetch needs.
        core.set_server(base).await.expect("the version probe");
        core.fetch_update(&dir.path().join("update")).await
    }

    /// The happy path: the bytes land in `update/neezer.apk`, counted, and the
    /// file on disk IS the APK (a zip's own header first).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_apk_is_fetched_written_and_counted() {
        let (base, seen, server) = server(vec![version(), apk(80 * 1024)]);
        let dir = TempDir::new("update-ok");
        let file = fetch(&base, &dir).await.expect("the APK");
        assert_eq!(file.bytes, 80 * 1024);
        assert_eq!(file.path, dir.path().join("update").join("neezer.apk"));
        let written = std::fs::read(&file.path).expect("on disk");
        assert_eq!(written.len() as u64, file.bytes);
        assert!(written.starts_with(b"PK\x03\x04"));
        let _version = seen.recv().expect("the version answer");
        assert_eq!(seen.recv().expect("the APK answer"), 80 * 1024);
        server.join().expect("the server thread");
    }

    /// A `202` is the server PREPARING the update: its JSON body is never saved
    /// as an APK, the fetch waits, and the next answer's bytes are the file.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_202_is_waited_out_and_its_json_never_saved() {
        let (base, seen, server) = server(vec![version(), preparing(), apk(70 * 1024)]);
        let dir = TempDir::new("update-202");
        let file = fetch(&base, &dir).await.expect("the APK after the wait");
        assert_eq!(file.bytes, 70 * 1024);
        assert!(
            std::fs::read(&file.path)
                .expect("on disk")
                .starts_with(b"PK\x03\x04"),
            "the JSON of the 202 was replaced by the real archive"
        );
        let _version = seen.recv().expect("the version answer");
        assert_eq!(
            seen.recv().expect("the 202 answer"),
            br#"{"state":"fetching"}"#.len() as u64,
            "the preparing body's own length"
        );
        server.join().expect("the server thread");
    }

    /// Only `202`s: three attempts, then `NotReady` with the server's own wait —
    /// a cause the UI can name and retry, not a file.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_server_that_keeps_preparing_answers_not_ready() {
        let (base, _seen, server) = server(vec![version(), preparing(), preparing(), preparing()]);
        let dir = TempDir::new("update-preparing");
        let error = fetch(&base, &dir).await.expect_err("never an APK");
        assert_eq!(
            error,
            CoreError::NotReady {
                retry_after_s: PREPARING_WAIT_S as u32
            }
        );
        assert!(
            !dir.path().join("update").join("neezer.apk").exists(),
            "nothing was left for an installer"
        );
        server.join().expect("the server thread");
    }

    /// A body too short to be an APK (a truncated archive, a hand-written file)
    /// is refused with its cause, and nothing is left for an installer. A body
    /// that DIES mid-transfer never even gets here: `nz-api`'s download answers
    /// `Offline` for it, which the screen shows as a cause too.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_body_too_short_to_be_an_apk_is_refused_and_removed() {
        let mut hop = apk(8 * 1024);
        hop.cut_after = Some(8 * 1024);
        let (base, _seen, server) = server(vec![version(), hop.clone(), hop.clone(), hop]);
        let dir = TempDir::new("update-short");
        let error = fetch(&base, &dir).await.expect_err("not an APK");
        assert_eq!(
            error,
            CoreError::NotReady {
                retry_after_s: PREPARING_WAIT_S as u32
            },
            "a short body is retried, then named as a wait, never installed"
        );
        assert!(!dir.path().join("update").join("neezer.apk").exists());
        server.join().expect("the server thread");
    }

    /// A body that stops mid-transfer is a cause too, and not one this lot
    /// invents: `nz-api`'s own download rule says `Offline` for it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_body_that_dies_mid_transfer_is_a_cause() {
        let mut hop = apk(80 * 1024);
        hop.cut_after = Some(8 * 1024);
        let (base, _seen, server) = server(vec![version(), hop]);
        let dir = TempDir::new("update-truncated");
        let error = fetch(&base, &dir).await.expect_err("truncated");
        assert_eq!(error, CoreError::Offline);
        assert!(!dir.path().join("update").join("neezer.apk").exists());
        server.join().expect("the server thread");
    }
}
