//! The preview: what adding the list would do, row by row, with the
//! organizer's decisions, and the request it sends.

use std::collections::{BTreeSet, HashMap, HashSet};

use rafi_api::{AddRow, AdminEntrant, RemovedAtRequest};
use rafi_lifecycle::{
    entrant::{self, Email, MAX_ADD_ROWS, NameError, RowError, Source},
    names::{NameFormat, NameParts, name_key, unique_names},
};

use super::{
    columns::{Mapping, NameColumns},
    parse::Table,
};

/// Who is in the raffle already, as the admin's entrant list has them: what
/// a row is matched against (email keys and registration IDs), and the
/// display names nobody new may take; and who was removed at their request
/// ([`with_removed`](Self::with_removed)).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Existing {
    /// By email key: the name they are shown as, and how they came in.
    emails: HashMap<String, (String, Source)>,
    list_refs: HashSet<String>,
    names: Vec<String>,
    /// The keys of the people removed at their request.
    removed_emails: HashSet<String>,
    removed_list_refs: HashSet<String>,
}

impl Existing {
    pub fn new<'a>(entrants: impl IntoIterator<Item = &'a AdminEntrant>) -> Self {
        let mut existing = Self::default();
        for e in entrants {
            if let Some(email) = e.email.as_deref().and_then(Email::parse) {
                existing
                    .emails
                    .entry(email.key)
                    .or_insert_with(|| (e.display_name.clone(), e.source));
            }
            existing.list_refs.extend(e.list_ref.clone());
            existing.names.push(e.display_name.clone());
        }
        existing
    }

    /// These entrants, and `removed` as the admin's data has them: the
    /// people removed at their request, whom a row is matched against too.
    pub fn with_removed<'a>(
        mut self,
        removed: impl IntoIterator<Item = &'a RemovedAtRequest>,
    ) -> Self {
        for r in removed {
            self.removed_emails.extend(r.email_key.clone());
            self.removed_list_refs.extend(r.list_ref.clone());
        }
        self
    }
}

/// The organizer's decisions on the rows that need one, by the rows' line
/// numbers and the groups' email keys: they survive a change of mapping or
/// format, and go with a new list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decisions {
    /// Rows whose name the blocklist refuses, approved as real names.
    pub approved: BTreeSet<usize>,
    /// Groups (one email, different names) of which only the first row is
    /// added: "Keep first only". The others keep all.
    pub first_only: BTreeSet<String>,
    /// Rows of people removed at their request, added anyway: the organizer
    /// lifts the removal ("Add anyway").
    pub lifted: BTreeSet<usize>,
}

/// A row to add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRow {
    /// The line the row starts on.
    pub line: usize,
    pub parts: NameParts,
    /// The display name it gets: in the chosen format, unique in the raffle
    /// ([`unique_names`]).
    pub name: String,
    /// As the list has it. Of a group, only the first added row has it.
    pub email: Option<String>,
    pub list_ref: Option<String>,
    /// Its name is in because the organizer approved it.
    pub approved: bool,
    /// It was removed at their request, and the organizer adds it anyway.
    pub lift_removal: bool,
}

/// What matched a row with someone in the raffle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchedBy {
    Email,
    ListRef,
}

/// A row someone in the raffle already is: skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlreadyIn {
    pub line: usize,
    /// The row's name, in the chosen format.
    pub name: String,
    pub by: MatchedBy,
    /// They joined with their phone under another name: someone may have
    /// used the registrant's email first.
    pub joined_as: Option<String>,
}

/// A row someone removed at their request is: left out, unless the
/// organizer adds it anyway ([`Decisions::lifted`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub line: usize,
    /// The row's name, in the chosen format.
    pub name: String,
    pub by: MatchedBy,
    /// Added anyway: the removal is lifted with the add.
    pub lifted: bool,
}

/// A row an earlier row of the list already is: left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duplicate {
    pub line: usize,
    pub name: String,
    /// The earlier row's line.
    pub of_line: usize,
    pub by: MatchedBy,
}

