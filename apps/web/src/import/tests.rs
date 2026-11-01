//! The list tab's reading, mapping and preview, against exports shaped
//! like the common registration tools' (`apps/web/fixtures/import`).

use rafi_api::AdminEntrant;
use rafi_lifecycle::{
    entrant::{MAX_ADD_ROWS, RowError, Source},
    names::{NameFormat, NameParts},
};

use super::{columns::*, parse::*, preview::*, *};

macro_rules! fixture {
    ($name:literal) => {
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/import/",
            $name
        ))
    };
}

fn table(bytes: &[u8]) -> Table {
    parse(&decode(bytes).unwrap()).unwrap()
}

fn cells(t: &Table) -> Vec<Vec<&str>> {
    t.records
        .iter()
        .map(|r| r.cells.iter().map(String::as_str).collect())
        .collect()
}

fn entrant(
    name: &str,
    email: Option<&str>,
    list_ref: Option<&str>,
    source: Source,
) -> AdminEntrant {
    AdminEntrant {
        id: format!("id-{name}"),
        display_name: name.into(),
        email: email.map(Into::into),
        source,
        hidden: false,
        claim_code: None,
        created_at: 0,
        delivered: false,
        deliverable: false,
        held_until: None,
        erased: false,
        outcome: None,
        prize: None,
        batch_id: None,
        list_ref: list_ref.map(Into::into),
        objected: false,
        actions: Default::default(),
    }
}

