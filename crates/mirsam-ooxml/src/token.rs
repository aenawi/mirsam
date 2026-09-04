//! The token-rewrite scaffold: edit a named element or attribute and leave
//! every other token of the part exactly as it was.
//!
//! This is the half of a repair that knows nothing about the format being
//! repaired. It reads a part into an event stream, finds elements by name,
//! splices attribute values in their raw bytes, inserts children at a position
//! a caller-supplied schema sequence decides, and reads and rewrites run text.
//! Which elements and attributes those are is the *vocabulary*, and the
//! vocabulary lives with its format — [`crate::rewrite`] holds DrawingML's.
//!
//! Nothing here names an element. `a:t` and `w:t` are both "the element whose
//! content is run text", and a caller says which it means; `a:pPr` and `w:pPr`
//! are both "a child that goes in schema position", and a caller supplies the
//! sequence. That is the whole of the separation, and it is why a second
//! adapter reuses this file rather than copying it.
//!
//! Two rules follow from the byte-preservation guarantee, and they shape all
//! the code here.
//!
//! **Attributes are edited in their raw bytes, never rebuilt.** Re-emitting an
//! element from its parsed attributes would normalise quoting and whitespace on
//! attributes the repair never mentioned — `algn='l'` silently becoming
//! `algn="l"` is exactly the kind of unintended diff M1 exists to prevent.
//! [`set_attribute`] splices a value in place, or appends one, and leaves the
//! rest of the tag alone.
//!
//! **Inserted children are placed by schema rank.** OOXML child order is
//! significant — most property elements are `xsd:sequence`, so a correct
//! element in the wrong position produces a file the application refuses to
//! open. Insertion is by rank against the sequence the caller passes rather
//! than by appending and hoping.

use mirsam_core::error::{Error, Result};
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};

// ------------------------------------------------------------ value lexemes

/// Whether an `ST_OnOff` lexical value is true.
///
/// Shared rather than per-vocabulary because it names no element: `ST_OnOff`
/// is defined once in ECMA-376 and every OOXML dialect spells its booleans
/// the same way — DrawingML's `rtl="1"`, WordprocessingML's `w:val="true"`.
/// One definition is also what stops a scanner and a rewriter from disagreeing
/// about what a document says.
///
/// This reads a value that is *present*. Whether an absent one means true is
/// a question about the element carrying it, not about the lexical space, and
/// the two vocabularies answer it differently: WordprocessingML's `<w:bidi/>`
/// is true, while a DrawingML attribute that is not there was never written.
pub fn is_true(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

// ------------------------------------------------------------ raw attributes

/// One attribute's name and the byte range of its value inside a tag's content.
struct RawAttribute {
    name: String,
    value: std::ops::Range<usize>,
}

/// Scan a start tag's raw content into attribute names and value ranges.
///
/// Hand-written rather than delegated to `quick_xml::Attributes` because the
/// byte ranges are the whole point: they are what makes an in-place value
/// splice possible, and the parsed API does not expose them.
fn scan_attributes(content: &str) -> Vec<RawAttribute> {
    let content = content.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    // Skip the element name.
    while i < content.len() && !content[i].is_ascii_whitespace() {
        i += 1;
    }

    while i < content.len() {
        while i < content.len() && content[i].is_ascii_whitespace() {
            i += 1;
        }
        // A trailing `/` on an empty element, or the end of the tag.
        if i >= content.len() || content[i] == b'/' {
            break;
        }

        let name_start = i;
        while i < content.len() && content[i] != b'=' && !content[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = String::from_utf8_lossy(&content[name_start..i]).into_owned();

        while i < content.len() && content[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= content.len() || content[i] != b'=' {
            // A valueless token: not well-formed XML, but skip it rather than
            // misreading everything after it.
            continue;
        }
        i += 1;
        while i < content.len() && content[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= content.len() || (content[i] != b'"' && content[i] != b'\'') {
            continue;
        }

        let quote = content[i];
        i += 1;
        let value_start = i;
        while i < content.len() && content[i] != quote {
            i += 1;
        }
        out.push(RawAttribute {
            name,
            value: value_start..i,
        });
        i += 1; // past the closing quote
    }

    out
}

/// Read an attribute's raw (still-escaped) value.
pub fn get_attribute(tag: &BytesStart, name: &str) -> Option<String> {
    let content: &str = tag;
    scan_attributes(content)
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| content[a.value].to_string())
}

/// Set an attribute, replacing its value in place if present or appending it if
/// not, and leaving every other byte of the tag untouched.
pub fn set_attribute(tag: &BytesStart, name: &str, value: &str) -> BytesStart<'static> {
    let content: &str = tag;
    let escaped = quick_xml::escape::escape(value);

    let replaced = match scan_attributes(content)
        .into_iter()
        .find(|a| a.name == name)
    {
        Some(found) => {
            format!(
                "{}{}{}",
                &content[..found.value.start],
                escaped,
                &content[found.value.end..]
            )
        }
        None => {
            // Attribute order is not schema-significant, so append. Trailing
            // whitespace inside the tag is preserved by inserting before it.
            let end = content
                .bytes()
                .rposition(|b| !b.is_ascii_whitespace())
                .map_or(content.len(), |i| i + 1);
            format!(
                "{} {name}=\"{escaped}\"{}",
                &content[..end],
                &content[end..]
            )
        }
    };

    BytesStart::from_content(replaced, tag.name().0.len())
}

// --------------------------------------------------------------- event stream

/// Read a part into an owned event stream.
pub fn read_events(part: &str, xml: &str) -> Result<Vec<Event<'static>>> {
    let mut reader = Reader::from_str(xml);
    let mut events = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| Error::Format(format!("{part}: {e}")))?
        {
            Event::Eof => break,
            event => events.push(event.into_owned()),
        }
    }
    Ok(events)
}

