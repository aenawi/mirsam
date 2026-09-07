//! The cross-reference: where each object is, and how to find that out when
//! the file lies about it.
//!
//! §7.5.4 gives two spellings and every real corpus contains both. A classic
//! *table* is plain text — `xref`, subsection headers, twenty-byte entries —
//! and is what anything writing PDF 1.4 or earlier emits. A cross-reference
//! *stream* is a binary table inside a `FlateDecode`d stream, introduced with
//! PDF 1.5 so that the table itself could be compressed alongside the object
//! streams it indexes. Both may be chained by `/Prev` into an arbitrarily long
//! history of incremental updates, and a *hybrid* file (§7.5.8.4) carries one
//! of each so that a 1.4 reader sees a subset of the objects a 1.5 reader sees.
//!
//! ## Newest wins, and it is the walk that enforces it
//!
//! An incremental update appends a new section and points its `/Prev` at the
//! old one, so the chain runs newest to oldest. Every insertion here is
//! therefore *first wins*: an object redefined by an update is found in the
//! section that redefined it, and the older entry is never consulted. The same
//! rule merges the trailers, which is why a `/Root` moved by an update is the
//! one that is used.
//!
//! ## And when the table is wrong
//!
//! Offsets go stale. A file concatenated by a script, truncated by a transfer
//! or edited by hand has a `startxref` pointing at nothing in particular, and
//! every viewer in the world opens it anyway by scanning for `obj`. So does
//! [`recover`] — because a document this tool refused while a reader opened it
//! would be a document reported as unreadable that is not, which is the worse
//! half of standing rule 4.

use mirsam_core::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};

use crate::filter::{self, Decoded};
use crate::lexer::{Lexer, find, rfind};
use crate::object::{Dictionary, Name, Object, is_regular, is_whitespace};

/// How long a `/Prev` chain is followed. An incremental update per save is
/// normal; ten thousand of them is a file built by a loop.
const MAX_SECTIONS: usize = 4096;

/// How far back from the end of the file `startxref` is looked for. §7.5.5
/// puts it in the last line; a kilobyte covers every writer that appends
/// something after it.
const TAIL: usize = 2048;

/// Where an object is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// At a byte offset in the file.
    InFile { offset: u64, generation: u16 },
    /// Inside an object stream, at `index` among the objects it holds.
    InStream { stream: u32, index: u32 },
    /// Deleted. Kept rather than dropped, because a free entry in a newer
    /// section is what *hides* an object an older section still names.
    Free,
}

/// A resolved cross-reference: every object's location, and the merged trailer.
#[derive(Debug, Default)]
pub struct Xref {
    pub slots: BTreeMap<u32, Slot>,
    pub trailer: Dictionary,
}

impl Xref {
    /// Record `slot` for `number` unless a newer section already did.
    fn note(&mut self, number: u32, slot: Slot) {
        self.slots.entry(number).or_insert(slot);
    }

    /// Merge a section's trailer under the same first-wins rule.
    fn merge_trailer(&mut self, trailer: &Dictionary) {
        for (key, value) in trailer.iter() {
            // The chain's own links are a property of the section that carried
            // them, never of the document.
            if key.is("Prev") || key.is("XRefStm") {
                continue;
            }
            if !self.trailer.contains_key(key.as_str().unwrap_or_default()) {
                self.trailer.insert(key.clone(), value.clone());
            }
        }
    }

    /// The objects stored in object streams, grouped by the stream holding
    /// them: loading one stream answers for every object in it.
    pub fn in_streams(&self) -> BTreeMap<u32, Vec<(u32, u32)>> {
        let mut grouped: BTreeMap<u32, Vec<(u32, u32)>> = BTreeMap::new();
        for (&number, slot) in &self.slots {
            if let Slot::InStream { stream, index } = *slot {
                grouped.entry(stream).or_default().push((number, index));
            }
        }
        grouped
    }
}

