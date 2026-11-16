//! The only HTTP client of the app: session, lanes, errors, one typed function
//! per server route.
//!
//! Owned by lot D2 (`docs/agents/desktop.md`, *Network errors* and *Rule 1 on
//! the device: lanes*). This is D0's skeleton: it proves the pinned reqwest
//! (rustls, cookie jar, stream) and tokio actually build and link here.

#[cfg(test)]
mod tests {
    /// The cookie jar and rustls TLS backend D2 relies on, built once with no
    /// network involved: `Api` holds exactly this client.
    #[test]
    fn a_client_with_a_cookie_jar_builds() {
        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .expect("reqwest client");
        assert!(client.get("http://127.0.0.1:1/api/me").build().is_ok());
    }

    /// The runtime the request lanes and the MPRIS server run on.
    #[tokio::test]
    async fn the_multi_thread_runtime_runs_task_macros() {
        let value = tokio::spawn(async { 6 * 7 }).await.expect("join");
        assert_eq!(value, 42);
    }
}
