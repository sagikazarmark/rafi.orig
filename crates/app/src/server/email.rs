//! Email normalization used to detect duplicate entries.

/// Normalizes an email address for duplicate detection.
///
/// Lowercases and trims, strips `+tag` suffixes, and for Gmail removes dots
/// and maps `googlemail.com` to `gmail.com`. Returns `None` for addresses that
/// are obviously malformed.
pub fn normalize(email: &str) -> Option<String> {
    let email = email.trim().to_lowercase();
    let (local, domain) = email.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    if email.chars().any(|c| c.is_whitespace()) || local.contains('@') {
        return None;
    }
    let local = local.split('+').next().unwrap_or(local);
    if local.is_empty() {
        return None;
    }
    let (local, domain) = match domain {
        "gmail.com" | "googlemail.com" => (local.replace('.', ""), "gmail.com"),
        _ => (local.to_owned(), domain),
    };
    Some(format!("{local}@{domain}"))
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn normalizes() {
        assert_eq!(
            normalize(" Foo@Example.COM ").as_deref(),
            Some("foo@example.com")
        );
        assert_eq!(
            normalize("a.b+meetup@gmail.com").as_deref(),
            Some("ab@gmail.com")
        );
        assert_eq!(
            normalize("a.b@googlemail.com").as_deref(),
            Some("ab@gmail.com")
        );
        assert_eq!(
            normalize("a.b+x@example.com").as_deref(),
            Some("a.b@example.com")
        );
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "foo",
            "@x.com",
            "foo@",
            "foo@bar",
            "a b@x.com",
            "+x@x.com",
            "a@.com",
        ] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
    }
}
