//! What a browser tab is allowed to leave on disk, and in what shape.
//!
//! Decision #61 C6 reduced a persisted webview URL to its **origin** because
//! every other part of a URL can carry a credential: `user:pass@`, `?code=…`,
//! `#access_token=…`, `/reset/<token>`, `/oauth/callback/<code>`. That is the
//! right default and stays the default. The complaint it produced — "I restart
//! and I'm back at github.com instead of my PR" — is answered by making it a
//! POLICY rather than the only behaviour.
//!
//! The canonicalisation itself used to live in `PaneManager.swift` only (Linux
//! never persisted webviews at all), so there was nothing keeping the two
//! platforms honest. It lives here now and both call it.

use serde::{Deserialize, Serialize};

/// How much of a live URL may reach disk.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RestorePolicy {
    /// `scheme://host[:port]` only. Path, query, fragment and userinfo are all
    /// dropped. The historical behaviour and still the default.
    #[default]
    Origin,
    /// The full URL minus userinfo and minus any query/fragment parameter whose
    /// key is in [`TOKEN_KEYS`]. **Sensitive persistence** — see the caveat below.
    Url,
    /// [`RestorePolicy::Url`] plus the opaque platform history blob (back/forward
    /// list + scroll). **Sensitive persistence.**
    Full,
}

impl RestorePolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "origin" => Some(Self::Origin),
            "url" => Some(Self::Url),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    /// True when this policy writes more than an origin, i.e. when the layout
    /// file must be treated as sensitive.
    ///
    /// The token denylist below is **hygiene, not a guarantee**: it cannot save
    /// a `/reset/<token>` PATH or an `/oauth/callback/<code>` path, and mode
    /// 0600 is no defence against an agent running as the same user. Choosing
    /// `url` or `full` is an informed opt-in (plan §5).
    pub fn is_sensitive(self) -> bool {
        !matches!(self, Self::Origin)
    }

    /// Whether the opaque history blob may be written at all.
    pub fn keeps_history(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Query/fragment parameter names whose values are dropped under
/// [`RestorePolicy::Url`] and [`RestorePolicy::Full`]. Matched
/// case-insensitively against the whole key.
pub const TOKEN_KEYS: &[&str] = &[
    "access_token",
    "auth",
    "authorization",
    "code",
    "id_token",
    "key",
    "password",
    "refresh_token",
    "session",
    "sig",
    "signature",
    "state",
    "token",
];

/// Reduce a live URL to `scheme://host[:port]`, http(s) only.
///
/// Returns `""` for anything non-http(s), hostless, or unparseable — the caller
/// restores the blank URL-entry placeholder for that. The scheme is compared
/// case-insensitively because URL schemes are, so `HTTPS://…` must not be
/// silently dropped.
pub fn canonical_origin(raw: &str) -> String {
    let Some(parts) = split_url(raw) else {
        return String::new();
    };
    parts.origin
}

/// Apply a [`RestorePolicy`] to a live URL, producing the string that goes on
/// disk. Never returns userinfo under any policy.
pub fn canonicalize_for_restore(raw: &str, policy: RestorePolicy) -> String {
    let Some(parts) = split_url(raw) else {
        return String::new();
    };
    match policy {
        RestorePolicy::Origin => parts.origin,
        RestorePolicy::Url | RestorePolicy::Full => {
            let mut out = parts.origin;
            out.push_str(&parts.path);
            if let Some(q) = &parts.query
                && let Some(scrubbed) = scrub_params(q)
            {
                out.push('?');
                out.push_str(&scrubbed);
            }
            if let Some(f) = &parts.fragment
                && let Some(scrubbed) = scrub_params(f)
            {
                out.push('#');
                out.push_str(&scrubbed);
            }
            out
        }
    }
}

/// Drop `key=value` pairs whose key is in [`TOKEN_KEYS`]. A fragment that is not
/// in `k=v` form (a plain anchor like `#install`) is passed through whole.
/// Returns `None` when nothing survives, so the caller omits the separator too.
fn scrub_params(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    if !raw.contains('=') {
        // A plain anchor. Nothing to key off, and no `=` means no `k=v` token.
        return Some(raw.to_string());
    }
    let kept: Vec<&str> = raw
        .split('&')
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            !is_token_key(key)
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("&"))
    }
}

