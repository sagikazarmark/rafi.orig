//! Text to a table: decoding a file's bytes, sniffing the delimiter, and
//! reading RFC 4180 records, within limits that keep a page from choking
//! on what it was given.

use std::fmt;

/// The most text (pasted) or bytes (a file) read, 5 MB.
pub const MAX_INPUT_BYTES: usize = 5 * 1024 * 1024;
/// The most rows read, besides a header: five parts of
/// [`MAX_ADD_ROWS`](rafi_lifecycle::entrant::MAX_ADD_ROWS).
pub const MAX_ROWS: usize = 10_000;
/// The most columns a row may have.
pub const MAX_COLUMNS: usize = 200;
/// The longest cell, in bytes (1 KB): a name, an email or an ID is far
/// shorter, and a longer one is mostly a quote never closed.
pub const MAX_FIELD_BYTES: usize = 1024;
/// How many lines the delimiter is sniffed from.
const SNIFF_LINES: usize = 20;

/// What separates the cells of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Delimiter {
    /// What a spreadsheet copies: the paste.
    Tab,
    Comma,
    /// Excel's CSV where the decimal separator is a comma (much of Europe).
    Semicolon,
}

impl Delimiter {
    /// In the order a tie goes to.
    pub const ALL: [Self; 3] = [Self::Tab, Self::Semicolon, Self::Comma];

    pub fn char(self) -> char {
        match self {
            Self::Tab => '\t',
            Self::Comma => ',',
            Self::Semicolon => ';',
        }
    }

    /// In the organizer's words.
    pub fn label(self) -> &'static str {
        match self {
            Self::Tab => "tabs",
            Self::Comma => "commas",
            Self::Semicolon => "semicolons",
        }
    }
}

/// Why a list can't be read. [`Display`](fmt::Display) says it to the
/// organizer, with what to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The file's bytes aren't UTF-8 (nor UTF-16 with a byte order mark).
    NotUtf8,
    /// More than [`MAX_INPUT_BYTES`].
    TooLarge,
    /// Nothing but blank lines.
    Empty,
    /// More than [`MAX_ROWS`] rows.
    TooManyRows,
    /// A row with more than [`MAX_COLUMNS`] cells, on its first line.
    TooManyColumns { line: usize },
    /// A cell longer than [`MAX_FIELD_BYTES`], in the row starting on `line`.
    FieldTooLong { line: usize },
    /// A quote opened on `line` and never closed: it would swallow the rest.
    UnclosedQuote { line: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUtf8 => f.write_str(
                "This file isn't UTF-8. Open it in your spreadsheet, select the cells, copy, and paste them here.",
            ),
            Self::TooLarge => write!(
                f,
                "This is more than {} MB. Copy only the columns you need (the names, the emails, the IDs) and paste them here.",
                MAX_INPUT_BYTES / (1024 * 1024)
            ),
            Self::Empty => f.write_str("There is nothing to read here."),
            Self::TooManyRows => write!(
                f,
                "This list has more than {MAX_ROWS} rows. Add it in parts: the people already in are skipped the next time."
            ),
            Self::TooManyColumns { line } => write!(
                f,
                "Line {line} has more than {MAX_COLUMNS} columns. Copy only the columns you need and paste them here."
            ),
            Self::FieldTooLong { line } => write!(
                f,
                "The row on line {line} has a cell longer than {MAX_FIELD_BYTES} characters. Copy only the columns you need (the names, the emails, the IDs) and paste them here."
            ),
            Self::UnclosedQuote { line } => write!(
                f,
                "A quote (\") opened on line {line} is never closed, so the rest of the list would be one cell. Fix it in the file, or copy the cells from your spreadsheet and paste them here."
            ),
        }
    }
}

/// A row of the list: its cells, trimmed, and the line it starts on (from
/// 1), which the preview names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub line: usize,
    pub cells: Vec<String>,
}

impl Record {
    /// Cell `i`, or empty if the row is shorter.
    pub fn cell(&self, i: usize) -> &str {
        self.cells.get(i).map_or("", String::as_str)
    }
}

