//! The one URL reader the core trusts: an absolute address, split into the
//! origin a session belongs to and the path a request names.
//!
//! Two security decisions live here and nowhere else (docs/agents/mobile.md,
//! *Covers* and *Auth*): a cleartext (`http://`) request is only ever sent to
//! the **configured origin** — the LAN install the person typed — and a cover
//! may only come from that origin or from `https://*.dzcdn.net`, never from a
//! private IP literal an album row could name. `nz-api` refuses the rest of the
//! rule (a redirect is followed by hand, the cookie never leaves the origin);
//! this file decides what is even worth asking for.
//!
//! A handful of lines rather than a dependency: the phone build is measured in
//! bytes, and everything the app needs is scheme, host, port, path and query —
//! which is what [`Url::parse`] keeps from something hostile.

/// One origin (`scheme://host[:port]`), normalized the way every comparison
/// below wants it: lowercase scheme and host, the default port written out,
/// no trailing slash.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    /// Reads `scheme://host[:port]` — and nothing else. A path, a query, a
    /// fragment or userinfo is refused rather than ignored, because an origin
    /// this app stores must be exactly what the person typed.
    pub fn parse(raw: &str) -> Option<Origin> {
        let url = Url::parse(raw)?;
        if !url.path.is_empty() && url.path != "/" {
            return None;
        }
        if url.query.is_some() || url.fragment.is_some() || url.userinfo {
            return None;
        }
        Some(url.origin())
    }

    /// The origin of an absolute URL: what a link may name.
    pub fn of(url: &Url) -> Origin {
        url.origin()
    }

    /// `https://host:port`, the ONE spelling stored on disk and compared.
    pub fn as_str(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// `true` for the schemes that are safe on a hostile network.
    pub fn is_secure(&self) -> bool {
        self.scheme == "https"
    }

    /// The host is a bare IP address rather than a name.
    pub fn host_is_ip_literal(&self) -> bool {
        is_ip_literal(&self.host)
    }
}

/// The parts of an absolute URL this app reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    /// `https` or `http`, lowercased.
    pub scheme: String,
    /// Lowercased, without brackets ([`Url::host`] keeps an IPv6 literal's
    /// brackets so it can be put back in an address).
    host: String,
    /// The explicit port, or the scheme's default.
    port: u16,
    /// `""` or a path starting with `/`, percent-encoded as it came.
    pub path: String,
    /// Without the `?`.
    pub query: Option<String>,
    /// Without the `#`.
    pub fragment: Option<String>,
    /// `true` when the URL carried `user:password@`.
    pub userinfo: bool,
}

impl Url {
    /// Parses an absolute `http`/`https` URL. Anything else — a relative
    /// address, another scheme, an unparseable host — is `None`, so no caller
    /// can invent a fetch out of a fragment of text.
    pub fn parse(raw: &str) -> Option<Url> {
        let raw = raw.trim();
        let (scheme, rest) = raw.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "https" => 443,
            "http" => 80,
            _ => return None,
        };

        // The authority ends at the first `/`, `?` or `#`.
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, tail) = rest.split_at(end);

        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((_, host_port)) => (true, host_port),
            None => (false, authority),
        };
        if host_port.is_empty() {
            return None;
        }

        let (host, port) = if let Some(rest) = host_port.strip_prefix('[') {
            // An IPv6 literal: `[::1]:5000`.
            let (inside, after) = rest.split_once(']')?;
            let port = match after.strip_prefix(':') {
                Some(port) => port.parse::<u16>().ok()?,
                None if after.is_empty() => default_port,
                None => return None,
            };
            if !inside.contains(':') || inside.parse::<std::net::Ipv6Addr>().is_err() {
                return None;
            }
            (format!("[{inside}]"), port)
        } else {
            let (host, port) = match host_port.rsplit_once(':') {
                Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
                None => (host_port, None),
            };
            let host = host.to_ascii_lowercase();
            if host.is_empty()
                || host.starts_with('.')
                || host.ends_with('.')
                || host.contains("..")
                || host
                    .chars()
                    .any(|c| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_'))
            {
                return None;
            }
            (host, port.unwrap_or(default_port))
        };
        if port == 0 {
            return None;
        }

        // What is left is `path[?query][#fragment]`.
        let (before_fragment, fragment) = match tail.split_once('#') {
            Some((before, fragment)) => (before, Some(fragment.to_owned())),
            None => (tail, None),
        };
        let (path, query) = match before_fragment.split_once('?') {
            Some((path, query)) => (path.to_owned(), Some(query.to_owned())),
            None => (before_fragment.to_owned(), None),
        };
        if !path.is_empty() && !path.starts_with('/') {
            return None;
        }

        Some(Url {
            scheme,
            host,
            port,
            path,
            query,
            fragment,
            userinfo,
        })
    }

    /// The origin this URL belongs to, default port written out.
    pub fn origin(&self) -> Origin {
        Origin {
            scheme: self.scheme.clone(),
            host: self.host.clone(),
            port: self.port,
        }
    }

    /// The host, without an IPv6 literal's brackets.
    pub fn hostname(&self) -> &str {
        self.host
            .strip_prefix('[')
            .and_then(|host| host.strip_suffix(']'))
            .unwrap_or(&self.host)
    }

    /// `path` + `?query`, the shape an API path takes (`/cover/12?size=big`).
    pub fn path_and_query(&self) -> String {
        match &self.query {
            Some(query) => format!("{}?{query}", self.path),
            None => self.path.clone(),
        }
    }

    /// Whether `host` is exactly `suffix` or one of its subdomains.
    pub fn host_is_or_under(&self, suffix: &str) -> bool {
        let host = self.hostname();
        host == suffix || host.ends_with(&format!(".{suffix}"))
    }
}

