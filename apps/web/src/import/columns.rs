//! Which column is which: the header row, and the name, email and
//! registration ID columns, found by their headers in the exports of the
//! common registration tools and in Hungarian, or by what the cells hold.

use rafi_lifecycle::entrant::Email;

use super::parse::Table;

/// Where a row's name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameColumns {
    /// Not chosen: the list can't be added until it is.
    None,
    /// One column with the whole name.
    Single(usize),
    /// First (given) and last (family) name columns.
    Split { given: usize, family: usize },
}

/// How the list's columns map to what an entrant is: the detector's guess
/// ([`detect`]), which the organizer corrects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub name: NameColumns,
    pub email: Option<usize>,
    /// The registration ID (the source system's ID for the row).
    pub list_ref: Option<usize>,
    /// A single name column writes the family name first ("Kovács Anna").
    pub family_first: bool,
    /// The first row is a header, not a person.
    pub header: bool,
}

/// What a header names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Name,
    Given,
    Family,
    Email,
    Ref,
}

/// A header's role and rank (lower wins: the attendee's or ticket's
/// columns beat the order's), and whether it is a Hungarian full name
/// column, where the family name comes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Known {
    role: Role,
    rank: u8,
    hungarian: bool,
}

/// Headers, folded ([`fold`]), by role and rank.
const ALIASES: &[(Role, u8, &[&str])] = &[
    (
        Role::Name,
        0,
        &[
            "name",
            "full name",
            "display name",
            "attendee name",
            "attendee full name",
            "ticket full name",
            "ticket holder name",
            "participant name",
            "guest name",
            "your name",
        ],
    ),
    (
        Role::Name,
        1,
        &["order name", "buyer name", "purchaser name", "billing name"],
    ),
    (
        Role::Given,
        0,
        &[
            "first name",
            "firstname",
            "given name",
            "forename",
            "ticket first name",
            "attendee first name",
            "attendee given name",
            "attendee name given name",
            "keresztnev",
            "utonev",
            "vorname",
            "prenom",
        ],
    ),
    (
        Role::Given,
        1,
        &["order first name", "buyer first name", "billing first name"],
    ),
    (
        Role::Family,
        0,
        &[
            "last name",
            "lastname",
            "surname",
            "family name",
            "ticket last name",
            "attendee last name",
            "attendee surname",
            "attendee family name",
            "attendee name family name",
            "vezeteknev",
            "csaladnev",
            "nachname",
            "nom de famille",
        ],
    ),
    (
        Role::Family,
        1,
        &["order last name", "buyer last name", "billing last name"],
    ),
    (
        Role::Email,
        0,
        &[
            "attendee email",
            "attendee e mail",
            "ticket email",
            "ticket e mail",
            "guest email",
            "participant email",
        ],
    ),
    (
        Role::Email,
        1,
        &[
            "email",
            "e mail",
            "mail",
            "email address",
            "e mail address",
            "e mail cim",
            "email cim",
            "e mail cime",
            "email cime",
        ],
    ),
    (
        Role::Email,
        2,
        &[
            "order email",
            "order e mail",
            "buyer email",
            "purchaser email",
            "billing email",
            "customer email",
        ],
    ),
    (
        Role::Ref,
        0,
        &[
            "registration id",
            "registration number",
            "api id",
            "attendee #",
            "attendee id",
            "attendee number",
            "ticket id",
            "ticket #",
            "ticket number",
            "ticket reference",
            "guest id",
            "participant id",
            "member id",
            "user id",
            "id",
            "azonosito",
        ],
    ),
    (
        Role::Ref,
        1,
        &[
            "order #",
            "order id",
            "order number",
            "order code",
            "order reference",
        ],
    ),
];

/// Hungarian full name headers: the family name comes first.
const HUNGARIAN_NAMES: &[&str] = &["nev", "teljes nev", "neved", "teljes neved"];