/// A list read ([`parse`]): its rows, blank ones left out, the header (if
/// it has one) among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub delimiter: Delimiter,
    pub records: Vec<Record>,
    /// The most cells a row has.
    pub columns: usize,
    /// Some cell looks like text saved in another encoding and read as
    /// UTF-8 ("Ã¡" for "á"), or has a replacement character: the names
    /// would be shown so.
    pub mojibake: bool,
}

/// A file's text: UTF-8, with or without a byte order mark (which goes),
/// or UTF-16 with one (Excel's "Unicode text"). Anything else, a Windows
/// code page above all, is [`ParseError::NotUtf8`]: guessing which one
/// would get some names wrong, and a paste from the spreadsheet is always
/// right.
pub fn decode(bytes: &[u8]) -> Result<String, ParseError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ParseError::TooLarge);
    }
    let utf16 = |bytes: &[u8], from: fn([u8; 2]) -> u16| {
        let (pairs, []) = bytes.as_chunks::<2>() else {
            return Err(ParseError::NotUtf8);
        };
        let units: Vec<u16> = pairs.iter().map(|&b| from(b)).collect();
        String::from_utf16(&units).map_err(|_| ParseError::NotUtf8)
    };
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => std::str::from_utf8(rest)
            .map(str::to_owned)
            .map_err(|_| ParseError::NotUtf8),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        _ => std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ParseError::NotUtf8),
    }
}

/// Reads a list: pasted cells (tab-separated) or a CSV file's text. The
/// delimiter is sniffed from the first lines ([`sniff`]), or taken from
/// Excel's `sep=;` line; quotes are RFC 4180's (`""` inside a quoted cell
/// is a quote; a quoted cell may hold delimiters and line breaks); line
/// ends are CRLF, LF or CR; every cell is trimmed; blank rows are left out.
pub fn parse(text: &str) -> Result<Table, ParseError> {
    if text.len() > MAX_INPUT_BYTES {
        return Err(ParseError::TooLarge);
    }
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let (delimiter, body, first_line) = match excel_sep(text) {
        Some((d, rest)) => (d, rest, 2),
        None => (sniff(text), text, 1),
    };
    let records = read(body, delimiter.char(), first_line, Mode::Strict)?;
    if records.is_empty() {
        return Err(ParseError::Empty);
    }
    let columns = records.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    let mojibake = records
        .iter()
        .any(|r| r.cells.iter().any(|c| looks_mojibake(c)));
    Ok(Table {
        delimiter,
        records,
        columns,
        mojibake,
    })
}

/// Excel's first line naming the delimiter (`sep=;`), and the text after it.
fn excel_sep(text: &str) -> Option<(Delimiter, &str)> {
    let (first, rest) = text.split_once('\n').unwrap_or((text, ""));
    let sep = first.trim_end_matches('\r').trim().strip_prefix("sep=")?;
    let d = match sep {
        "\t" => Delimiter::Tab,
        "," => Delimiter::Comma,
        ";" => Delimiter::Semicolon,
        _ => return None,
    };
    Some((d, rest))
}