/// Write an event stream back out as a part.
pub fn write_events(part: &str, events: &[Event<'static>]) -> Result<String> {
    let mut writer = Writer::new(Vec::new());
    for event in events {
        writer
            .write_event(event.clone())
            .map_err(|e| Error::Format(format!("{part}: {e}")))?;
    }
    String::from_utf8(writer.into_inner()).map_err(|e| Error::Format(format!("{part}: {e}")))
}

/// Re-emit a part unchanged. The identity case, named because it is the
/// property the round-trip test pins down.
pub fn passthrough(part: &str, xml: &str) -> Result<String> {
    write_events(part, &read_events(part, xml)?)
}

// ------------------------------------------------------------------- elements

/// Element name of a start-ish event.
pub fn name_of(event: &Event<'_>) -> Option<String> {
    match event {
        Event::Start(e) | Event::Empty(e) => Some(e.name().0.to_string()),
        _ => None,
    }
}

/// Indices of the direct element children of `events[0]`, which must be the
/// container's `Start`.
pub fn direct_children(events: &[Event<'static>]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    for (i, event) in events.iter().enumerate().skip(1) {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    out.push(i);
                }
                depth += 1;
            }
            Event::Empty(_) => {
                if depth == 0 {
                    out.push(i);
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    out
}

/// The half-open event range of the element starting at `start`.
pub fn element_range(events: &[Event<'static>], start: usize) -> std::ops::Range<usize> {
    if matches!(events[start], Event::Empty(_)) {
        return start..start + 1;
    }
    let mut depth = 0usize;
    for (i, event) in events.iter().enumerate().skip(start) {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return start..i + 1;
                }
            }
            _ => {}
        }
    }
    start..events.len()
}

