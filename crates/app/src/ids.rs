//! Random identifiers, codes and tokens (all from the OS CSPRNG).

use sha2::{Digest, Sha256};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("OS randomness is available");
    buf
}

/// A random 128-bit lowercase hex ID.
pub fn id() -> String {
    hex::encode(random_bytes::<16>())
}

/// A short human-friendly code (join codes, claim codes) of `len` (at most
/// 16) characters.
pub fn code(len: usize) -> String {
    crate::codes::from_bytes(&random_bytes::<16>(), len)
}

/// A random generated display name, like "Brave Otter"
/// ([`rafi_lifecycle::entrant::generated_name`]).
pub fn generated_name() -> String {
    rafi_lifecycle::entrant::generated_name(u64::from_le_bytes(random_bytes::<8>()))
}

/// A random generated display name that isn't in `taken`, if one turns up in a
/// few tries; otherwise one that is (entrants are told apart by their ID).
pub fn generated_name_besides(taken: &std::collections::HashSet<String>) -> String {
    let mut name = generated_name();
    for _ in 0..20 {
        if !taken.contains(&name) {
            break;
        }
        name = generated_name();
    }
    name
}

/// A URL slug derived from a title plus a random suffix.
pub fn slug(title: &str) -> String {
    let mut base: String = title
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' | 'ö' | 'ő' => 'o',
            'ú' | 'ü' | 'ű' => 'u',
            c if c.is_ascii_alphanumeric() => c,
            _ => '-',
        })
        .collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let base = base.trim_matches('-');
    let base: String = base.chars().take(32).collect();
    let suffix = code(4).to_lowercase();
    if base.is_empty() {
        suffix
    } else {
        format!("{}-{suffix}", base.trim_end_matches('-'))
    }
}

/// A secret token for the participant cookie.
pub fn token() -> String {
    hex::encode(random_bytes::<16>())
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn slugs() {
        let s = super::slug("KCD Budapest Meetup #3!");
        assert!(s.starts_with("kcd-budapest-meetup-3-"), "{s}");
        let s = super::slug("Árvíztűrő tükörfúrógép");
        assert!(s.starts_with("arvizturo-tukorfurogep-"), "{s}");
        assert_eq!(super::slug("!!!").len(), 4);
    }
}
