//! The Add people dialog's list tab: a list pasted or uploaded, read in the
//! browser ([`crate::import`]), mapped, previewed and added in one go.

use std::{cell::RefCell, rc::Rc};

use dioxus_icons::lucide::{FileUp, ListChecks, TriangleAlert, Upload, X};
use rafi_api::{AddRequest, NameFormat, row_error_text};
use rafi_lifecycle::entrant::{AddRefusal, MAX_ADD_ROWS, Origin};

use super::*;
use crate::import::{
    self, Attempt, Decisions, Existing, MAX_INPUT_BYTES, Mapping, MatchedBy, NameColumns, Preview,
    PreviewError, Table,
};

/// How many rows of each kind the preview lists; the rest are counted.
const SHOWN: usize = 50;
/// How many of the rows to add the preview shows.
const SAMPLE: usize = 10;

/// What the list tab says instead of itself when the raffle doesn't take a
/// list now, as the server said ([`AdminActions::add_list_refusal`](rafi_api::AdminActions::add_list_refusal)).
pub(super) fn list_refusal_text(refusal: Option<AddRefusal>) -> &'static str {
    match refusal {
        Some(AddRefusal::Unpublished) => {
            "Publish the raffle first: the paragraph you send the people on the list links its rules and privacy notice, which are online once it's published. Until then, add people one by one."
        }
        Some(AddRefusal::TestMode) => {
            "Not in test mode: a rehearsal's draws are public, and real people from a list would be drawn in it. Rehearse with fake entrants, then switch test mode off in the settings to add your list."
        }
        Some(AddRefusal::ErasureDatePassed) => {
            "The erasure date has passed: the raffle's entrants are being deleted."
        }
        Some(AddRefusal::Frozen) | None => "The entrant list is frozen.",
    }
}

/// A preview, or why there is none.
type Made = Result<Rc<Preview>, PreviewError>;

/// What a preview is made of.
#[derive(PartialEq)]
struct PreviewKey {
    table: Rc<Table>,
    mapping: Mapping,
    format: NameFormat,
    decisions: Decisions,
    existing: Existing,
}

/// How a name format reads on its button.
fn format_label(format: NameFormat) -> &'static str {
    match format {
        NameFormat::Full => "Full name",
        NameFormat::FirstInitial => "First name and initial",
    }
}

/// "1 person", "3 people".
fn people(n: usize) -> String {
    match n {
        1 => "1 person".to_owned(),
        n => format!("{n} people"),
    }
}