/// `header` for matching: accents dropped, lowercase, anything but letters,
/// digits and `#` a single space ("E-mail cím" is "e mail cim", "api_id"
/// is "api id", "Order #" is "order #").
pub fn fold(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    for c in rafi_lifecycle::names::fold_accents(header).chars() {
        if c.is_alphanumeric() || c == '#' {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

fn known(header: &str) -> Option<Known> {
    let h = fold(header);
    if HUNGARIAN_NAMES.contains(&h.as_str()) {
        return Some(Known {
            role: Role::Name,
            rank: 0,
            hungarian: true,
        });
    }
    ALIASES
        .iter()
        .find(|(_, _, names)| names.contains(&h.as_str()))
        .map(|&(role, rank, _)| Known {
            role,
            rank,
            hungarian: false,
        })
}

/// Whether `cell` is an email address ([`Email::parse`]).
pub fn email_shaped(cell: &str) -> bool {
    !cell.is_empty() && Email::parse(cell).is_some()
}

/// Whether the table's first row is a header: some cell is a header Rafi
/// knows, or it has no email while later rows do.
pub fn has_header(table: &Table) -> bool {
    let Some((first, rest)) = table.records.split_first() else {
        return false;
    };
    if first.cells.iter().any(|c| known(c).is_some()) {
        return true;
    }
    !first.cells.iter().any(|c| email_shaped(c))
        && rest
            .iter()
            .take(50)
            .any(|r| r.cells.iter().any(|c| email_shaped(c)))
}

/// The detector's guess at the mapping ([`detect_with_header`]), the header
/// guessed too ([`has_header`]).
pub fn detect(table: &Table) -> Mapping {
    detect_with_header(table, has_header(table))
}

/// The detector's guess at the mapping, with the first row a header or not:
///
/// - by the header, when there is one: the name in one column or in first
///   and last name columns (both, if most rows fill them; else the whole
///   name's column), the email (an attendee's or ticket's beats a plain one,
///   which beats the order's), the registration ID (only a column whose
///   values are unique: an order number shared by a group order isn't one
///   person's);
/// - otherwise the email is the column of mostly email addresses, and,
///   without a header, the name the first column besides it.
///
/// A Hungarian name column ("Név") is taken to write the family name first.
pub fn detect_with_header(table: &Table, header: bool) -> Mapping {
    let data = if header {
        table.records.get(1..).unwrap_or_default()
    } else {
        &table.records[..]
    };
    let filled = |col: usize| data.iter().filter(|r| !r.cell(col).is_empty()).count();
    let mut mapping = Mapping {
        name: NameColumns::None,
        email: None,
        list_ref: None,
        family_first: false,
        header,
    };

    let mut found: Vec<(usize, Known)> = Vec::new();
    if header && let Some(first) = table.records.first() {
        found = first
            .cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| known(c).map(|k| (i, k)))
            .collect();
        // Best rank first; the leftmost of equals.
        found.sort_by_key(|&(i, k)| (k.rank, i));
    }
    let best = |role: Role| found.iter().find(|(_, k)| k.role == role).copied();

    mapping.email = found
        .iter()
        .filter(|(_, k)| k.role == Role::Email)
        .map(|&(i, _)| i)
        .find(|&i| data.iter().any(|r| email_shaped(r.cell(i))))
        .or_else(|| email_by_content(table, data));

    let full = best(Role::Name);
    mapping.name = match (best(Role::Given), best(Role::Family), full) {
        (Some((g, _)), Some((f, _)), full)
            if full.is_none() || 2 * filled(g).min(filled(f)) >= data.len() =>
        {
            NameColumns::Split {
                given: g,
                family: f,
            }
        }
        (_, _, Some((n, _))) => NameColumns::Single(n),
        (Some((g, _)), _, None) => NameColumns::Single(g),
        _ if !header => (0..table.columns)
            .find(|&i| Some(i) != mapping.email && filled(i) > 0)
            .map_or(NameColumns::None, NameColumns::Single),
        _ => NameColumns::None,
    };
    mapping.family_first = matches!(
        (mapping.name, full),
        (NameColumns::Single(n), Some((m, k))) if n == m && k.hungarian
    );

    mapping.list_ref = found
        .iter()
        .filter(|(_, k)| k.role == Role::Ref)
        .map(|&(i, _)| i)
        .find(|&i| {
            let mut seen = std::collections::HashSet::new();
            filled(i) > 0
                && data
                    .iter()
                    .map(|r| r.cell(i))
                    .filter(|v| !v.is_empty())
                    .all(|v| seen.insert(v))
        });
    mapping
}

/// The column whose filled cells are mostly email addresses (at least half
/// of them, of the first 200 rows), the most so.
fn email_by_content(table: &Table, data: &[super::parse::Record]) -> Option<usize> {
    let sample = &data[..data.len().min(200)];
    (0..table.columns)
        .filter_map(|i| {
            let filled = sample.iter().filter(|r| !r.cell(i).is_empty()).count();
            let emails = sample.iter().filter(|r| email_shaped(r.cell(i))).count();
            (emails > 0 && 2 * emails >= filled).then_some((emails, i))
        })
        .max_by_key(|&(emails, i)| (emails, std::cmp::Reverse(i)))
        .map(|(_, i)| i)
}

/// The columns as the mapping's choices name them: the header's cell, or
/// "Column 3" (with its first value, "Column 3: anna@example.com") without
/// a header or for a blank one.
pub fn column_labels(table: &Table, header: bool) -> Vec<String> {
    let first = table.records.first();
    let sample = |i: usize| {
        table
            .records
            .iter()
            .skip(usize::from(header))
            .map(|r| r.cell(i))
            .find(|v| !v.is_empty())
    };
    (0..table.columns)
        .map(|i| {
            let named = first.map(|r| r.cell(i)).filter(|h| header && !h.is_empty());
            match (named, sample(i)) {
                (Some(h), _) => h.to_owned(),
                (None, Some(v)) => format!("Column {}: {}", i + 1, shorten(v, 24)),
                (None, None) => format!("Column {}", i + 1),
            }
        })
        .collect()
}

/// `text`, cut to `max` characters with "…".
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let cut: String = text.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}
