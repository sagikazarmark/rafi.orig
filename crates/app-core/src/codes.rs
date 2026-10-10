//! Human-friendly codes (join codes, claim codes).

/// Uppercase alphanumerics without look-alikes (0/O, 1/I/L).
pub const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";

/// Encodes random bytes into a code of `len` characters.
///
/// `bytes` must come from a CSPRNG; each byte picks one character, so there
/// must be at least `len` of them. The modulo
/// bias (256 % 31) is irrelevant for codes that are not used for selection.
pub fn from_bytes(bytes: &[u8], len: usize) -> String {
    assert!(
        bytes.len() >= len,
        "a {len}-character code from {} bytes",
        bytes.len()
    );
    bytes
        .iter()
        .take(len)
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

/// Normalizes user input of a code (case, spaces, dashes). Look-alikes need
/// no mapping: [`ALPHABET`] has neither of a pair, so a mistyped one matches
/// no code either way.
pub fn normalize(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_alphabet() {
        let code = from_bytes(&[0, 30, 31, 255], 4);
        assert_eq!(code.len(), 4);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
    }

    #[test]
    fn normalize_input() {
        assert_eq!(normalize(" k7q2 "), "K7Q2");
        assert_eq!(normalize("ab-cd"), "ABCD");
    }
}
