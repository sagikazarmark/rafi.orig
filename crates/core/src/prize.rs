//! A raffle's prizes as the drawing sees them: one item per copy.

/// The drawing's item ID for the `n`th copy (from 1) of prize `prize_id`.
pub fn item_id(prize_id: &str, n: u64) -> String {
    format!("{prize_id}-{n}")
}

/// The prize an item ID was made from by [`item_id`].
pub fn prize_id(item_id: &str) -> &str {
    item_id
        .rsplit_once('-')
        .map_or(item_id, |(prize_id, _)| prize_id)
}

/// Expands prizes `(prize_id, name, quantity)` into the drawing's items
/// `(item_id, name)`: one per copy, in order.
pub fn items<'a>(
    prizes: impl IntoIterator<Item = (&'a str, &'a str, u64)>,
) -> Vec<(String, String)> {
    prizes
        .into_iter()
        .flat_map(|(prize_id, name, qty)| {
            (1..=qty).map(move |n| (item_id(prize_id, n), name.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prizes_expand_into_items_in_order() {
        let items = items([("a", "Book", 2), ("b", "Mug", 0), ("c", "Pen", 1)]);
        let ids: Vec<_> = items.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["a-1", "a-2", "c-1"]);
        assert_eq!(items[2].1, "Pen");
    }

    #[test]
    fn an_item_knows_its_prize() {
        assert_eq!(prize_id(&item_id("0f3a", 12)), "0f3a");
        assert_eq!(prize_id(&item_id("with-dash", 1)), "with-dash");
    }
}