/// Does this raw query key denote a token parameter?
///
/// The key is percent-decoded first: `?%63ode=SECRET` is `?code=SECRET` to every
/// server that parses it, so matching the raw spelling would let an OAuth code
/// through untouched (codex review r3-C2). `+` is decoded to a space as well,
/// since query strings use form encoding. Only the COMPARISON uses the decoded
/// form — a parameter we keep is written back with its original spelling, so we
/// never re-encode a URL differently from how the site issued it.
fn is_token_key(raw_key: &str) -> bool {
    let decoded = percent_decode_lossy(raw_key);
    TOKEN_KEYS
        .iter()
        .any(|deny| decoded.eq_ignore_ascii_case(deny))
}

/// Decode `%XX` escapes and `+`. Invalid escapes are left verbatim (lossy), which
/// is the safe direction here: a key we fail to decode simply keeps its raw
/// spelling and is compared as-is.
fn percent_decode_lossy(raw: &str) -> String {
    if !raw.contains('%') && !raw.contains('+') {
        return raw.to_string();
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Strict host charset. A registered name is ASCII letters/digits/`-`/`.`/`_`
/// (IDN reaches us as punycode, which is ASCII); an IPv6 literal is bracketed
/// hex/colons/dots. Anything else — a backslash, a space, a quote, a stray
/// delimiter — means we mis-parsed or the input is hostile, and we refuse rather
/// than emit an "origin" that is really a path.
fn is_plausible_host(host: &str) -> bool {
    if host.is_empty() {
        return false;
    }
    if let Some(inner) = host.strip_prefix('[') {
        let Some(inner) = inner.strip_suffix(']') else {
            return false;
        };
        return !inner.is_empty()
            && inner
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }
    // A single trailing dot is the DNS root label and makes a name fully
    // qualified — `https://example.com./account` is valid and the Swift
    // canonicalizer this replaces preserved it. Strip it before the empty-label
    // check so a legal FQDN is not turned into a blank pane.
    let labels = host.strip_suffix('.').unwrap_or(host);
    !labels.is_empty()
        && !labels.starts_with('.')
        && !labels.ends_with('.')
        && labels
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
}

struct UrlParts {
    origin: String,
    path: String,
    query: Option<String>,
    fragment: Option<String>,
}

/// Minimal http(s) URL splitter. `copad-core` deliberately carries no `url`
/// crate (see its Cargo.toml — the dependency set is kept small because every
/// crate here also builds into the FFI staticlib the macOS app links), and the
/// grammar we accept is narrow enough to hand-roll safely: we only ever emit an
/// origin we rebuilt ourselves, never a substring of the input.
fn split_url(raw: &str) -> Option<UrlParts> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }

    // Authority runs to the first delimiter. A BACKSLASH counts: browsers treat
    // it as '/' in the authority, so without it `https://example.com\reset\TOKEN`
    // parses as one enormous "hostname" and the origin-only policy would emit the
    // whole string — path, token and all (codex review C2).
    let auth_end = rest.find(['/', '\\', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    let tail = &rest[auth_end..];

    // Strip userinfo — never persisted under any policy.
    let hostport = match authority.rsplit_once('@') {
        Some((_userinfo, hp)) => hp,
        None => authority,
    };
    if hostport.is_empty() {
        return None;
    }

    // Split host:port, taking IPv6 literals into account.
    let (host, port) = if let Some(close) = hostport.rfind(']') {
        let host = &hostport[..=close];
        let port = hostport[close + 1..].strip_prefix(':');
        (host, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if !is_plausible_host(host) {
        return None;
    }
    // A port must be numeric; a non-numeric tail means we mis-split (or the URL
    // is malformed) and we refuse rather than guess.
    if let Some(p) = port
        && (p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }

    let mut origin = format!("{scheme}://{}", host.to_ascii_lowercase());
    if let Some(p) = port {
        origin.push(':');
        origin.push_str(p);
    }

    // Split the tail into path / query / fragment. Query wins over fragment
    // when both are present, matching URL grammar (`?` before `#`).
    let (before_frag, fragment) = match tail.split_once('#') {
        Some((b, f)) => (b, Some(f.to_string())),
        None => (tail, None),
    };
    let (path, query) = match before_frag.split_once('?') {
        Some((p, q)) => (p.to_string(), Some(q.to_string())),
        None => (before_frag.to_string(), None),
    };

    Some(UrlParts {
        origin,
        path,
        query,
        fragment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_matches_the_swift_behaviour_it_replaces() {
        assert_eq!(
            canonical_origin("https://github.com/a/b?c=d#e"),
            "https://github.com"
        );
        assert_eq!(
            canonical_origin("https://example.com:8443/x"),
            "https://example.com:8443"
        );
        assert_eq!(
            canonical_origin("HTTPS://Example.COM/x"),
            "https://example.com"
        );
        assert_eq!(
            canonical_origin("http://u:p@example.com/x"),
            "http://example.com"
        );
    }

    #[test]
    fn origin_rejects_non_http_and_hostless() {
        assert_eq!(canonical_origin("file:///etc/passwd"), "");
        assert_eq!(canonical_origin("about:blank"), "");
        assert_eq!(canonical_origin("javascript:alert(1)"), "");
        assert_eq!(canonical_origin("https:///nohost"), "");
        assert_eq!(canonical_origin(""), "");
    }

    #[test]
    fn ipv6_literals_keep_their_brackets_and_port() {
        assert_eq!(canonical_origin("http://[::1]:3000/x"), "http://[::1]:3000");
        assert_eq!(canonical_origin("http://[::1]/x"), "http://[::1]");
    }

    #[test]
    fn a_backslash_cannot_smuggle_a_path_into_the_origin() {
        // codex review C2. Browsers normalize `\` to `/` in the authority, so
        // this URL's real origin is `https://example.com` — emitting the whole
        // string would leak the very token origin-only mode exists to drop.
        assert_eq!(
            canonical_origin(r"https://example.com\reset\SECRET"),
            "https://example.com"
        );
        assert_eq!(
            canonicalize_for_restore(r"https://example.com\reset\SECRET", RestorePolicy::Origin),
            "https://example.com"
        );
    }

    #[test]
    fn an_implausible_host_is_refused_rather_than_echoed() {
        for raw in [
            "https://exa mple.com/x",
            "https://exa\"mple.com/x",
            "https://.example.com/x",
            "https://example.com../x",
            "https://[not-hex]/x",
            "https://[]/x",
        ] {
            assert_eq!(canonical_origin(raw), "", "{raw}");
        }
    }

    #[test]
    fn ordinary_hosts_still_parse_after_the_charset_tightening() {
        assert_eq!(
            canonical_origin("https://sub.example.co.uk/x"),
            "https://sub.example.co.uk"
        );
        assert_eq!(canonical_origin("http://my_host/x"), "http://my_host");
        // A trailing dot is the DNS root label — a fully qualified name, not a
        // malformed one (codex review C1).
        assert_eq!(
            canonical_origin("https://example.com./account"),
            "https://example.com."
        );
        assert_eq!(
            canonical_origin("http://127.0.0.1:8080/x"),
            "http://127.0.0.1:8080"
        );
        // Punycode IDN is ASCII and must survive.
        assert_eq!(
            canonical_origin("https://xn--9t4b11yi5a.com/x"),
            "https://xn--9t4b11yi5a.com"
        );
    }

    #[test]
    fn a_non_numeric_port_is_refused_rather_than_guessed() {
        assert_eq!(canonical_origin("https://example.com:notaport/x"), "");
    }

    #[test]
    fn url_policy_keeps_the_path_that_origin_threw_away() {
        assert_eq!(
            canonicalize_for_restore("https://github.com/o/r/pull/42", RestorePolicy::Url),
            "https://github.com/o/r/pull/42"
        );
    }

    #[test]
    fn url_policy_drops_userinfo_and_token_params() {
        assert_eq!(
            canonicalize_for_restore(
                "https://u:p@example.com/cb?code=SECRET&page=2#access_token=X",
                RestorePolicy::Url
            ),
            "https://example.com/cb?page=2"
        );
    }

    #[test]
    fn token_key_match_is_case_insensitive() {
        assert_eq!(
            canonicalize_for_restore("https://e.com/x?Access_Token=a&ok=1", RestorePolicy::Url),
            "https://e.com/x?ok=1"
        );
    }

    #[test]
    fn a_percent_encoded_token_key_is_still_recognised() {
        // codex review r3-C2: `%63ode` is `code` to every server that parses it.
        assert_eq!(
            canonicalize_for_restore("https://e.com/?%63ode=SECRET&page=2", RestorePolicy::Url),
            "https://e.com/?page=2"
        );
        assert_eq!(
            canonicalize_for_restore("https://e.com/?%41ccess%5Ftoken=X", RestorePolicy::Url),
            "https://e.com/"
        );
    }

    #[test]
    fn a_kept_parameter_keeps_its_original_spelling() {
        // We compare decoded but write back verbatim, so a site's own encoding
        // survives a round-trip through the restore policy.
        assert_eq!(
            canonicalize_for_restore("https://e.com/?q=a%20b&r=c+d", RestorePolicy::Url),
            "https://e.com/?q=a%20b&r=c+d"
        );
    }

    #[test]
    fn a_malformed_escape_does_not_panic_and_keeps_the_parameter() {
        assert_eq!(
            canonicalize_for_restore("https://e.com/?%zz=1&%=2&x=3", RestorePolicy::Url),
            "https://e.com/?%zz=1&%=2&x=3"
        );
    }

    #[test]
    fn a_plain_anchor_survives_but_a_token_fragment_does_not() {
        assert_eq!(
            canonicalize_for_restore("https://e.com/d#install", RestorePolicy::Url),
            "https://e.com/d#install"
        );
        assert_eq!(
            canonicalize_for_restore("https://e.com/d#token=abc", RestorePolicy::Url),
            "https://e.com/d"
        );
    }

    #[test]
    fn full_policy_writes_the_same_url_as_url_policy() {
        // `full` differs only in that it ALSO keeps the history blob; the URL
        // text it persists is identical, so the two must not drift.
        let raw = "https://e.com/a?code=x&b=1";
        assert_eq!(
            canonicalize_for_restore(raw, RestorePolicy::Full),
            canonicalize_for_restore(raw, RestorePolicy::Url)
        );
    }

    #[test]
    fn policy_flags_say_what_the_docs_say() {
        assert!(!RestorePolicy::Origin.is_sensitive());
        assert!(RestorePolicy::Url.is_sensitive());
        assert!(RestorePolicy::Full.is_sensitive());
        assert!(!RestorePolicy::Url.keeps_history());
        assert!(RestorePolicy::Full.keeps_history());
        assert_eq!(RestorePolicy::default(), RestorePolicy::Origin);
    }

    #[test]
    fn policy_parses_from_config_text() {
        assert_eq!(RestorePolicy::parse("  FULL "), Some(RestorePolicy::Full));
        assert_eq!(RestorePolicy::parse("nonsense"), None);
    }
}