/// Rows with one email and different names: a group order, whose buyer's
/// email is on every ticket. All of them are added unless the organizer
/// keeps the first only; the email goes with the first added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The email key, which [`Decisions::first_only`] names it by.
    pub key: String,
    /// The email, as the first row has it.
    pub email: String,
    /// The rows' lines and names, in the list's order.
    pub members: Vec<(usize, String)>,
    pub first_only: bool,
}

/// A row whose name the blocklist refuses: added only if approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    pub line: usize,
    pub name: String,
    pub approved: bool,
}

/// A row that can't be added: fixed in the list, or left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    pub line: usize,
    /// The row's name as the list has it, or its email if it has none.
    pub name: String,
    pub problem: RowError,
}

/// What adding the list would do ([`preview`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    /// In the list's order.
    pub to_add: Vec<NewRow>,
    pub already_in: Vec<AlreadyIn>,
    /// Removed at their request, added anyway or not.
    pub removed: Vec<Removed>,
    pub duplicates: Vec<Duplicate>,
    pub groups: Vec<Group>,
    pub blocked: Vec<Blocked>,
    pub invalid: Vec<Invalid>,
    /// Rows to add with neither an email nor a registration ID on the
    /// list: nothing matches them on a later add.
    pub unkeyed: usize,
}

impl Preview {
    /// The rows that need the organizer's eye: names to approve, rows that
    /// can't be added, group orders.
    pub fn to_check(&self) -> usize {
        self.blocked.len()
            + self.invalid.len()
            + self.groups.iter().map(|g| g.members.len()).sum::<usize>()
    }

    /// More rows to add than one add takes ([`MAX_ADD_ROWS`]).
    pub fn too_many(&self) -> bool {
        self.to_add.len() > MAX_ADD_ROWS
    }

    /// The rows of the request, in [`to_add`](Self::to_add)'s order: what
    /// leaves the browser, the mapped fields of the rows to add and
    /// nothing else. A server's row problem names its index here.
    pub fn rows(&self) -> Vec<AddRow> {
        self.to_add
            .iter()
            .map(|r| AddRow {
                name: r.name.clone(),
                email: r.email.clone(),
                list_ref: r.list_ref.clone(),
                approved: r.approved,
                lift_removal: r.lift_removal,
            })
            .collect()
    }

    /// The rows removed at their request and left out: not lifted.
    pub fn left_out_at_request(&self) -> usize {
        self.removed.iter().filter(|r| !r.lifted).count()
    }
}

/// Why there is no preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewError {
    /// No name column is chosen.
    NoName,
}

/// A row as the list has it, checked on its own and against the raffle.
struct Candidate {
    line: usize,
    parts: NameParts,
    /// Its display name in the format, before [`unique_names`].
    name: String,
    email: Option<Email>,
    list_ref: Option<String>,
    blocked: bool,
    /// Removed at their request, added anyway.
    lift: bool,
}