/// Follow the chain from `startxref` and answer with every object's location.
///
/// `shift` is the offset of `%PDF-` within the file, which §7.5.5 makes the
/// origin every stored offset is relative to. It is zero for a well-formed
/// document and non-zero for one with a prologue bolted on the front.
pub fn read(data: &[u8], shift: usize) -> Result<Xref> {
    let Some(start) = startxref(data) else {
        return Err(Error::Format(
            "no `startxref`: the file does not say where its cross-reference table is".into(),
        ));
    };

    let mut xref = Xref::default();
    let mut next = Some(start);
    let mut seen = BTreeSet::new();

    for _ in 0..MAX_SECTIONS {
        let Some(offset) = next.take() else { break };
        let Some(at) = locate(data, offset, shift) else {
            return Err(Error::Format(format!(
                "cross-reference section at byte {offset} is not there"
            )));
        };
        if !seen.insert(at) {
            // A `/Prev` pointing into a section already walked. Two writers
            // producing the same offset is a loop, not a longer history.
            break;
        }

        let section = section(data, at)?;

        // §7.5.8.4: in a hybrid file the stream is the authority for the
        // section, and the classic table beside it deliberately understates
        // what the file holds. Reading it first makes it win.
        if let Some(hybrid) = section.hybrid
            && let Some(hybrid_at) = locate(data, hybrid, shift)
            && seen.insert(hybrid_at)
            && let Ok(stream_section) = self::section(data, hybrid_at)
        {
            for (number, slot) in stream_section.slots {
                xref.note(number, slot);
            }
            xref.merge_trailer(&stream_section.trailer);
        }

        for (number, slot) in section.slots {
            xref.note(number, slot);
        }
        xref.merge_trailer(&section.trailer);
        next = section.previous;
    }

    if xref.slots.is_empty() {
        return Err(Error::Format(
            "the cross-reference table names no objects".into(),
        ));
    }
    Ok(xref)
}

/// Rebuild the table by scanning for `<n> <g> obj`.
///
/// Later definitions win, which is the same rule the chain walk enforces from
/// the other end: an incremental update appends, so the last definition of an
/// object number in the file is the current one.
pub fn recover(data: &[u8]) -> Xref {
    let mut xref = Xref::default();
    let mut at = 0usize;

    while let Some(found) = find(&data[at..], b"obj") {
        let keyword = at + found;
        at = keyword + 3;
        // `obj` has to be a token of its own: `objstm` is not one.
        if data.get(at).copied().is_some_and(is_regular) {
            continue;
        }
        let Some((number, generation, start)) = header_before(data, keyword) else {
            continue;
        };
        xref.slots.insert(
            number,
            Slot::InFile {
                offset: start as u64,
                generation,
            },
        );
    }

    // Every `trailer` in the file, oldest first, so the newest wins.
    let mut at = 0usize;
    while let Some(found) = find(&data[at..], b"trailer") {
        let keyword = at + found;
        at = keyword + b"trailer".len();
        let mut lexer = Lexer::at(data, at);
        if let Ok(Object::Dictionary(dict)) = lexer.object() {
            for (key, value) in dict.iter() {
                if key.is("Prev") || key.is("XRefStm") {
                    continue;
                }
                xref.trailer.insert(key.clone(), value.clone());
            }
        }
    }
    xref
}

/// Walk back from an `obj` keyword over `<n> <g> `, answering the two numbers
/// and where the indirect object begins.
fn header_before(data: &[u8], keyword: usize) -> Option<(u32, u16, usize)> {
    let mut end = keyword;
    let back = |end: &mut usize| -> Option<(usize, usize)> {
        while *end > 0 && is_whitespace(data[*end - 1]) {
            *end -= 1;
        }
        let stop = *end;
        while *end > 0 && data[*end - 1].is_ascii_digit() {
            *end -= 1;
        }
        (*end < stop).then_some((*end, stop))
    };

    let (generation_start, generation_end) = back(&mut end)?;
    let (number_start, number_end) = back(&mut end)?;
    // The number must start a token: `x12 0 obj` is not an object header.
    if number_start > 0 && is_regular(data[number_start - 1]) {
        return None;
    }
    let number = std::str::from_utf8(&data[number_start..number_end])
        .ok()?
        .parse::<u32>()
        .ok()?;
    let generation = std::str::from_utf8(&data[generation_start..generation_end])
        .ok()?
        .parse::<u16>()
        .ok()?;
    Some((number, generation, number_start))
}

