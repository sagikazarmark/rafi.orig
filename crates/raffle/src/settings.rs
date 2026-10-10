//! What a raffle's settings and prizes may hold.

/// The longest raffle title or prize name, in characters.
pub const MAX_TITLE_CHARS: usize = 120;
/// The longest description, in characters.
pub const MAX_DESCRIPTION_CHARS: usize = 2000;
/// The longest rules text, in characters.
pub const MAX_RULES_CHARS: usize = 5000;
/// The most copies of one prize: each is an item of the drawing.
pub const MAX_PRIZE_QTY: u32 = 500;

/// `input` trimmed, if it is a valid title (or prize name): 1 to
/// [`MAX_TITLE_CHARS`] characters.
pub fn title(input: &str) -> Option<&str> {
    let title = input.trim();
    (!title.is_empty() && title.chars().count() <= MAX_TITLE_CHARS).then_some(title)
}

/// Whether a description and rules fit.
pub fn texts_fit(description: &str, rules: &str) -> bool {
    description.chars().count() <= MAX_DESCRIPTION_CHARS && rules.chars().count() <= MAX_RULES_CHARS
}

/// Whether a prize can have `qty` copies.
pub fn prize_qty(qty: u32) -> bool {
    (1..=MAX_PRIZE_QTY).contains(&qty)
}

/// The title of a raffle copied from one titled `title`: still a valid title.
pub fn copy_title(title: &str) -> String {
    const SUFFIX: &str = " (copy)";
    let keep = MAX_TITLE_CHARS - SUFFIX.chars().count();
    format!(
        "{}{SUFFIX}",
        title.trim().chars().take(keep).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles() {
        assert_eq!(title("  Meetup "), Some("Meetup"));
        assert_eq!(title("   "), None);
        assert_eq!(title(&"x".repeat(MAX_TITLE_CHARS + 1)), None);
    }

    #[test]
    fn a_copy_title_is_a_title() {
        let long = "x".repeat(MAX_TITLE_CHARS);
        assert!(title(&copy_title(&long)).is_some());
        assert_eq!(copy_title("Meetup"), "Meetup (copy)");
    }

    #[test]
    fn quantities() {
        assert!(!prize_qty(0));
        assert!(prize_qty(1));
        assert!(prize_qty(MAX_PRIZE_QTY));
        assert!(!prize_qty(MAX_PRIZE_QTY + 1));
    }
}