/// The delimiter of `text`, from its first [`SNIFF_LINES`] lines: the one
/// that splits the most of them into the same number of cells, more than
/// one, then the one that makes more cells, then tab, `;`, `,` in that
/// order. A list of one column is read with tabs (nothing splits it).
pub fn sniff(text: &str) -> Delimiter {
    let end = text
        .match_indices('\n')
        .nth(SNIFF_LINES - 1)
        .map_or(text.len(), |(i, _)| i);
    let head = &text[..end];
    Delimiter::ALL
        .into_iter()
        .enumerate()
        .filter_map(|(priority, d)| {
            let records = read(head, d.char(), 1, Mode::Sniff).ok()?;
            let mut counts: std::collections::HashMap<usize, usize> = Default::default();
            for r in &records {
                *counts.entry(r.cells.len()).or_default() += 1;
            }
            let (mode, matching) = counts
                .into_iter()
                .max_by_key(|&(cells, rows)| (rows, cells))?;
            (mode > 1).then_some(((matching, mode, usize::MAX - priority), d))
        })
        .max_by_key(|(score, _)| *score)
        .map_or(Delimiter::Tab, |(_, d)| d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Every limit holds, and an unclosed quote is an error.
    Strict,
    /// The first lines only, cut anywhere: a quote left open by the cut
    /// ends the text, and no limit applies.
    Sniff,
}

/// The records of `text`, cells split at `delimiter`, numbered from
/// `first_line`; blank rows left out.
fn read(
    text: &str,
    delimiter: char,
    first_line: usize,
    mode: Mode,
) -> Result<Vec<Record>, ParseError> {
    let strict = mode == Mode::Strict;
    let mut records = Vec::new();
    let mut cells: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut line = first_line;
    let mut record_line = first_line;
    // Inside a quoted cell, and the line its quote opened on.
    let mut quoted: Option<usize> = None;
    // The cell's closing quote has passed: what follows is kept as text.
    let mut closed = false;
    let mut chars = text.chars().peekable();

    let mut end_record = |cells: &mut Vec<String>, record_line: usize| -> Result<(), ParseError> {
        let cells = std::mem::take(cells);
        if cells.iter().all(String::is_empty) {
            return Ok(());
        }
        if strict && records.len() > MAX_ROWS {
            return Err(ParseError::TooManyRows);
        }
        records.push(Record {
            line: record_line,
            cells,
        });
        Ok(())
    };
    let end_field = |field: &mut String, cells: &mut Vec<String>, line: usize| {
        cells.push(field.trim().to_owned());
        field.clear();
        if strict && cells.len() > MAX_COLUMNS {
            return Err(ParseError::TooManyColumns { line });
        }
        Ok(())
    };

    while let Some(c) = chars.next() {
        if quoted.is_some() {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => {
                    quoted = None;
                    closed = true;
                }
                '\r' | '\n' => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    line += 1;
                    field.push('\n');
                }
                c => field.push(c),
            }
            if strict && field.len() > MAX_FIELD_BYTES {
                // No quote left to close it: the quote swallows the rest.
                if let Some(opened) = quoted.filter(|_| !chars.clone().any(|c| c == '"')) {
                    return Err(ParseError::UnclosedQuote { line: opened });
                }
                return Err(ParseError::FieldTooLong { line: record_line });
            }
            continue;
        }
        match c {
            c if c == delimiter => {
                end_field(&mut field, &mut cells, record_line)?;
                closed = false;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                end_field(&mut field, &mut cells, record_line)?;
                end_record(&mut cells, record_line)?;
                closed = false;
                line += 1;
                record_line = line;
            }
            // A quote opens a quoted cell at its start (spaces before it
            // aside); anywhere else it is a character of the cell.
            '"' if !closed && field.trim().is_empty() => {
                field.clear();
                quoted = Some(line);
            }
            c => field.push(c),
        }
        if strict && field.len() > MAX_FIELD_BYTES {
            return Err(ParseError::FieldTooLong { line: record_line });
        }
    }
    if let Some(opened) = quoted
        && strict
    {
        return Err(ParseError::UnclosedQuote { line: opened });
    }
    end_field(&mut field, &mut cells, record_line)?;
    end_record(&mut cells, record_line)?;
    Ok(records)
}

/// Whether `cell` looks like UTF-8 read in a Windows code page, then saved
/// as UTF-8 again: an accented letter's two bytes shown as two characters
/// ("Ã¡" for "á", "Å‘" for "ő", "Ã¶" for "ö"), or a character that couldn't
/// be read at all (U+FFFD).
pub fn looks_mojibake(cell: &str) -> bool {
    // What a UTF-8 continuation byte (0x80–0xBF) is in Windows-1252 or
    // Latin-1.
    fn continuation(c: char) -> bool {
        matches!(c, '\u{80}'..='\u{BF}') || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c)
    }
    cell.contains('\u{FFFD}')
        || cell
            .chars()
            .zip(cell.chars().skip(1))
            .any(|(a, b)| matches!(a, 'Ã' | 'Å' | 'Ä' | 'Â' | 'Ð' | 'Ñ') && continuation(b))
}