/// A bare IPv4 or IPv6 address (brackets allowed).
pub fn is_ip_literal(host: &str) -> bool {
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.parse::<std::net::Ipv4Addr>().is_ok() || host.parse::<std::net::Ipv6Addr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_the_scheme_host_and_port_and_nothing_else() {
        let origin = Origin::parse("https://Neezer.Example").expect("an origin");
        assert_eq!(origin.as_str(), "https://neezer.example:443");
        assert_eq!(
            Origin::parse("http://192.168.1.10:5000").unwrap().as_str(),
            "http://192.168.1.10:5000"
        );
        assert_eq!(
            Origin::parse("http://192.168.1.10:5000/").unwrap().as_str(),
            "http://192.168.1.10:5000"
        );
        // A path is not an origin: the setup screen must not store one.
        assert!(Origin::parse("https://host/neezer").is_none());
        assert!(Origin::parse("https://host?x=1").is_none());
        assert!(Origin::parse("https://user:pw@host").is_none());
        assert!(Origin::parse("ftp://host").is_none());
        assert!(Origin::parse("host:5000").is_none());
    }

    #[test]
    fn a_url_keeps_its_path_query_and_fragment_apart() {
        let url =
            Url::parse("https://e-cdns-images.dzcdn.net/images/cover/ab/264x264.jpg?x=1#frag")
                .expect("a CDN url");
        assert_eq!(url.hostname(), "e-cdns-images.dzcdn.net");
        assert_eq!(url.origin().as_str(), "https://e-cdns-images.dzcdn.net:443");
        assert_eq!(url.path_and_query(), "/images/cover/ab/264x264.jpg?x=1");
        assert_eq!(url.fragment.as_deref(), Some("frag"));
        assert!(url.host_is_or_under("dzcdn.net"));
    }

    #[test]
    fn a_hostile_url_is_refused_instead_of_guessed() {
        for bad in [
            "",
            "x.jpg",
            "/api/cover/1",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "https://host:99999/x",
            "https://host:0/x",
            "https://.host/x",
            "https://ho st/x",
            "http://[::1/x",
        ] {
            assert!(Url::parse(bad).is_none(), "{bad} parsed");
        }
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_in_the_origin() {
        let url = Url::parse("http://[::1]:8080/api/version").unwrap();
        assert_eq!(url.hostname(), "::1");
        assert_eq!(url.origin().as_str(), "http://[::1]:8080");
        assert!(url.origin().host_is_ip_literal());
        assert!(is_ip_literal("10.0.0.1"));
        assert!(!is_ip_literal("e-cdns-images.dzcdn.net"));
    }

    #[test]
    fn a_subdomain_matches_but_a_lookalike_does_not() {
        let url = Url::parse("https://evil-dzcdn.net/x.jpg").unwrap();
        assert!(!url.host_is_or_under("dzcdn.net"));
        let url = Url::parse("https://cdn.dzcdn.net/x.jpg").unwrap();
        assert!(url.host_is_or_under("dzcdn.net"));
    }
}