/// The offset of the last `startxref` value in the file.
fn startxref(data: &[u8]) -> Option<u64> {
    let from = data.len().saturating_sub(TAIL);
    let found = rfind(&data[from..], b"startxref")? + from;
    let mut lexer = Lexer::at(data, found + b"startxref".len());
    lexer.object().ok()?.as_i64()?.try_into().ok()
}

/// Turn a stored offset into a position in this file.
///
/// A file with bytes before its `%PDF-` header stores offsets relative to the
/// header, and one whose prologue was added *after* it was written stores them
/// relative to the file. Both are common; both are tried, and the one that
/// lands on something that begins a cross-reference section wins.
fn locate(data: &[u8], offset: u64, shift: usize) -> Option<usize> {
    let candidates = [
        usize::try_from(offset).ok()?.checked_add(shift)?,
        usize::try_from(offset).ok()?,
    ];
    for at in candidates {
        if at >= data.len() {
            continue;
        }
        let mut lexer = Lexer::at(data, at);
        let rewind = lexer.pos();
        if lexer.keyword("xref") {
            return Some(at);
        }
        lexer.seek(rewind);
        if lexer.indirect().is_ok() {
            return Some(at);
        }
    }
    None
}

/// One section of the chain, before it is merged into the whole.
struct Section {
    slots: Vec<(u32, Slot)>,
    trailer: Dictionary,
    previous: Option<u64>,
    /// `/XRefStm`, the cross-reference stream a hybrid file hides beside its
    /// classic table.
    hybrid: Option<u64>,
}

fn section(data: &[u8], at: usize) -> Result<Section> {
    let mut lexer = Lexer::at(data, at);
    if lexer.keyword("xref") {
        return classic(data, lexer);
    }
    lexer.seek(at);
    let (_, object) = lexer.indirect()?;
    let Some(stream) = object.as_stream() else {
        return Err(Error::Format(format!(
            "byte {at} begins neither an `xref` table nor a cross-reference stream"
        )));
    };
    stream_section(stream)
}

/// The plain-text table: subsection headers, then twenty-byte entries.
fn classic(data: &[u8], mut lexer: Lexer<'_>) -> Result<Section> {
    let mut slots = Vec::new();
    loop {
        let rewind = lexer.pos();
        if lexer.keyword("trailer") {
            break;
        }
        lexer.seek(rewind);
        let Ok(Some(first)) = lexer.object().map(|o| o.as_i64()) else {
            lexer.seek(rewind);
            break;
        };
        let Ok(Some(count)) = lexer.object().map(|o| o.as_i64()) else {
            return Err(Error::Format(format!(
                "cross-reference subsection at byte {rewind} has no entry count"
            )));
        };
        let Ok(first) = u32::try_from(first) else {
            return Err(Error::Format(format!(
                "cross-reference subsection at byte {rewind} starts at object {first}"
            )));
        };

        for index in 0..count.max(0) {
            let offset = lexer.object()?.as_i64().unwrap_or(-1);
            let generation = lexer.object()?.as_i64().unwrap_or(0);
            lexer.skip_space();
            let kind = data.get(lexer.pos()).copied().unwrap_or(b'n');
            lexer.seek(lexer.pos() + 1);

            let Some(number) = first.checked_add(index as u32) else {
                break;
            };
            let slot = match kind {
                b'f' => Slot::Free,
                _ => match (u64::try_from(offset), u16::try_from(generation)) {
                    (Ok(offset), Ok(generation)) => Slot::InFile { offset, generation },
                    // An entry a reader cannot use is a free one: it names no
                    // place in the file, and treating it as one would send the
                    // object loader to byte zero.
                    _ => Slot::Free,
                },
            };
            slots.push((number, slot));
        }
    }

    let trailer = match lexer.object()? {
        Object::Dictionary(dict) => dict,
        other => {
            return Err(Error::Format(format!(
                "expected a trailer dictionary, found {other:?}"
            )));
        }
    };
    Ok(Section {
        previous: trailer
            .get("Prev")
            .and_then(Object::as_i64)
            .and_then(|p| u64::try_from(p).ok()),
        hybrid: trailer
            .get("XRefStm")
            .and_then(Object::as_i64)
            .and_then(|p| u64::try_from(p).ok()),
        slots,
        trailer,
    })
}