/// The preview of `bytes` as detected, in `format`, into an empty raffle.
fn previewed(bytes: &[u8], format: NameFormat) -> Preview {
    let t = table(bytes);
    preview(
        &t,
        &detect(&t),
        format,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap()
}

fn names(p: &Preview) -> Vec<&str> {
    p.to_add.iter().map(|r| r.name.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[test]
fn files_must_be_utf8() {
    assert_eq!(decode(b"name\nAnna").unwrap(), "name\nAnna");
    assert_eq!(decode(b"\xEF\xBB\xBFname").unwrap(), "name", "the BOM goes");
    // Windows-1250, as Excel saves a Hungarian CSV: "Kovács" with 0xE1.
    let e = decode(b"name\nKov\xE1cs").unwrap_err();
    assert_eq!(e, ParseError::NotUtf8);
    assert_eq!(
        e.to_string(),
        "This file isn't UTF-8. Open it in your spreadsheet, select the cells, copy, and paste them here."
    );
    // Excel's "Unicode text": UTF-16 with a byte order mark.
    let utf16le: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain("név\tKovács".encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    assert_eq!(decode(&utf16le).unwrap(), "név\tKovács");
    let utf16be: Vec<u8> = [0xFE, 0xFF]
        .into_iter()
        .chain("Ő".encode_utf16().flat_map(u16::to_be_bytes))
        .collect();
    assert_eq!(decode(&utf16be).unwrap(), "Ő");
    assert_eq!(decode(&[0xFF, 0xFE, 0x41]), Err(ParseError::NotUtf8));
}

#[test]
fn limits() {
    let big = vec![b'a'; MAX_INPUT_BYTES + 1];
    assert_eq!(decode(&big), Err(ParseError::TooLarge));
    assert_eq!(
        parse(std::str::from_utf8(&big).unwrap()),
        Err(ParseError::TooLarge)
    );

    let rows = |n: usize| (0..n).map(|i| format!("Person {i}\n")).collect::<String>();
    // A header and MAX_ROWS people.
    assert_eq!(
        parse(&rows(MAX_ROWS + 1)).unwrap().records.len(),
        MAX_ROWS + 1
    );
    assert_eq!(parse(&rows(MAX_ROWS + 2)), Err(ParseError::TooManyRows));

    let wide = |n: usize| format!("Anna\n{}", vec!["x"; n].join("\t"));
    assert!(parse(&wide(MAX_COLUMNS)).is_ok());
    assert_eq!(
        parse(&wide(MAX_COLUMNS + 1)),
        Err(ParseError::TooManyColumns { line: 2 })
    );

    let long = |n: usize| format!("name,notes\nAnna,{}\n", "x".repeat(n));
    assert!(parse(&long(MAX_FIELD_BYTES)).is_ok());
    assert_eq!(
        parse(&long(MAX_FIELD_BYTES + 1)),
        Err(ParseError::FieldTooLong { line: 2 })
    );
    let quoted = format!("name,notes\nAnna,\"{}\"\n", "x".repeat(MAX_FIELD_BYTES + 1));
    assert_eq!(parse(&quoted), Err(ParseError::FieldTooLong { line: 2 }));

    assert_eq!(parse(""), Err(ParseError::Empty));
    assert_eq!(parse("\n \n\t\n"), Err(ParseError::Empty));
    for e in [
        ParseError::TooLarge,
        ParseError::TooManyRows,
        ParseError::TooManyColumns { line: 2 },
        ParseError::FieldTooLong { line: 2 },
        ParseError::UnclosedQuote { line: 2 },
        ParseError::Empty,
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn an_unclosed_quote_is_an_error() {
    let e = parse(&decode(fixture!("unclosed-quote.csv")).unwrap()).unwrap_err();
    assert_eq!(e, ParseError::UnclosedQuote { line: 3 });
    assert!(
        e.to_string().starts_with("A quote (\") opened on line 3"),
        "{e}"
    );
    // One swallowing more than a cell may hold, with no quote after it.
    let mut text = "name,email\n\"Anna,anna@example.com\n".to_owned();
    for i in 0..100 {
        text.push_str(&format!("Person {i},p{i}@example.com\n"));
    }
    assert_eq!(parse(&text), Err(ParseError::UnclosedQuote { line: 2 }));
}

#[test]
fn delimiters_are_sniffed() {
    assert_eq!(sniff_of("a\tb\tc\n1\t2\t3"), Delimiter::Tab);
    assert_eq!(sniff_of("a,b,c\n1,2,3"), Delimiter::Comma);
    assert_eq!(sniff_of("a;b;c\n1;2;3"), Delimiter::Semicolon);
    // Decimal commas don't fool it, nor commas in names of a paste.
    assert_eq!(
        sniff_of("name;price\nAnna;1,5\nBence;2"),
        Delimiter::Semicolon
    );
    assert_eq!(
        sniff_of("name\temail\nKovács, Anna\ta@x.hu\nBence\tb@x.hu"),
        Delimiter::Tab
    );
    // Quoted delimiters don't count.
    assert_eq!(
        sniff_of("\"Kovács, Anna\";a@x.hu\n\"Nagy, Bence\";b@x.hu"),
        Delimiter::Semicolon
    );
    // One column: nothing splits it.
    let one = parse("Anna Kovács\nKiss, Péter\nBence").unwrap();
    assert_eq!(one.delimiter, Delimiter::Tab);
    assert_eq!(one.columns, 1);
    // Excel's sep= line names it, and isn't a row.
    let sep = parse("sep=;\nname;email\nAnna;a@x.hu").unwrap();
    assert_eq!(sep.delimiter, Delimiter::Semicolon);
    assert_eq!(cells(&sep), [["name", "email"], ["Anna", "a@x.hu"]]);
    assert_eq!(sep.records[0].line, 2);
}

fn sniff_of(text: &str) -> Delimiter {
    parse(text).unwrap().delimiter
}

#[test]
fn quoting_follows_rfc_4180_and_forgives() {
    let t = table(fixture!("quoting.csv"));
    assert_eq!(t.delimiter, Delimiter::Comma);
    assert_eq!(
        cells(&t),
        [
            vec!["name", "email", "id"],
            vec!["Kovács, Anna", "anna@example.com", "1"],
            vec!["Nagy \"Bence\" B", "bence@example.com", "2"],
            vec!["Szabó\nZoé", "zoe@example.com", "3"],
            vec!["TóthEszter", "eszter@example.com", "4"],
            vec!["O\"Brien Pat", "pat@example.com", "5"],
        ]
    );
    // Lines as the file numbers them; the blank row (line 6) and the
    // trailing empty lines are left out.
    let lines: Vec<usize> = t.records.iter().map(|r| r.line).collect();
    assert_eq!(lines, [1, 2, 3, 4, 7, 8]);
    // The line break in a name is a space on the screen.
    let p = previewed(fixture!("quoting.csv"), NameFormat::Full);
    assert_eq!(
        names(&p),
        [
            "Kovács, Anna",
            "Nagy \"Bence\" B",
            "Szabó Zoé",
            "TóthEszter",
            "O\"Brien Pat"
        ]
    );
}

#[test]
fn mojibake_is_noticed() {
    assert!(table(fixture!("mojibake.csv")).mojibake);
    for clean in ["luma.csv", "google-forms.csv", "semicolon-bom.csv"] {
        let bytes = std::fs::read(format!(
            "{}/fixtures/import/{clean}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        assert!(!table(&bytes).mojibake, "{clean}");
    }
    assert!(parse("name\nKov\u{FFFD}cs").unwrap().mojibake);
    // Names that only look like it: São, Åse, Âge.
    assert!(!parse("name\nJoão São\nÅse\nÂge Ñandú").unwrap().mojibake);
}

// ---------------------------------------------------------------------------
// Mapping, by export
// ---------------------------------------------------------------------------

#[test]
fn luma() {
    let t = table(fixture!("luma.csv"));
    let m = detect(&t);
    assert_eq!(
        m,
        Mapping {
            name: NameColumns::Split {
                given: 2,
                family: 3
            },
            email: Some(4),
            list_ref: Some(0),
            family_first: false,
            header: true,
        }
    );
    let p = previewed(fixture!("luma.csv"), NameFormat::Full);
    assert_eq!(
        names(&p),
        [
            "Anna Kovács",
            "Bence Nagy",
            "Zoé Szabó",
            "Dávid Tóth",
            "Eszter Kiss"
        ]
    );
    assert_eq!(p.to_add[0].list_ref.as_deref(), Some("gst-a1B2c3D4e5"));
    assert_eq!(p.to_add[3].email.as_deref(), Some("David.Toth@Example.com"));
    assert_eq!(p.to_add[2].line, 4, "the multi-line cell is on line 4–5");
    assert_eq!(p.to_add[3].line, 6);
    assert_eq!(p.unkeyed, 0);
    assert_eq!(p.to_check(), 0);

    // Luma without first and last names (not asked): the whole name.
    let text = "api_id,name,first_name,last_name,email\ngst-1,Anna Kovács,,,anna@x.hu\ngst-2,Bence Nagy,,,bence@x.hu\n";
    let m = detect(&parse(text).unwrap());
    assert_eq!(m.name, NameColumns::Single(1));
}

#[test]
fn eventbrite() {
    let t = table(fixture!("eventbrite.csv"));
    let m = detect(&t);
    assert_eq!(
        m.name,
        NameColumns::Split {
            given: 2,
            family: 3
        }
    );
    assert_eq!(m.email, Some(4));
    // The order number is shared by the group order's tickets: not one
    // person's ID.
    assert_eq!(m.list_ref, None);
    let p = previewed(fixture!("eventbrite.csv"), NameFormat::Full);
    assert_eq!(p.groups.len(), 1);
    assert_eq!(p.groups[0].email, "office@acme.example");
    assert_eq!(
        p.groups[0].members,
        [
            (3, "Bence Nagy".to_owned()),
            (4, "Csilla Varga".to_owned()),
            (5, "Dóra Molnár".to_owned())
        ]
    );
    assert_eq!(names(&p).len(), 5, "a group order gives every name");

    // With an attendee number, that's the registration ID.
    let text = "Order #,Attendee #,First Name,Last Name,Email\n1,11,Anna,Kovács,a@x.hu\n2,21,Bence,Nagy,o@x.hu\n2,22,Csilla,Varga,o@x.hu\n";
    assert_eq!(detect(&parse(text).unwrap()).list_ref, Some(1));
}

#[test]
fn pretix() {
    let t = table(fixture!("pretix.csv"));
    assert_eq!(t.delimiter, Delimiter::Semicolon);
    let m = detect(&t);
    assert_eq!(
        m.name,
        NameColumns::Split {
            given: 7,
            family: 8
        }
    );
    // The attendee's email beats the order's.
    assert_eq!(m.email, Some(9));
    assert_eq!(m.list_ref, None, "order codes repeat");
    let p = previewed(fixture!("pretix.csv"), NameFormat::FirstInitial);
    assert_eq!(names(&p), ["Anna K.", "Bence N.", "Gábor F."]);
    assert!(p.groups.is_empty());
}

#[test]
fn tito() {
    let t = table(fixture!("tito.csv"));
    let m = detect(&t);
    assert_eq!(
        m.name,
        NameColumns::Split {
            given: 4,
            family: 5
        }
    );
    assert_eq!(m.email, Some(6), "the ticket's email, not the order's");
    assert_eq!(m.list_ref, Some(7), "the ticket reference");
    let p = previewed(fixture!("tito.csv"), NameFormat::Full);
    assert_eq!(names(&p), ["Anna Kovács", "Bence Nagy", "Csilla Varga"]);
    assert!(p.groups.is_empty());
}

#[test]
fn meetup() {
    let t = table(fixture!("meetup.csv"));
    let m = detect(&t);
    assert_eq!(m.name, NameColumns::Single(0));
    assert_eq!(m.email, None, "Meetup gives no emails");
    assert_eq!(m.list_ref, Some(1), "the member ID");
    let p = previewed(fixture!("meetup.csv"), NameFormat::Full);
    assert_eq!(names(&p), ["Anna Kovács", "Bence N.", "Csilla"]);
    assert_eq!(p.unkeyed, 0, "the IDs match them on a later add");

    // Without the IDs, nothing would.
    let t = table(fixture!("meetup.csv"));
    let m = Mapping {
        list_ref: None,
        ..detect(&t)
    };
    let p = preview(
        &t,
        &m,
        NameFormat::Full,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap();
    assert_eq!(p.unkeyed, 3);
}

#[test]
fn google_forms_in_hungarian() {
    let t = table(fixture!("google-forms.csv"));
    let m = detect(&t);
    assert_eq!(m.name, NameColumns::Single(2));
    assert_eq!(m.email, Some(1), "E-mail-cím");
    assert!(m.family_first, "a Hungarian name column");
    let p = previewed(fixture!("google-forms.csv"), NameFormat::Full);
    assert_eq!(names(&p), ["Anna Kovács", "Bence Nagy", "Eszter Réka Tóth"]);
    let p = previewed(fixture!("google-forms.csv"), NameFormat::FirstInitial);
    assert_eq!(names(&p), ["Anna K.", "Bence N.", "Eszter Réka T."]);
}

#[test]
fn excel_paste() {
    let t = table(fixture!("excel-paste.tsv"));
    assert_eq!(t.delimiter, Delimiter::Tab);
    assert_eq!(t.records.len(), 4, "the trailing empty lines are left out");
    assert_eq!(t.records[2].cells[3], "Quote \"Ltd\"");
    let m = detect(&t);
    assert_eq!(
        m.name,
        NameColumns::Split {
            given: 1,
            family: 0
        }
    );
    assert_eq!(m.email, Some(2));
    let p = previewed(fixture!("excel-paste.tsv"), NameFormat::Full);
    assert_eq!(names(&p), ["Anna Kovács", "Bence Nagy", "Zoé Szabó"]);
    assert_eq!(p.to_add[2].email, None);
    assert_eq!(p.unkeyed, 1);
}

#[test]
fn semicolons_with_a_bom() {
    let t = table(fixture!("semicolon-bom.csv"));
    assert_eq!(t.delimiter, Delimiter::Semicolon);
    assert_eq!(t.records[0].cells[0], "Name", "no BOM in the header");
    let m = detect(&t);
    assert_eq!(m.name, NameColumns::Single(0));
    assert_eq!(m.email, Some(1));
    let p = previewed(fixture!("semicolon-bom.csv"), NameFormat::Full);
    assert_eq!(names(&p), ["Anna Kovács", "Nagy; Bence", "Csilla Varga"]);
}

#[test]
fn headers_and_contents() {
    // No header: the email by its cells, the name the first other column.
    let t = parse("anna@x.hu\tAnna Kovács\nbence@x.hu\tBence Nagy").unwrap();
    assert_eq!(
        detect(&t),
        Mapping {
            name: NameColumns::Single(1),
            email: Some(0),
            list_ref: None,
            family_first: false,
            header: false,
        }
    );
    // A header Rafi doesn't know, told by its lack of emails.
    let t = parse("Ki vagy?\tHova írjunk?\nAnna\tanna@x.hu\nBence\tbence@x.hu").unwrap();
    let m = detect(&t);
    assert!(m.header);
    assert_eq!(m.email, Some(1));
    assert_eq!(m.name, NameColumns::None, "the organizer picks it");
    assert_eq!(
        preview(
            &t,
            &m,
            NameFormat::Full,
            &Existing::default(),
            &Decisions::default()
        ),
        Err(PreviewError::NoName)
    );
    // Flipped by the organizer: the first row is a person.
    let m = detect_with_header(&t, false);
    assert!(!m.header);
    assert_eq!(m.name, NameColumns::Single(0));
    // Just names.
    let t = parse("Anna Kovács\nBence Nagy").unwrap();
    let m = detect(&t);
    assert!(!m.header);
    assert_eq!(m.name, NameColumns::Single(0));
    // Headers in any case, with accents or not, and aliases.
    for (header, col) in [
        ("NÉV;EMAIL", 0),
        ("Full Name;E-Mail Address", 0),
        ("Attendee name;Attendee Email", 0),
        ("vezeteknev;keresztnev;e-mail cim", 0),
    ] {
        let t = parse(&format!("{header}\nx;y;z")).unwrap();
        assert!(has_header_of(&t), "{header}");
        assert_ne!(detect(&t).name, NameColumns::None, "{header}: {col}");
    }
    assert_eq!(
        column_labels(&table(fixture!("meetup.csv")), true)[..2],
        ["Name", "Member ID"]
    );
    assert_eq!(
        column_labels(&parse("anna@x.hu\tAnna\n\tBence").unwrap(), false),
        ["Column 1: anna@x.hu", "Column 2: Anna"]
    );
}

fn has_header_of(t: &Table) -> bool {
    detect(t).header
}

// ---------------------------------------------------------------------------
// The preview
// ---------------------------------------------------------------------------

#[test]
fn group_orders_duplicates_blocked_and_invalid_rows() {
    let t = table(fixture!("group-order.csv"));
    let m = detect(&t);
    assert_eq!(m.list_ref, None, "T-5 is there twice");
    let m = Mapping {
        list_ref: Some(2),
        ..m
    };
    let mut decisions = Decisions::default();
    let p = preview(&t, &m, NameFormat::Full, &Existing::default(), &decisions).unwrap();

    // A group order: every name, the email on the first only.
    assert_eq!(
        p.groups,
        [Group {
            key: "office@acme.example".into(),
            email: "office@acme.example".into(),
            members: vec![
                (2, "Anna Kovács".into()),
                (3, "Bence Nagy".into()),
                (4, "Csilla Varga".into())
            ],
            first_only: false,
        }]
    );
    // Anna twice with one email is one person; T-5 twice, the first kept.
    assert_eq!(
        p.duplicates,
        [
            Duplicate {
                line: 5,
                name: "Anna Kovács".into(),
                of_line: 2,
                by: MatchedBy::Email
            },
            Duplicate {
                line: 7,
                name: "Dóra Molnár (again)".into(),
                of_line: 6,
                by: MatchedBy::ListRef
            },
        ]
    );
    // A real name the blocklist refuses, not approved: left out.
    assert_eq!(
        p.blocked,
        [Blocked {
            line: 8,
            name: "Ernő Buzinkay".into(),
            approved: false
        }]
    );
    assert_eq!(
        p.invalid,
        [Invalid {
            line: 9,
            name: "Fanni".into(),
            problem: RowError::Email
        }]
    );
    assert_eq!(
        names(&p),
        ["Anna Kovács", "Bence Nagy", "Csilla Varga", "Dóra Molnár"]
    );
    let emails: Vec<Option<&str>> = p.to_add.iter().map(|r| r.email.as_deref()).collect();
    assert_eq!(
        emails,
        [
            Some("office@acme.example"),
            None,
            None,
            Some("dora@example.com")
        ]
    );
    assert_eq!(
        p.to_check(),
        5,
        "the group's three, the blocked, the invalid"
    );

    // Approved, and the group kept to its first.
    decisions.approved.insert(8);
    decisions.first_only.insert("office@acme.example".into());
    let p = preview(&t, &m, NameFormat::Full, &Existing::default(), &decisions).unwrap();
    assert!(p.groups[0].first_only);
    assert!(p.blocked[0].approved);
    assert_eq!(names(&p), ["Anna Kovács", "Dóra Molnár", "Ernő Buzinkay"]);
    let rows = p.rows();
    assert!(rows[2].approved);
    assert_eq!(rows[0].email.as_deref(), Some("office@acme.example"));

    // What is sent passes the server's own checks.
    let inputs = rows.iter().map(|r| rafi_lifecycle::entrant::RowInput {
        name: &r.name,
        email: r.email.as_deref(),
        list_ref: r.list_ref.as_deref(),
        approved: r.approved,
    });
    assert!(rafi_lifecycle::entrant::added_rows(inputs).is_ok());
}

#[test]
fn a_blocked_first_of_a_group_passes_the_email_on() {
    let t = parse("name,email\nErnő Buzinkay,o@x.hu\nAnna Kovács,o@x.hu").unwrap();
    let p = preview(
        &t,
        &detect(&t),
        NameFormat::Full,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap();
    assert_eq!(names(&p), ["Anna Kovács"]);
    assert_eq!(p.to_add[0].email.as_deref(), Some("o@x.hu"));
}

#[test]
fn people_already_in_are_skipped() {
    let existing = Existing::new(&[
        entrant(
            "Batman",
            Some("Anna@Example.com"),
            None,
            Source::SelfService,
        ),
        entrant(
            "Bence Nagy",
            Some("bence@example.org"),
            None,
            Source::SelfService,
        ),
        entrant("Zoé Szabó", None, Some("gst-k1L2m3N4o5"), Source::Manual),
        entrant("Eszter Kiss", None, None, Source::SelfService),
    ]);
    let t = table(fixture!("luma.csv"));
    let m = detect(&t);
    // Luma's emails differ in case from the one who joined.
    let t = Table {
        records: t
            .records
            .into_iter()
            .map(|mut r| {
                if r.cells[4] == "anna.kovacs@example.com" {
                    r.cells[4] = "anna@example.com".into();
                }
                r
            })
            .collect(),
        ..t
    };
    let p = preview(&t, &m, NameFormat::Full, &existing, &Decisions::default()).unwrap();
    assert_eq!(
        p.already_in,
        [
            AlreadyIn {
                line: 2,
                name: "Anna Kovács".into(),
                by: MatchedBy::Email,
                joined_as: Some("Batman".into()),
            },
            AlreadyIn {
                line: 3,
                name: "Bence Nagy".into(),
                by: MatchedBy::Email,
                joined_as: None,
            },
            AlreadyIn {
                line: 4,
                name: "Zoé Szabó".into(),
                by: MatchedBy::ListRef,
                joined_as: None,
            },
        ]
    );
    // Eszter Kiss joined without that email: a namesake, numbered.
    assert_eq!(names(&p), ["Dávid Tóth", "Eszter Kiss (2)"]);
}

#[test]
fn names_are_unique_in_the_raffle_and_the_list() {
    let existing = Existing::new(&[entrant("Anna K.", None, None, Source::SelfService)]);
    let t = parse("first name,last name\nAnna,Kovács\nAnna,Kiss\nAnna,Kovács\nBence,Nagy").unwrap();
    let m = detect(&t);
    let p = preview(
        &t,
        &m,
        NameFormat::FirstInitial,
        &existing,
        &Decisions::default(),
    )
    .unwrap();
    assert_eq!(
        names(&p),
        ["Anna Ko.", "Anna Ki.", "Anna Ko. (2)", "Bence N."]
    );
    assert_eq!(p.unkeyed, 4);
    let p = preview(&t, &m, NameFormat::Full, &existing, &Decisions::default()).unwrap();
    assert_eq!(
        names(&p),
        ["Anna Kovács", "Anna Kiss", "Anna Kovács (2)", "Bence Nagy"]
    );
    // What the server checks: nothing clashes.
    let clashes = rafi_lifecycle::names::name_clashes(
        ["Anna K."],
        p.to_add
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.as_str())),
    );
    assert!(clashes.is_empty(), "{clashes:?}");
    assert_eq!(p.to_add[0].parts, NameParts::new("Anna", "Kovács"));
}

#[test]
fn single_names_split_by_the_order_chosen() {
    let t = parse("name\nKovács Anna\nNagy Bence Péter").unwrap();
    let mut m = detect(&t);
    assert!(!m.family_first);
    let first = |m: &Mapping| {
        let p = preview(
            &t,
            m,
            NameFormat::FirstInitial,
            &Existing::default(),
            &Decisions::default(),
        )
        .unwrap();
        names(&p).into_iter().map(str::to_owned).collect::<Vec<_>>()
    };
    assert_eq!(first(&m), ["Kovács A.", "Nagy Bence P."]);
    m.family_first = true;
    assert_eq!(first(&m), ["Anna K.", "Bence Péter N."]);
}

#[test]
fn invalid_rows_say_why() {
    let long = "A".repeat(65);
    let text = format!(
        "name,email,id\n,a@x.hu,1\n{long},b@x.hu,2\nZero\u{200B}Width,c@x.hu,3\nDóra,d@x.hu,{}\n!!!,e@x.hu,5\n",
        "x".repeat(65)
    );
    let t = parse(&text).unwrap();
    let p = preview(
        &t,
        &detect(&t),
        NameFormat::Full,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap();
    let problems: Vec<(usize, &str, RowError)> = p
        .invalid
        .iter()
        .map(|i| (i.line, i.name.as_str(), i.problem))
        .collect();
    assert_eq!(
        problems,
        [
            (2, "a@x.hu", RowError::Name),
            (3, long.as_str(), RowError::Name),
            (4, "Zero\u{200B}Width", RowError::Name),
            (5, "Dóra", RowError::ListRef),
            (6, "!!!", RowError::Name),
        ]
    );
    assert!(p.to_add.is_empty());
}

#[test]
fn too_many_to_add_at_once() {
    let mut text = "name\n".to_owned();
    for i in 0..=MAX_ADD_ROWS {
        text.push_str(&format!("Person {i}\n"));
    }
    let t = parse(&text).unwrap();
    let p = preview(
        &t,
        &detect(&t),
        NameFormat::Full,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap();
    assert_eq!(p.to_add.len(), MAX_ADD_ROWS + 1);
    assert!(p.too_many());
}

#[test]
fn a_retry_sends_the_same_batch_id() {
    let p = previewed(fixture!("tito.csv"), NameFormat::Full);
    let rows = p.rows();
    let first = batch_id(None, &rows, || "b1".into());
    assert_eq!(first, "b1");
    let sent = Attempt {
        batch_id: first,
        rows: rows.clone(),
    };
    assert_eq!(batch_id(Some(&sent), &rows, || "b2".into()), "b1");
    let other = previewed(fixture!("tito.csv"), NameFormat::FirstInitial).rows();
    assert_eq!(batch_id(Some(&sent), &other, || "b2".into()), "b2");
}

/// The largest list read, previewed: many namesakes, so the names need
/// letters and numbers.
#[test]
fn the_largest_list() {
    let mut text = "first name\tlast name\temail\n".to_owned();
    for i in 0..MAX_ROWS {
        let family = ["Kovács", "Kiss", "Nagy", "Kovacsics"][i % 4];
        text.push_str(&format!("Anna{}\t{family}\tp{i}@example.com\n", i % 50));
    }
    let t = parse(&text).unwrap();
    let p = preview(
        &t,
        &detect(&t),
        NameFormat::FirstInitial,
        &Existing::default(),
        &Decisions::default(),
    )
    .unwrap();
    assert_eq!(p.to_add.len(), MAX_ROWS);
    assert!(p.too_many());
    let clashes = rafi_lifecycle::names::name_clashes(
        [],
        p.to_add
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.as_str())),
    );
    assert!(clashes.is_empty());
}

/// Someone removed at their request is left out by their email key or
/// registration ID, before any other check but "already in", unless the
/// organizer adds them anyway: then the row goes through every check, and
/// is sent lifting the removal.
#[test]
fn people_removed_at_their_request_are_left_out_unless_added_anyway() {
    let removed = [
        rafi_api::RemovedAtRequest {
            email_key: Some("anna@example.com".into()),
            list_ref: None,
            removed_at: 1,
        },
        rafi_api::RemovedAtRequest {
            email_key: None,
            list_ref: Some("T-2".into()),
            removed_at: 2,
        },
    ];
    let existing = Existing::new(&[entrant(
        "Dóra",
        Some("dora@example.com"),
        None,
        Source::SelfService,
    )])
    .with_removed(&removed);
    let t = parse(
        "name,email,id\nAnna Kovács,Anna+x@Example.com,\nBence,,T-2\nCsilla,c@example.com,\nDóra,dora@example.com,T-2",
    )
    .unwrap();
    let m = Mapping {
        list_ref: Some(2),
        ..detect(&t)
    };
    let p = preview(&t, &m, NameFormat::Full, &existing, &Decisions::default()).unwrap();
    assert_eq!(
        p.removed,
        [
            Removed {
                line: 2,
                name: "Anna Kovács".into(),
                by: MatchedBy::Email,
                lifted: false,
            },
            Removed {
                line: 3,
                name: "Bence".into(),
                by: MatchedBy::ListRef,
                lifted: false,
            },
        ],
        "Dóra is in already: that comes first"
    );
    assert_eq!(names(&p), ["Csilla"]);
    assert_eq!(p.left_out_at_request(), 2);
    assert!(p.rows().iter().all(|r| !r.lift_removal));

    let decisions = Decisions {
        lifted: [3].into(),
        ..Decisions::default()
    };
    let p = preview(&t, &m, NameFormat::Full, &existing, &decisions).unwrap();
    assert_eq!(names(&p), ["Bence", "Csilla"]);
    assert_eq!(p.left_out_at_request(), 1);
    let lifts: Vec<bool> = p.rows().iter().map(|r| r.lift_removal).collect();
    assert_eq!(lifts, [true, false], "sent lifting Bence's removal");
}
