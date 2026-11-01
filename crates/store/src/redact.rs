//! A database URL fit for a log line or an error message.

/// `url` without its credentials: the user and password before the host
/// (`postgres://user:password@host/db` → `postgres://host/db`), and the
/// value of a `password` query parameter. The scheme, host, port, database
/// and other parameters stay, which is what an operator needs to tell which
/// database a process opened. A URL without credentials (SQLite's) comes
/// back as it is.
pub fn redact_url(url: &str) -> String {
    // SQLite's URL is a path, which may hold an `@`, and never credentials.
    if url
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("sqlite:"))
    {
        return url.to_owned();
    }
    let (scheme, rest) = match url.split_once("://") {
        Some((scheme, rest)) => (Some(scheme), rest),
        None => (None, url),
    };
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };
    // The authority ends at the first `/`; its user information at the
    // authority's last `@` (a password may itself hold an `@`).
    let authority_end = path.find('/').unwrap_or(path.len());
    let (authority, tail) = path.split_at(authority_end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let mut out = String::with_capacity(url.len());
    if let Some(scheme) = scheme {
        out.push_str(scheme);
        out.push_str("://");
    }
    out.push_str(host);
    out.push_str(tail);
    if let Some(query) = query {
        out.push('?');
        let params: Vec<String> = query
            .split('&')
            .map(|param| match param.split_once('=') {
                Some((key, _)) if key.eq_ignore_ascii_case("password") => format!("{key}=redacted"),
                _ => param.to_owned(),
            })
            .collect();
        out.push_str(&params.join("&"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact_url;

    #[test]
    fn credentials_are_left_out() {
        let redacted = redact_url("postgres://alice:s3cret@db.example:5432/rafi");
        assert_eq!(redacted, "postgres://db.example:5432/rafi");
        assert!(!redacted.contains("alice") && !redacted.contains("s3cret"));
        // A password holding an `@`, and a user without one.
        assert_eq!(
            redact_url("postgresql://alice:p@ss@db/rafi"),
            "postgresql://db/rafi"
        );
        assert_eq!(redact_url("postgres://alice@db/rafi"), "postgres://db/rafi");
        // The query's password too; the other parameters stay.
        let redacted = redact_url("postgres://db/rafi?sslmode=require&password=s3cret");
        assert_eq!(
            redacted,
            "postgres://db/rafi?sslmode=require&password=redacted"
        );
    }

    #[test]
    fn urls_without_credentials_are_unchanged() {
        for url in [
            "sqlite://rafi.db",
            "sqlite::memory:",
            "sqlite:///data/rafi.db?mode=rwc",
            "sqlite://./at@home/rafi.db",
            "postgres://db.example/rafi",
        ] {
            assert_eq!(redact_url(url), url);
        }
    }
}