/// The list tab: paste (the main path) or pick a file, check the mapping
/// and the name format, decide on the rows that need it, and add. See
/// [`crate::import`] for how a list is read and how retries keep their
/// batch ID.
#[component]
pub(super) fn AddList(
    admin: AdminData,
    /// After an add, or a refusal that changes the entrants to match.
    on_added: EventHandler<()>,
    /// The list went in: the dialog closes.
    on_done: EventHandler<()>,
) -> Element {
    // The list, read: in memory only.
    let mut table = use_signal(|| None::<Result<Rc<Table>, String>>);
    let mut file = use_signal(|| None::<String>);
    let mut reading = use_signal(|| false);
    // Each new input supersedes a read still under way.
    let mut generation = use_signal(|| 0u64);
    // Remounts the (uncontrolled) textarea, to clear it.
    let mut cleared = use_signal(|| 0u64);
    let mut mapping = use_signal(|| None::<Mapping>);
    let mut format = use_signal(NameFormat::default);
    let mut decisions = use_signal(Decisions::default);
    let mut attempt = use_signal(|| None::<Attempt>);
    // What the server refused, by the line of the list.
    let mut problems = use_signal(Vec::<(usize, String)>::new);
    let mut message = use_signal(|| None::<String>);
    let mut sending = use_signal(|| false);
    let toast = use_toast();

    // Reads `text` after a moment (so a paste shows the spinner, and typing
    // doesn't read every keystroke), unless something newer came meanwhile.
    let load = move |text: String| {
        let (mut generation, mut reading, mut table, mut mapping) =
            (generation, reading, table, mapping);
        let (mut decisions, mut problems, mut message) = (decisions, problems, message);
        let this = generation() + 1;
        generation.set(this);
        reading.set(true);
        spawn(async move {
            gloo_timers::future::TimeoutFuture::new(150).await;
            if generation() != this {
                return;
            }
            let read = import::parse(&text).map(Rc::new).map_err(|e| e.to_string());
            if generation() != this {
                return;
            }
            mapping.set(read.as_ref().ok().map(|t| import::detect(t)));
            table.set(if text.trim().is_empty() {
                None
            } else {
                Some(read)
            });
            decisions.set(Decisions::default());
            problems.set(Vec::new());
            message.set(None);
            reading.set(false);
        });
    };
    let clear = move |_| {
        generation += 1;
        cleared += 1;
        table.set(None);
        file.set(None);
        mapping.set(None);
        decisions.set(Decisions::default());
        problems.set(Vec::new());
        message.set(None);
        reading.set(false);
    };
    let pick = move |e: FormEvent| {
        let Some(picked) = e.files().into_iter().next() else {
            return;
        };
        file.set(Some(picked.name()));
        let this = generation() + 1;
        generation.set(this);
        // Not read at all if it's too large.
        if picked.size() > MAX_INPUT_BYTES as u64 {
            table.set(Some(Err(import::ParseError::TooLarge.to_string())));
            mapping.set(None);
            reading.set(false);
            return;
        }
        reading.set(true);
        spawn(async move {
            let text = match picked.read_bytes().await {
                Ok(bytes) => import::decode(&bytes).map_err(|e| e.to_string()),
                Err(e) => Err(format!("The file couldn't be read: {e}")),
            };
            if generation() != this {
                return;
            }
            match text {
                Ok(text) => load(text),
                Err(e) => {
                    table.set(Some(Err(e)));
                    mapping.set(None);
                    reading.set(false);
                }
            }
        });
    };

    // An add sent and not known to have gone in is left out of the
    // preview, so it stays what was sent and a retry sends it again with
    // its batch ID.
    let pending = attempt().map(|a| a.batch_id);
    let existing = Existing::new(
        admin
            .entrants
            .iter()
            .filter(|e| pending.is_none() || e.batch_id != pending),
    );
    let shown_table = table();
    // Made again only when what it depends on changed: a long list takes a
    // moment, and the dialog renders on every click.
    let cache = use_hook(|| Rc::new(RefCell::new(None::<(PreviewKey, Made)>)));
    let preview: Option<Made> = match (&shown_table, mapping()) {
        (Some(Ok(t)), Some(mapping)) => {
            let key = PreviewKey {
                table: t.clone(),
                mapping,
                format: format(),
                decisions: decisions(),
                existing,
            };
            let mut cache = cache.borrow_mut();
            match &*cache {
                Some((k, made)) if *k == key => Some(made.clone()),
                _ => {
                    let made = import::preview(
                        &key.table,
                        &key.mapping,
                        key.format,
                        &key.existing,
                        &key.decisions,
                    )
                    .map(Rc::new);
                    *cache = Some((key, made.clone()));
                    Some(made)
                }
            }
        }
        _ => None,
    };
    let ready = matches!(&preview, Some(Ok(p)) if !p.to_add.is_empty() && !p.too_many());
    let to_send = preview.clone();

    let entrants = admin.entrants.clone();
    let raffle_id = admin.id.clone();
    let send = move |_| {
        let Some(Ok(p)) = &to_send else {
            return;
        };
        let rows = p.rows();
        if rows.is_empty() || p.too_many() {
            return;
        }
        let batch_id = import::batch_id(attempt().as_ref(), &rows, command_id);
        // The rows changed since an add whose reply was lost: if that add
        // went in after all, the organizer sees it before adding more.
        if let Some(earlier) = attempt()
            && earlier.batch_id != batch_id
            && entrants
                .iter()
                .any(|e| e.batch_id.as_deref() == Some(&earlier.batch_id))
        {
            attempt.set(None);
            message.set(Some(
                "Your earlier add went in after all: the preview now shows its people as already in. Check it, and add again if anyone is left.".to_owned(),
            ));
            return;
        }
        attempt.set(Some(Attempt {
            batch_id: batch_id.clone(),
            rows: rows.clone(),
        }));
        let lines: Vec<usize> = p.to_add.iter().map(|r| r.line).collect();
        let already = p.already_in.len();
        let add = AddRequest {
            batch_id,
            origin: Origin::List,
            rows,
            name_format: Some(format()),
        };
        let raffle_id = raffle_id.clone();
        spawn(async move {
            sending.set(true);
            problems.set(Vec::new());
            message.set(None);
            let result = api::add_entrants(raffle_id, add).await;
            sending.set(false);
            match result {
                Ok(result) => {
                    attempt.set(None);
                    let skipped = already + result.skipped.len();
                    toast.success(
                        format!(
                            "🙋 Added {} ({skipped} already in)",
                            people(result.added.len())
                        ),
                        ToastOptions::new(),
                    );
                    on_added.call(());
                    on_done.call(());
                }
                // Refused for its rows: nothing went in. The entrants are
                // read again (a joiner may have taken a name meanwhile), and
                // the preview makes its names again with them.
                Err(e) if !e.rows.is_empty() => {
                    attempt.set(None);
                    problems.set(
                        e.rows
                            .iter()
                            .filter_map(|p| Some((*lines.get(p.row as usize)?, p.text())))
                            .collect(),
                    );
                    let rows = match e.rows.len() {
                        1 => "1 row".to_owned(),
                        n => format!("{n} rows"),
                    };
                    message.set(Some(format!(
                        "{rows} can't be added, so nothing was (below, by their line in your list). The preview now has the raffle's entrants as they are: check it, and add again."
                    )));
                    on_added.call(());
                }
                // Perhaps nothing went in, perhaps all of it: adding again
                // sends the same batch ID, which the server recognizes.
                Err(e) => message.set(Some(format!(
                    "{} Adding again is safe: nobody is added twice.",
                    err_text(e)
                ))),
            }
        });
    };

    let labels = match &shown_table {
        Some(Ok(t)) => import::column_labels(t, mapping().is_some_and(|m| m.header)),
        _ => Vec::new(),
    };

    rsx! {
        div { class: "space-y-4",
            p { class: "text-sm text-base-content/70",
                "Paste the rows from your spreadsheet or registration tool's export, or pick a CSV file. The check-in list is best: winners must be at the event. Only the names, emails and IDs of the people added leave this page."
            }
            if let Some(name) = file() {
                div { class: "flex flex-wrap items-center gap-2 rounded-box bg-base-200 px-3 py-2 text-sm",
                    FileUp { class: "size-4 text-primary" }
                    span { class: "flex-1 font-medium break-all", "{name}" }
                    button { class: "btn btn-ghost btn-xs", r#type: "button", onclick: clear, X { class: "size-4" } "Clear" }
                }
            } else {
                textarea {
                    key: "{cleared}",
                    class: "textarea w-full font-mono text-xs",
                    rows: "6",
                    spellcheck: "false",
                    aria_label: "The list, pasted",
                    placeholder: "Name\tEmail\nAnna Kovács\tanna@example.com\n…",
                    oninput: move |e| load(e.value()),
                }
                div { class: "flex flex-wrap items-center gap-3",
                    label { class: "flex items-center gap-2 text-sm",
                        Upload { class: "size-4 text-primary" }
                        "Or pick a file"
                        input {
                            class: "file-input file-input-sm",
                            r#type: "file",
                            accept: ".csv,.tsv,.txt,text/csv,text/tab-separated-values,text/plain",
                            onchange: pick,
                        }
                    }
                    span { class: "text-xs text-base-content/60", "CSV or TSV, UTF-8, up to 5 MB. From Excel or Numbers, copying the cells is easiest." }
                    if table().is_some() {
                        button { class: "btn btn-ghost btn-xs", r#type: "button", onclick: clear, X { class: "size-4" } "Clear" }
                    }
                }
            }
            if reading() {
                div { class: "flex items-center gap-2 text-sm text-base-content/70",
                    span { class: "loading loading-spinner loading-sm" }
                    "Reading the list…"
                }
            }
            match &shown_table {
                Some(Err(e)) => rsx! {
                    div { role: "alert", class: "alert alert-error alert-soft text-sm", TriangleAlert { class: "size-4" } "{e}" }
                },
                Some(Ok(t)) if !reading() => rsx! {
                    if t.mojibake {
                        div { role: "alert", class: "alert alert-warning alert-soft text-sm",
                            TriangleAlert { class: "size-4" }
                            "This looks like text saved in another encoding: names like “KovÃ¡cs” would be shown so. Open the file in your spreadsheet, select the cells, copy, and paste them here."
                        }
                    }
                    if let Some(m) = mapping() {
                        MappingForm { table: t.clone(), mapping: m, labels: labels.clone(), on_change: move |m| { mapping.set(Some(m)); problems.set(Vec::new()); } }
                    }
                    div { class: "flex flex-wrap items-center gap-2",
                        span { class: "text-sm font-medium", "Show names as" }
                        div { class: "join", role: "group", aria_label: "How names are shown",
                            for f in NameFormat::ALL {
                                button {
                                    key: "{f.as_str()}",
                                    r#type: "button",
                                    class: if f == format() { "btn btn-sm join-item btn-active btn-secondary" } else { "btn btn-sm join-item" },
                                    aria_pressed: "{f == format()}",
                                    onclick: move |_| format.set(f),
                                    "{format_label(f)}"
                                }
                            }
                        }
                        span { class: "text-xs text-base-content/60", "like “{format().example()}”" }
                    }
                },
                _ => rsx! {},
            }
            match &preview {
                Some(Err(PreviewError::NoName)) => rsx! {
                    div { role: "alert", class: "alert alert-info alert-soft text-sm", "Which column has the names? Pick it above." }
                },
                Some(Ok(p)) if !reading() => rsx! {
                    PreviewView { preview: p.clone(), decisions, problems: problems() }
                },
                _ => rsx! {},
            }
            if let Some(m) = message() {
                div { role: "alert", class: "alert alert-warning alert-soft text-sm", TriangleAlert { class: "size-4" } "{m}" }
            }
            if let Some(Ok(p)) = &preview {
                div { class: "flex flex-wrap items-center gap-3",
                    Button { color: ButtonColor::Secondary, r#type: "button", disabled: !ready || sending() || reading(), onclick: send,
                        if sending() { span { class: "loading loading-spinner loading-sm" } } else { ListChecks { class: "size-5" } }
                        "Add {people(p.to_add.len())}"
                    }
                }
            }
            p { class: "text-sm text-base-content/70",
                "You're responsible for being allowed to share this list, and for telling the people on it about the draw before it, with the paragraph below."
            }
            ParagraphToSend { admin: admin.clone(), format: Some(format()) }
        }
    }
}

/// The column mapping: which row is the header, where the names, emails
/// and registration IDs are. The detector's guess, for the organizer to
/// correct.
#[component]
fn MappingForm(
    table: Rc<Table>,
    mapping: Mapping,
    labels: Vec<String>,
    on_change: EventHandler<Mapping>,
) -> Element {
    let m = mapping;
    let split = matches!(m.name, NameColumns::Split { .. });
    // A select's value: a column's index, or blank for none.
    let column = |v: String| v.parse::<usize>().ok();
    let header_table = table.clone();
    let rows = table.records.len() - usize::from(m.header);
    rsx! {
        div { class: "space-y-3 rounded-box bg-base-200 p-4",
            div { class: "flex flex-wrap items-center gap-x-6 gap-y-2",
                h3 { class: "flex-1 font-semibold", "Columns" }
                label { class: "flex items-center gap-2 text-sm",
                    Switch { value: Some(m.header), on_change: move |on| on_change.call(import::detect_with_header(&header_table, on)) }
                    "The first row is a header"
                }
                span { class: "text-xs text-base-content/60", "{rows} rows, separated by {table.delimiter.label()}" }
            }
            div { class: "grid gap-3 sm:grid-cols-2",
                div { class: "flex flex-col gap-1",
                    span { class: "text-sm font-medium", "Name" }
                    div { class: "join", role: "group", aria_label: "How the names are written",
                        button {
                            r#type: "button",
                            class: if split { "btn btn-xs join-item" } else { "btn btn-xs join-item btn-active" },
                            aria_pressed: "{!split}",
                            onclick: move |_| on_change.call(Mapping { name: match m.name {
                                NameColumns::Split { given, .. } => NameColumns::Single(given),
                                other => other,
                            }, ..m }),
                            "One column"
                        }
                        button {
                            r#type: "button",
                            class: if split { "btn btn-xs join-item btn-active" } else { "btn btn-xs join-item" },
                            aria_pressed: "{split}",
                            onclick: move |_| on_change.call(Mapping { name: match m.name {
                                NameColumns::Single(c) => NameColumns::Split { given: c, family: c },
                                NameColumns::None => NameColumns::Split { given: 0, family: 0 },
                                other => other,
                            }, ..m }),
                            "First and last name"
                        }
                    }
                    match m.name {
                        NameColumns::Split { given, family } => rsx! {
                            ColumnSelect { label: "First name", labels: labels.clone(), value: Some(given), none: "",
                                on_change: move |v: String| if let Some(c) = column(v) { on_change.call(Mapping { name: NameColumns::Split { given: c, family }, ..m }) } }
                            ColumnSelect { label: "Last name", labels: labels.clone(), value: Some(family), none: "",
                                on_change: move |v: String| if let Some(c) = column(v) { on_change.call(Mapping { name: NameColumns::Split { given, family: c }, ..m }) } }
                        },
                        name => rsx! {
                            ColumnSelect {
                                label: "Name",
                                labels: labels.clone(),
                                value: if let NameColumns::Single(c) = name { Some(c) } else { None },
                                none: "Choose the column…",
                                on_change: move |v: String| on_change.call(Mapping { name: column(v).map_or(NameColumns::None, NameColumns::Single), ..m }),
                            }
                            label { class: "flex items-center gap-2 text-sm",
                                Switch { value: Some(m.family_first), on_change: move |on| on_change.call(Mapping { family_first: on, ..m }) }
                                "Names are written family name first (Kovács Anna → Anna Kovács)"
                            }
                        },
                    }
                }
                div { class: "flex flex-col gap-2",
                    ColumnSelect { label: "Email (optional)", labels: labels.clone(), value: m.email, none: "None",
                        on_change: move |v: String| on_change.call(Mapping { email: column(v), ..m }) }
                    ColumnSelect { label: "Registration ID (optional)", labels: labels.clone(), value: m.list_ref, none: "None",
                        on_change: move |v: String| on_change.call(Mapping { list_ref: column(v), ..m }) }
                    p { class: "text-xs text-base-content/60", "An email or an ID recognizes people when you add an updated list: only the new ones are added." }
                }
            }
        }
    }
}

/// A select of the list's columns.
#[component]
fn ColumnSelect(
    label: String,
    labels: Vec<String>,
    value: Option<usize>,
    /// The choice of no column ("None"), or empty for no such choice.
    none: &'static str,
    on_change: EventHandler<String>,
) -> Element {
    rsx! {
        label { class: "flex flex-col gap-1 text-xs text-base-content/70",
            "{label}"
            select {
                class: "select select-sm w-full",
                value: value.map_or(String::new(), |i| i.to_string()),
                onchange: move |e| on_change.call(e.value()),
                if !none.is_empty() {
                    option { value: "", selected: value.is_none(), "{none}" }
                }
                for (i , l) in labels.iter().enumerate() {
                    option { key: "{i}", value: "{i}", selected: value == Some(i), "{l}" }
                }
            }
        }
    }
}

/// What adding the list would do: the counts, the rows that need a
/// decision or can't be added, and the first rows to be added.
#[component]
fn PreviewView(
    preview: Rc<Preview>,
    decisions: Signal<Decisions>,
    /// The server's refusal of the last add, by line.
    problems: Vec<(usize, String)>,
) -> Element {
    let p = preview;
    let joined: Vec<_> = p
        .already_in
        .iter()
        .filter(|a| a.joined_as.is_some())
        .collect();
    rsx! {
        div { class: "space-y-3",
            div { class: "flex flex-wrap gap-2", aria_live: "polite",
                span { class: "badge badge-lg badge-success badge-soft", "{p.to_add.len()} to add" }
                span { class: "badge badge-lg badge-ghost", "{p.already_in.len()} already in" }
                span { class: if p.to_check() > 0 { "badge badge-lg badge-warning badge-soft" } else { "badge badge-lg badge-ghost" }, "{p.to_check()} to check" }
            }
            if p.too_many() {
                div { role: "alert", class: "alert alert-error alert-soft text-sm",
                    TriangleAlert { class: "size-4" }
                    "That's more than {MAX_ADD_ROWS} people to add at once. Add the list in parts of up to {MAX_ADD_ROWS} rows: the people already in are skipped the next time."
                }
            }
            if !problems.is_empty() {
                div { class: "rounded-box bg-error/10 p-3 text-sm",
                    p { class: "font-semibold", "Refused by the server" }
                    ul { class: "list-disc pl-5",
                        for (line , text) in problems.iter().take(SHOWN) {
                            li { key: "{line}", "Line {line}: {text}" }
                        }
                    }
                }
            }
            if p.unkeyed > 0 {
                p { class: "text-sm text-warning",
                    "Without emails or IDs, adding this list again may add people twice "
                    if p.unkeyed == 1 { "(1 row has neither)." } else { "({p.unkeyed} rows have neither)." }
                }
            }
            if !p.duplicates.is_empty() {
                details { class: "text-sm",
                    summary { class: "cursor-pointer",
                        if p.duplicates.len() == 1 { "1 row repeats an earlier one and is left out" } else { "{p.duplicates.len()} rows repeat an earlier one and are left out" }
                    }
                    ul { class: "list-disc pl-5 text-base-content/70",
                        for d in p.duplicates.iter().take(SHOWN) {
                            li { key: "{d.line}",
                                "Line {d.line}: "
                                bdi { "{d.name}" }
                                match d.by {
                                    MatchedBy::Email => rsx! { ", the same person as line {d.of_line}" },
                                    MatchedBy::ListRef => rsx! { ", the same registration ID as line {d.of_line}" },
                                }
                            }
                        }
                        if let Some(more) = more(p.duplicates.len()) { li { class: "list-none", "{more}" } }
                    }
                }
            }
            for g in p.groups.iter().take(SHOWN) {
                div { key: "{g.key}", class: "space-y-2 rounded-box border border-base-300 p-3 text-sm",
                    div { class: "flex flex-wrap items-center gap-2",
                        span { class: "flex-1",
                            "Same email, different names ({g.members.len()}): "
                            span { class: "font-mono text-xs break-all", "{g.email}" }
                        }
                        div { class: "join", role: "group", aria_label: "Who of the group is added",
                            for (first_only , label) in [(false, "Keep all"), (true, "Keep first only")] {
                                button {
                                    key: "{label}",
                                    r#type: "button",
                                    class: if g.first_only == first_only { "btn btn-xs join-item btn-active" } else { "btn btn-xs join-item" },
                                    aria_pressed: "{g.first_only == first_only}",
                                    onclick: {
                                        let key = g.key.clone();
                                        move |_| {
                                            let mut decisions = decisions;
                                            let mut d = decisions.write();
                                            if first_only { d.first_only.insert(key.clone()); } else { d.first_only.remove(&key); }
                                        }
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                    p { class: "text-base-content/70",
                        for (i , (line , name)) in g.members.iter().enumerate() {
                            span { key: "{line}", if i > 0 { ", " } bdi { "{name}" } }
                        }
                    }
                    p { class: "text-xs text-base-content/60", "A group order? Keep all: each is in the draw, the email goes with the first. One person under several names? Keep the first only." }
                }
            }
            if !p.blocked.is_empty() {
                div { class: "space-y-1 rounded-box border border-base-300 p-3 text-sm",
                    p { class: "font-semibold", "Names the blocklist refuses" }
                    p { class: "text-xs text-base-content/60", "It matches parts of words, so it refuses some real names. Approve the ones that are; the rest are left out." }
                    for b in p.blocked.iter().take(SHOWN) {
                        label { key: "{b.line}", class: "flex items-center gap-2",
                            Switch {
                                value: Some(b.approved),
                                on_change: {
                                    let line = b.line;
                                    move |on: bool| {
                                        let mut decisions = decisions;
                                        let mut d = decisions.write();
                                        if on { d.approved.insert(line); } else { d.approved.remove(&line); }
                                    }
                                },
                            }
                            span { "Line {b.line}: " bdi { "{b.name}" } " is their real name" }
                        }
                    }
                    if let Some(more) = more(p.blocked.len()) { p { class: "text-base-content/60", "{more}" } }
                }
            }
            if !p.invalid.is_empty() {
                div { class: "space-y-1 rounded-box border border-base-300 p-3 text-sm",
                    p { class: "font-semibold", "Left out: fix these in the list to add them" }
                    ul { class: "list-disc pl-5 text-base-content/70",
                        for i in p.invalid.iter().take(SHOWN) {
                            li { key: "{i.line}", "Line {i.line}: " bdi { "{import::shorten(&i.name, 40)}" } ": {row_error_text(i.problem)}" }
                        }
                        if let Some(more) = more(p.invalid.len()) { li { class: "list-none", "{more}" } }
                    }
                }
            }
            if !joined.is_empty() {
                div { class: "space-y-1 rounded-box border border-base-300 p-3 text-sm",
                    p { class: "font-semibold", "Already in under another name" }
                    p { class: "text-xs text-base-content/60", "They joined with their phone using the email on your list. If that isn't them, someone used their email first: remove that entry in People, then add the list again." }
                    ul { class: "list-disc pl-5 text-base-content/70",
                        for a in joined.iter().take(SHOWN) {
                            li { key: "{a.line}", "Line {a.line}: " bdi { "{a.name}" } ", joined by phone as " bdi { "{a.joined_as.clone().unwrap_or_default()}" } }
                        }
                        if let Some(more) = more(joined.len()) { li { class: "list-none", "{more}" } }
                    }
                }
            }
            if !p.to_add.is_empty() {
                p { class: "text-sm text-base-content/70",
                    if p.to_add.len() > SAMPLE { "The first {SAMPLE} to be added:" } else { "To be added:" }
                }
                div { class: "overflow-x-auto",
                    table { class: "table table-sm",
                        thead { tr { th { "Line" } th { "Shown as" } th { "Email" } th { "ID" } } }
                        tbody {
                            for r in p.to_add.iter().take(SAMPLE) {
                                tr { key: "{r.line}",
                                    td { class: "text-base-content/60", "{r.line}" }
                                    td { class: "font-semibold wrap-break-word", bdi { "{r.name}" } }
                                    td { class: "text-base-content/70 break-all", "{r.email.clone().unwrap_or_default()}" }
                                    td { class: "text-base-content/70 break-all", "{r.list_ref.clone().unwrap_or_default()}" }
                                }
                            }
                        }
                    }
                }
            } else if p.already_in.is_empty() {
                p { class: "text-sm text-base-content/60", "Nobody to add." }
            } else {
                p { class: "text-sm text-base-content/60", "Everyone on the list is in already." }
            }
        }
    }
}

/// "…and 12 more", after the first [`SHOWN`] of `total`.
fn more(total: usize) -> Option<String> {
    (total > SHOWN).then(|| format!("…and {} more", total - SHOWN))
}

/// The list tab's static state, server-rendered natively: what it shows
/// before a list, and a preview's counts and decisions.
#[cfg(test)]
mod tests {
    use crate::components::toast::ToastProvider;

    use super::*;

    fn admin() -> AdminData {
        serde_json::from_value(serde_json::json!({
            "id": "r1",
            "settings": {
                "title": "Meetup", "description": "", "organizer_name": "Acme",
                "contact_email": "hello@acme.example", "rules": "", "event_date": "2026-11-05",
                "test_mode": false
            },
            "prizes": [],
            "entrants": [],
            "view": {
                "v": 1, "follow": { "kind": "poll", "every_ms": 2500 }, "raffle_id": "r1",
                "slug": "meetup", "title": "Meetup", "description": "", "state": "ready",
                "join_code": "ABCD", "join_url": "https://rafi.example/j/ABCD", "entrant_count": 0,
                "recent_names": [], "prizes": [], "items": [], "current": null, "draws": [],
                "last_failure": null, "manifest_hash": null, "server_now_ms": 0,
                "test_mode": false, "verifiable": true
            },
            "public_url": "https://rafi.example",
            "actions": { "start": null, "draw": false, "export": false, "purge": false, "add_list": true }
        }))
        .unwrap()
    }

    #[test]
    fn before_a_list() {
        #[component]
        fn Tab() -> Element {
            rsx! {
                ToastProvider {
                    AddList { admin: admin(), on_added: |_| {}, on_done: |_| {} }
                }
            }
        }
        let html = dioxus_ssr::render_element(rsx! { Tab {} });
        for expected in [
            "Or pick a file",
            r#"accept=".csv,.tsv,.txt,"#,
            "You&#39;re responsible for being allowed to share this list, and for telling the people on it about the draw before it, with the paragraph below.",
            "What to tell the people you add",
            "https://rafi.example/rules/meetup",
        ] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
        assert!(!html.contains("to add"), "no preview yet: {html}");
    }

    #[component]
    fn PreviewOf(text: String) -> Element {
        let decisions = use_signal(Decisions::default);
        let t = import::parse(&text).unwrap();
        let mapping = Mapping {
            list_ref: Some(2),
            ..import::detect(&t)
        };
        let p = import::preview(
            &t,
            &mapping,
            NameFormat::Full,
            &Existing::default(),
            &decisions(),
        )
        .unwrap();
        rsx! { PreviewView { preview: Rc::new(p), decisions, problems: vec![(4, "someone is already shown as “Anna”".to_owned())] } }
    }

    #[test]
    fn a_preview() {
        let text = std::str::from_utf8(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/import/group-order.csv"
        )))
        .unwrap()
        .to_owned();
        let html = dioxus_ssr::render_element(rsx! { PreviewOf { text } });
        for expected in [
            "4 to add",
            "0 already in",
            "5 to check",
            "Same email, different names (3)",
            "Keep all",
            "Keep first only",
            "Ernő Buzinkay",
            "is their real name",
            "Line 9: ",
            ": not an email address",
            "Line 4: someone is already shown as “Anna”",
            "2 rows repeat an earlier one and are left out",
            "Line 7: <bdi>Dóra Molnár (again)</bdi>, the same registration ID as line 6",
            "To be added:",
        ] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
    }

    #[test]
    fn refusals_say_why() {
        assert!(
            list_refusal_text(Some(AddRefusal::Unpublished))
                .starts_with("Publish the raffle first")
        );
        assert!(list_refusal_text(Some(AddRefusal::TestMode)).starts_with("Not in test mode"));
    }
}
