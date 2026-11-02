use super::is_invisible;

/// The longest email address, in characters (RFC 5321's path limit).
pub const MAX_EMAIL_CHARS: usize = 254;

/// An entrant's email address: as given, and the key that identifies the
/// entrant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    /// The address as given, trimmed: where mail goes.
    pub raw: String,
    /// The normalized address ([`Email::parse`]): one entry per key.
    pub key: String,
}

impl Email {
    /// Parses an address, or `None` if it is obviously malformed, longer
    /// than [`MAX_EMAIL_CHARS`], or has whitespace, control or invisible
    /// characters ([`is_invisible`]) in it.
    ///
    /// The unseen ones are refused as in a display name: an address with a
    /// zero-width space is another key for the same-looking mailbox (a second
    /// entry), a bidirectional control makes it read differently in the admin
    /// and the winners' export, and neither is in any address mail reaches.
    ///
    /// The key is lowercased, drops a `+tag`, and for Gmail drops dots and
    /// maps `googlemail.com` to `gmail.com`: the spellings of one mailbox
    /// make one entrant.
    pub fn parse(input: &str) -> Option<Self> {
        let raw = input.trim();
        if raw.chars().count() > MAX_EMAIL_CHARS
            || raw
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || is_invisible(c))
        {
            return None;
        }
        Some(Self {
            key: key(raw)?,
            raw: raw.to_owned(),
        })
    }
}

fn key(email: &str) -> Option<String> {
    let email = email.to_lowercase();
    let (local, domain) = email.rsplit_once('@')?;
    if local.is_empty()
        || domain.is_empty()
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
    {
        return None;
    }
    if local.contains('@') {
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
    use super::*;

    #[test]
    fn an_email_key_is_its_own_key() {
        for input in [
            " Foo@Example.COM ",
            "a.b+meetup@gmail.com",
            "x.y@googlemail.com",
            "a+b+c@example.org",
        ] {
            let once = key(input).unwrap();
            assert_eq!(key(&once).as_deref(), Some(once.as_str()), "{input}");
        }
    }

    fn key(input: &str) -> Option<String> {
        Email::parse(input).map(|e| e.key)
    }

    #[test]
    fn email_keys() {
        assert_eq!(key(" Foo@Example.COM ").as_deref(), Some("foo@example.com"));
        assert_eq!(key("a.b+meetup@gmail.com").as_deref(), Some("ab@gmail.com"));
        assert_eq!(key("a.b@googlemail.com").as_deref(), Some("ab@gmail.com"));
        assert_eq!(key("a.b+x@example.com").as_deref(), Some("a.b@example.com"));
        assert_eq!(Email::parse(" A@x.com ").unwrap().raw, "A@x.com");
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
            assert_eq!(key(bad), None, "{bad}");
        }
        let long = format!("{}@example.com", "a".repeat(MAX_EMAIL_CHARS));
        assert_eq!(key(&long), None, "too long");
    }

    /// Nothing unseen in an address, as in a display name: a zero-width
    /// space would make a second key for a mailbox that looks the same, a
    /// bidirectional control would make it read differently.
    #[test]
    fn rejects_control_and_invisible_characters() {
        for bad in [
            "a\tb@x.com",
            "a\u{0}b@x.com",
            "a\u{7}b@x.com",
            "ab@x.com\u{7f}",
            "a\u{85}b@x.com",
            "anna\u{200B}@x.com",
            "anna@x\u{200D}.com",
            "\u{202E}anna@x.com",
            "anna@x.com\u{2066}",
            "an\u{00AD}na@x.com",
            "anna\u{FEFF}@x.com",
            "anna@x.com\u{E0041}",
            "a\u{3164}@x.com",
            "a\u{00A0}b@x.com",
        ] {
            assert_eq!(Email::parse(bad), None, "{bad:?}");
        }
        // Around the address it is trimmed as before; letters beyond ASCII
        // are fine.
        assert_eq!(key("\tanna@x.com\n").as_deref(), Some("anna@x.com"));
        assert_eq!(key("Árvíz@példa.hu").as_deref(), Some("árvíz@példa.hu"));
    }
}