/// What adding `table`'s rows, mapped by `mapping`, with names in `format`,
/// to a raffle with `existing` entrants would do, with the organizer's
/// `decisions`. In the list's order, each row is:
///
/// 1. left out silently if what is mapped of it is blank;
/// 2. **already in** if its email (by key) or registration ID is someone's
///    in the raffle: skipped, as the server would;
/// 3. **removed at their request** if its email or registration ID is a
///    removed person's: skipped, as the server would, unless the organizer
///    adds it anyway ([`Decisions::lifted`]), and then checked as any row;
/// 4. **invalid** if its email, registration ID or name can't be added
///    (the shared rules, [`entrant::added_row`]'s), except a name only the
///    blocklist refuses, which is **blocked**: added if approved;
/// 5. a **duplicate** if an earlier row has its registration ID, or its
///    email and the same full name (one person, twice);
/// 6. in a **group** if an earlier row has its email with another name;
/// 7. **to add** otherwise, with a display name made unique among the
///    raffle's and the other new rows' ([`unique_names`]).
pub fn preview(
    table: &Table,
    mapping: &Mapping,
    format: NameFormat,
    existing: &Existing,
    decisions: &Decisions,
) -> Result<Preview, PreviewError> {
    if mapping.name == NameColumns::None {
        return Err(PreviewError::NoName);
    }
    let data = if mapping.header {
        table.records.get(1..).unwrap_or_default()
    } else {
        &table.records[..]
    };
    let mut p = Preview::default();
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut by_ref: HashMap<String, usize> = HashMap::new();
    // Email key → the group's rows, as candidates' indexes.
    let mut by_email: HashMap<String, Vec<usize>> = HashMap::new();
    let mut email_order: Vec<String> = Vec::new();

    for record in data {
        let cell = |i: Option<usize>| i.map_or("", |i| record.cell(i));
        let parts = match mapping.name {
            NameColumns::Single(c) => NameParts::split(record.cell(c), mapping.family_first),
            NameColumns::Split { given, family } => {
                NameParts::new(record.cell(given), record.cell(family))
            }
            NameColumns::None => unreachable!("checked above"),
        };
        let name = parts.display(format);
        let (email_cell, ref_cell) = (cell(mapping.email), cell(mapping.list_ref));
        if name.is_empty() && email_cell.is_empty() && ref_cell.is_empty() {
            continue;
        }
        let email = (!email_cell.is_empty()).then(|| Email::parse(email_cell));
        let list_ref = (!ref_cell.is_empty()).then(|| entrant::list_ref(ref_cell));

        // Already in, by what can be matched, whatever else is wrong.
        if let Some(Some(e)) = &email
            && let Some((shown, source)) = existing.emails.get(&e.key)
        {
            let same = [name_key(&name), name_key(&parts.display(NameFormat::Full))];
            p.already_in.push(AlreadyIn {
                line: record.line,
                name,
                by: MatchedBy::Email,
                joined_as: (*source == Source::SelfService && !same.contains(&name_key(shown)))
                    .then(|| shown.clone()),
            });
            continue;
        }
        if let Some(Some(r)) = &list_ref
            && existing.list_refs.contains(r)
        {
            p.already_in.push(AlreadyIn {
                line: record.line,
                name,
                by: MatchedBy::ListRef,
                joined_as: None,
            });
            continue;
        }

        // Removed at their request, by what can be matched: left out
        // unless added anyway.
        let removed_by = match (&email, &list_ref) {
            (Some(Some(e)), _) if existing.removed_emails.contains(&e.key) => {
                Some(MatchedBy::Email)
            }
            (_, Some(Some(r))) if existing.removed_list_refs.contains(r) => {
                Some(MatchedBy::ListRef)
            }
            _ => None,
        };
        let lift = removed_by.is_some() && decisions.lifted.contains(&record.line);
        if let Some(by) = removed_by {
            p.removed.push(Removed {
                line: record.line,
                name: name.clone(),
                by,
                lifted: lift,
            });
            if !lift {
                continue;
            }
        }

        let invalid = |problem| Invalid {
            line: record.line,
            name: if name.is_empty() {
                email_cell.to_owned()
            } else {
                name.clone()
            },
            problem,
        };
        let email = match email {
            Some(None) => {
                p.invalid.push(invalid(RowError::Email));
                continue;
            }
            Some(Some(e)) => Some(e),
            None => None,
        };
        let list_ref = match list_ref {
            Some(None) => {
                p.invalid.push(invalid(RowError::ListRef));
                continue;
            }
            Some(Some(r)) => Some(r),
            None => None,
        };
        let blocked = match entrant::added_name(&name) {
            Ok(_) => false,
            Err(NameError::Blocked) => true,
            Err(NameError::Invalid) => {
                p.invalid.push(invalid(RowError::Name));
                continue;
            }
        };

        if let Some(r) = &list_ref
            && let Some(&of_line) = by_ref.get(r)
        {
            p.duplicates.push(Duplicate {
                line: record.line,
                name,
                of_line,
                by: MatchedBy::ListRef,
            });
            continue;
        }
        if let Some(e) = &email
            && let Some(group) = by_email.get(&e.key)
        {
            let full = name_key(&parts.display(NameFormat::Full));
            if let Some(&same) = group
                .iter()
                .find(|&&c| name_key(&candidates[c].parts.display(NameFormat::Full)) == full)
            {
                p.duplicates.push(Duplicate {
                    line: record.line,
                    name,
                    of_line: candidates[same].line,
                    by: MatchedBy::Email,
                });
                continue;
            }
        }
        if let Some(r) = &list_ref {
            by_ref.insert(r.clone(), record.line);
        }
        if let Some(e) = &email {
            by_email
                .entry(e.key.clone())
                .or_insert_with(|| {
                    email_order.push(e.key.clone());
                    Vec::new()
                })
                .push(candidates.len());
        }
        candidates.push(Candidate {
            line: record.line,
            parts,
            name,
            email,
            list_ref,
            blocked,
            lift,
        });
    }

    // Which candidates go in: approved if blocked, and of a group kept to
    // its first row, the first only.
    let mut included: Vec<bool> = candidates
        .iter()
        .map(|c| !c.blocked || decisions.approved.contains(&c.line))
        .collect();
    // Of each email, the candidate that carries it.
    let mut carries: Vec<bool> = vec![false; candidates.len()];
    for key in &email_order {
        let members = &by_email[key];
        let first_only = members.len() > 1 && decisions.first_only.contains(key);
        if members.len() > 1 {
            let first = &candidates[members[0]];
            p.groups.push(Group {
                key: key.clone(),
                email: first
                    .email
                    .as_ref()
                    .map(|e| e.raw.clone())
                    .unwrap_or_default(),
                members: members
                    .iter()
                    .map(|&c| (candidates[c].line, candidates[c].name.clone()))
                    .collect(),
                first_only,
            });
        }
        if first_only {
            for &c in &members[1..] {
                included[c] = false;
            }
        }
        if let Some(&c) = members.iter().find(|&&c| included[c]) {
            carries[c] = true;
        }
    }
    for candidate in &candidates {
        if candidate.blocked {
            p.blocked.push(Blocked {
                line: candidate.line,
                name: candidate.name.clone(),
                approved: decisions.approved.contains(&candidate.line),
            });
        }
    }

    let new: Vec<usize> = (0..candidates.len()).filter(|&c| included[c]).collect();
    let parts: Vec<NameParts> = new.iter().map(|&c| candidates[c].parts.clone()).collect();
    let names = unique_names(existing.names.iter().map(String::as_str), &parts, format);
    for (&c, name) in new.iter().zip(names) {
        let candidate = &candidates[c];
        // A name told apart (letters, a number) is checked again, as the
        // server will; the change can't make it invalid or blocked.
        let checked = || {
            if candidate.blocked {
                entrant::added_name_approved(&name)
            } else {
                entrant::added_name(&name)
            }
        };
        if name != candidate.name && checked().is_err() {
            p.invalid.push(Invalid {
                line: candidate.line,
                name,
                problem: RowError::Name,
            });
            continue;
        }
        if candidate.email.is_none() && candidate.list_ref.is_none() {
            p.unkeyed += 1;
        }
        p.to_add.push(NewRow {
            line: candidate.line,
            parts: candidate.parts.clone(),
            name,
            email: candidate
                .email
                .as_ref()
                .filter(|_| carries[c])
                .map(|e| e.raw.clone()),
            list_ref: candidate.list_ref.clone(),
            approved: candidate.blocked,
            lift_removal: candidate.lift,
        });
    }
    p.invalid.sort_by_key(|i| i.line);
    Ok(p)
}