/// The binary table: `/W` gives each field's width in bytes, `/Index` which
/// object numbers the rows are for.
fn stream_section(stream: &crate::object::Stream) -> Result<Section> {
    let dict = &stream.dict;
    let bytes = match decode_directly(stream)? {
        Decoded::Bytes(bytes) => bytes,
        Decoded::Stopped { filter } => {
            return Err(Error::Format(format!(
                "cross-reference stream is behind {filter}, which is not a filter this tool decodes"
            )));
        }
    };

    let widths: Vec<usize> = dict
        .get("W")
        .and_then(Object::as_array)
        .ok_or_else(|| Error::Format("cross-reference stream has no /W".into()))?
        .iter()
        .map(|w| w.as_i64().unwrap_or(0).clamp(0, 8) as usize)
        .collect();
    if widths.len() < 3 {
        return Err(Error::Format(format!(
            "cross-reference stream /W has {} fields, and the format has three",
            widths.len()
        )));
    }
    let row = widths.iter().sum::<usize>();
    if row == 0 {
        return Err(Error::Format(
            "cross-reference stream /W gives every field a width of zero".into(),
        ));
    }

    // §7.5.8.2: /Index defaults to the whole of /Size.
    let size = dict.get("Size").and_then(Object::as_i64).unwrap_or(0);
    let index: Vec<i64> = match dict.get("Index").and_then(Object::as_array) {
        Some(pairs) => pairs.iter().map(|p| p.as_i64().unwrap_or(0)).collect(),
        None => vec![0, size],
    };

    let mut slots = Vec::new();
    let mut cursor = 0usize;
    for pair in index.chunks(2) {
        let (&first, &count) = match pair {
            [first, count] => (first, count),
            _ => break,
        };
        for i in 0..count.max(0) {
            if cursor + row > bytes.len() {
                break;
            }
            let mut fields = [0u64; 3];
            let mut at = cursor;
            for (field, &width) in fields.iter_mut().zip(&widths) {
                for _ in 0..width {
                    *field = (*field << 8) | u64::from(bytes[at]);
                    at += 1;
                }
            }
            // A zero-width first field means type 1, per §7.5.8.2.
            if widths[0] == 0 {
                fields[0] = 1;
            }
            cursor += row;

            let Ok(number) = u32::try_from(first + i) else {
                continue;
            };
            let slot = match fields[0] {
                0 => Slot::Free,
                1 => Slot::InFile {
                    offset: fields[1],
                    generation: u16::try_from(fields[2]).unwrap_or(0),
                },
                2 => match (u32::try_from(fields[1]), u32::try_from(fields[2])) {
                    (Ok(stream), Ok(index)) => Slot::InStream { stream, index },
                    _ => continue,
                },
                // §7.5.8.3: a type this reader does not know is to be treated
                // as free rather than guessed at.
                _ => Slot::Free,
            };
            slots.push((number, slot));
        }
    }

    Ok(Section {
        previous: dict
            .get("Prev")
            .and_then(Object::as_i64)
            .and_then(|p| u64::try_from(p).ok()),
        hybrid: None,
        slots,
        trailer: dict.clone(),
    })
}

/// Decode a stream whose dictionary cannot contain indirect references.
///
/// A cross-reference stream is the one stream that has to be read before the
/// table exists, so §7.5.8.2 requires its `/Filter`, `/W` and `/Index` to be
/// direct. That is what makes this function possible, and why the general path
/// through [`crate::Pdf::decoded`] is not used here.
fn decode_directly(stream: &crate::object::Stream) -> Result<Decoded> {
    let filters = match stream.dict.get("Filter") {
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(items)) => items.iter().filter_map(Object::as_name).cloned().collect(),
        _ => Vec::<Name>::new(),
    };
    let parms = match stream.dict.get("DecodeParms") {
        Some(Object::Dictionary(dict)) => vec![dict.clone()],
        Some(Object::Array(items)) => items
            .iter()
            .map(|item| item.as_dict().cloned().unwrap_or_default())
            .collect(),
        _ => Vec::new(),
    };
    filter::decode(&stream.data, &filters, &parms)
}