/// Every element named `name` in the part with its event range, 1-based in
/// document order — exactly as a scanner numbers the elements it reports on.
pub fn element_ranges(
    events: &[Event<'static>],
    name: &str,
) -> Vec<(usize, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut index = 0usize;
    for i in 0..events.len() {
        if name_of(&events[i]).as_deref() == Some(name) {
            index += 1;
            out.push((index, element_range(events, i)));
        }
    }
    out
}

/// A direct child of the element at `at`, by name.
pub fn find_direct_child(events: &[Event<'static>], at: usize, name: &str) -> Option<usize> {
    let range = element_range(events, at);
    let inner = &events[range.clone()];
    direct_children(inner)
        .into_iter()
        .find(|&i| name_of(&inner[i]).as_deref() == Some(name))
        .map(|i| range.start + i)
}

// -------------------------------------------------------------------- editing

/// Position of `name` in a schema sequence; unknown elements sort last so an
/// extension we do not model is never moved.
fn rank(order: &[&str], name: &str) -> usize {
    order.iter().position(|n| *n == name).unwrap_or(order.len())
}

/// Turn `<x/>` into `<x></x>` so a child can be inserted, and report where the
/// children go.
fn expand_empty(events: &mut Vec<Event<'static>>, at: usize) {
    if let Event::Empty(tag) = events[at].clone() {
        let name = tag.name().0.to_string();
        events[at] = Event::Start(tag);
        events.insert(at + 1, Event::End(BytesEnd::new(name)));
    }
}

/// Insert `child` into the element at `at`, in schema-sequence position.
fn insert_child(
    events: &mut Vec<Event<'static>>,
    at: usize,
    order: &[&str],
    child: Event<'static>,
) {
    insert_children(events, at, order, vec![child]);
}

/// Insert a whole element — its start tag, its contents and its end tag —
/// into the element at `at`, in the schema-sequence position its first event
/// ranks at.
pub fn insert_children(
    events: &mut Vec<Event<'static>>,
    at: usize,
    order: &[&str],
    children: Vec<Event<'static>>,
) {
    let Some(first) = children.first() else {
        return;
    };
    expand_empty(events, at);
    let range = element_range(events, at);
    let inner: Vec<Event<'static>> = events[range.clone()].to_vec();
    let child_rank = rank(order, name_of(first).unwrap_or_default().as_str());

    // The first existing child that sorts after the newcomer; if there is none,
    // the newcomer goes last, immediately before the closing tag.
    let position = direct_children(&inner)
        .into_iter()
        .find(|&i| rank(order, name_of(&inner[i]).unwrap_or_default().as_str()) > child_rank)
        .map_or(range.end - 1, |i| range.start + i);

    events.splice(position..position, children);
}

/// Find a direct child by name, or create it in schema position and return its
/// index.
pub fn child_or_insert(
    events: &mut Vec<Event<'static>>,
    at: usize,
    order: &[&str],
    name: &str,
) -> usize {
    let range = element_range(events, at);
    let inner: Vec<Event<'static>> = events[range.clone()].to_vec();
    if let Some(i) = direct_children(&inner)
        .into_iter()
        .find(|&i| name_of(&inner[i]).as_deref() == Some(name))
    {
        return range.start + i;
    }

    insert_child(
        events,
        at,
        order,
        Event::Empty(BytesStart::new(name.to_string())),
    );
    // Locate it again: insert_child may have expanded an empty parent, moving
    // everything after it.
    let range = element_range(events, at);
    let inner: Vec<Event<'static>> = events[range.clone()].to_vec();
    range.start
        + direct_children(&inner)
            .into_iter()
            .find(|&i| name_of(&inner[i]).as_deref() == Some(name))
            .expect("the child was just inserted")
}

/// Apply `f` to the tag at `at`, whether it is a `Start` or an `Empty`.
pub fn edit_tag(
    events: &mut [Event<'static>],
    at: usize,
    f: impl FnOnce(&BytesStart) -> BytesStart<'static>,
) {
    events[at] = match &events[at] {
        Event::Start(tag) => Event::Start(f(tag)),
        Event::Empty(tag) => Event::Empty(f(tag)),
        other => other.clone(),
    };
}

// ----------------------------------------------------------------------- text

/// The event range holding each text element's content, where `text_element`
/// names the element a run's characters live in — `a:t` in DrawingML, `w:t` in
/// WordprocessingML.
///
/// A range rather than a single index because that content is a *run* of
/// events: quick-xml reports every character or entity reference separately
/// from the text around it, so `&#1575;` and its neighbours arrive as three
/// events describing one string.
pub fn text_content_ranges(
    events: &[Event<'static>],
    text_element: &str,
) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, event) in events.iter().enumerate() {
        match event {
            Event::Start(e) if e.name().0 == text_element => start = Some(i + 1),
            Event::End(e) if e.name().0 == text_element => {
                if let Some(from) = start.take() {
                    out.push(from..i);
                }
            }
            _ => {}
        }
    }
    out
}

/// Resolve a run of content events to logical-order text.
pub fn read_content(events: &[Event<'static>]) -> String {
    let mut out = String::new();
    for event in events {
        match event {
            Event::Text(e) => {
                let raw = e.xml10_content();
                match quick_xml::escape::unescape(raw.as_ref()) {
                    Ok(text) => out.push_str(text.as_ref()),
                    Err(_) => out.push_str(raw.as_ref()),
                }
            }
            Event::GeneralRef(e) => {
                let reference = e.as_ref();
                match quick_xml::escape::unescape(&format!("&{reference};")) {
                    Ok(text) => out.push_str(text.as_ref()),
                    Err(_) => {
                        out.push('&');
                        out.push_str(reference);
                        out.push(';');
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Replace a text event's content, escaping it for XML.
pub fn write_text(text: &str) -> Event<'static> {
    Event::Text(quick_xml::events::BytesText::from_escaped(
        quick_xml::escape::escape(text).into_owned(),
    ))
}

/// Every run's text, in document order.
pub fn read_runs(events: &[Event<'static>], ranges: &[std::ops::Range<usize>]) -> Vec<String> {
    ranges
        .iter()
        .map(|r| read_content(&events[r.clone()]))
        .collect()
}

/// Splice edited run text back into the element, touching only the runs that
/// actually changed. Back to front, so a replacement does not move the ranges
/// still to be written.
pub fn replace_content(
    events: &mut Vec<Event<'static>>,
    ranges: &[std::ops::Range<usize>],
    before: &[String],
    after: &[String],
) {
    for (i, range) in ranges.iter().enumerate().rev() {
        if before[i] != after[i] {
            events.splice(range.clone(), [write_text(&after[i])]);
        }
    }
}

/// Rewrite every run's text with `f`, leaving a run it does not change
/// untouched — character references and all.
pub fn map_runs(events: &mut Vec<Event<'static>>, text_element: &str, f: impl Fn(&str) -> String) {
    let ranges = text_content_ranges(events, text_element);
    let before = read_runs(events, &ranges);
    let after: Vec<String> = before.iter().map(|text| f(text)).collect();
    replace_content(events, &ranges, &before, &after);
}

/// Remove the characters at the given offsets into the element's text.
///
/// The offsets index the concatenation of every run, which is what a scanner
/// produced and therefore what the domain reasoned about. They are applied back
/// to front so that removing one does not move the next.
pub fn remove_at_offsets(events: &mut Vec<Event<'static>>, text_element: &str, offsets: &[usize]) {
    let ranges = text_content_ranges(events, text_element);
    let before = read_runs(events, &ranges);
    let mut runs = before.clone();

    let mut sorted: Vec<usize> = offsets.to_vec();
    sorted.sort_unstable();

    for offset in sorted.into_iter().rev() {
        let mut base = 0usize;
        for text in runs.iter_mut() {
            if offset < base + text.len() {
                let local = offset - base;
                if text.is_char_boundary(local)
                    && let Some(c) = text[local..].chars().next()
                {
                    text.replace_range(local..local + c.len_utf8(), "");
                }
                break;
            }
            base += text.len();
        }
    }

    replace_content(events, &ranges, &before, &runs);
}

/// Strip a leading marker character, and the whitespace after it, from the
/// start of the element's first run.
pub fn strip_leading_marker(events: &mut Vec<Event<'static>>, text_element: &str, marker: char) {
    let ranges = text_content_ranges(events, text_element);
    let Some(first) = ranges.first().cloned() else {
        return;
    };
    let text = read_content(&events[first.clone()]);
    let trimmed = text.trim_start();
    if !trimmed.starts_with(marker) {
        return;
    }
    let leading = &text[..text.len() - trimmed.len()];
    let rest = trimmed[marker.len_utf8()..].trim_start();
    events.splice(first, [write_text(&format!("{leading}{rest}"))]);
}
