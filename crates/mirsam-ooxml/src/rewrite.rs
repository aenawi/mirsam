//! Token-stream rewriting: change what a repair names, and nothing else.
//!
//! [`crate::package`] guarantees that a part a repair does not touch survives
//! byte for byte. This module owes the same guarantee one level down: inside a
//! part that *is* rewritten, every token the repair did not address comes out
//! exactly as it went in. `tests/xml_passthrough.rs` pins that down for the
//! identity case, and every mutation below is expressed as a small edit to an
//! otherwise untouched event stream.
//!
//! Two rules follow from that, and they shape all the code here.
//!
//! **Attributes are edited in their raw bytes, never rebuilt.** Re-emitting an
//! element from its parsed attributes would normalise quoting and whitespace on
//! attributes the repair never mentioned — `algn='l'` silently becoming
//! `algn="l"` is exactly the kind of unintended diff this milestone exists to
//! prevent. `set_attribute` splices a value in place, or appends one, and
//! leaves the rest of the tag alone.
//!
//! **Inserted children are placed by schema rank.** DrawingML child order is
//! significant: `CT_TextParagraphProperties` and `CT_TextCharacterProperties`
//! are `xsd:sequence`, so a correct element in the wrong position produces a
//! file PowerPoint refuses to open. `PPR_ORDER` and `RPR_ORDER` encode
//! those sequences, and insertion is by rank against them rather than by
//! appending and hoping.

use mirsam_core::Fix;
use mirsam_core::error::{Error, Result};
use mirsam_core::text::{Alignment, Direction};
use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::collections::BTreeMap;

/// Repairs for one part, keyed by paragraph index.
///
/// The index counts every `a:p` in the part, 1-based, exactly as the scanner
/// counts them — including paragraphs that produced no text unit, so the two
/// numberings cannot drift.
pub type PartFixes = BTreeMap<usize, Vec<Fix>>;

// ---------------------------------------------------------------- schema order

/// `CT_TextParagraphProperties`, in schema sequence order.
const PPR_ORDER: &[&str] = &[
    "a:lnSpc",
    "a:spcBef",
    "a:spcAft",
    "a:buClrTx",
    "a:buClr",
    "a:buSzTx",
    "a:buSzPct",
    "a:buSzPts",
    "a:buFontTx",
    "a:buFont",
    "a:buNone",
    "a:buAutoNum",
    "a:buChar",
    "a:tabLst",
    "a:defRPr",
    "a:extLst",
];

/// `CT_TextCharacterProperties`, in schema sequence order.
const RPR_ORDER: &[&str] = &[
    "a:ln",
    "a:noFill",
    "a:solidFill",
    "a:gradFill",
    "a:blipFill",
    "a:pattFill",
    "a:grpFill",
    "a:effectLst",
    "a:effectDag",
    "a:highlight",
    "a:uLnTx",
    "a:uLn",
    "a:uFillTx",
    "a:uFill",
    "a:latin",
    "a:ea",
    "a:cs",
    "a:sym",
    "a:hlinkClick",
    "a:hlinkMouseOver",
    "a:rtl",
    "a:extLst",
];

/// Position of `name` in a schema sequence; unknown elements sort last so an
/// extension we do not model is never moved.
fn rank(order: &[&str], name: &str) -> usize {
    order.iter().position(|n| *n == name).unwrap_or(order.len())
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
fn get_attribute(tag: &BytesStart, name: &str) -> Option<String> {
    let content: &str = tag;
    scan_attributes(content)
        .into_iter()
        .find(|a| a.name == name)
        .map(|a| content[a.value].to_string())
}

/// Set an attribute, replacing its value in place if present or appending it if
/// not, and leaving every other byte of the tag untouched.
fn set_attribute(tag: &BytesStart, name: &str, value: &str) -> BytesStart<'static> {
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

fn read_events(part: &str, xml: &str) -> Result<Vec<Event<'static>>> {
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

fn write_events(part: &str, events: &[Event<'static>]) -> Result<String> {
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

/// Element name of a start-ish event.
fn name_of(event: &Event<'_>) -> Option<String> {
    match event {
        Event::Start(e) | Event::Empty(e) => Some(e.name().0.to_string()),
        _ => None,
    }
}

/// Indices of the direct element children of `events[0]`, which must be the
/// container's `Start`.
fn direct_children(events: &[Event<'static>]) -> Vec<usize> {
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
fn element_range(events: &[Event<'static>], start: usize) -> std::ops::Range<usize> {
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
    expand_empty(events, at);
    let range = element_range(events, at);
    let inner: Vec<Event<'static>> = events[range.clone()].to_vec();
    let child_rank = rank(order, name_of(&child).unwrap_or_default().as_str());

    // The first existing child that sorts after the newcomer; if there is none,
    // the newcomer goes last, immediately before the closing tag.
    let position = direct_children(&inner)
        .into_iter()
        .find(|&i| rank(order, name_of(&inner[i]).unwrap_or_default().as_str()) > child_rank)
        .map_or(range.end - 1, |i| range.start + i);

    events.insert(position, child);
}

/// Find a direct child by name, or create it in schema position and return its
/// index.
fn child_or_insert(
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
fn edit_tag(
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

/// `CT_TextParagraph`: `a:pPr` precedes every run.
const A_P_ORDER: &[&str] = &["a:pPr"];
/// `CT_RegularTextRun`: `a:rPr` precedes `a:t`.
const A_R_ORDER: &[&str] = &["a:rPr"];

/// Right indent and hanging indent applied when a typed bullet is converted to
/// a native list. PowerPoint's own default for a first-level bullet, in EMU.
const BULLET_INDENT_EMU: i64 = 342_900;

/// A direct child of the element at `at`, by name.
fn find_direct_child(events: &[Event<'static>], at: usize, name: &str) -> Option<usize> {
    let range = element_range(events, at);
    let inner = &events[range.clone()];
    direct_children(inner)
        .into_iter()
        .find(|&i| name_of(&inner[i]).as_deref() == Some(name))
        .map(|i| range.start + i)
}

/// The paragraph's `a:pPr`, created in schema position if it has none.
fn ensure_ppr(para: &mut Vec<Event<'static>>) -> usize {
    child_or_insert(para, 0, A_P_ORDER, "a:pPr")
}

/// Lower a direction-relative alignment onto DrawingML's `algn`, which has only
/// physical values.
///
/// `Start` is the side reading begins on: the right in RTL, the left in LTR.
fn drawingml_alignment(alignment: Alignment, rtl: bool) -> &'static str {
    match alignment {
        Alignment::Start if rtl => "r",
        Alignment::Start => "l",
        Alignment::End if rtl => "l",
        Alignment::End => "r",
        Alignment::Left => "l",
        Alignment::Right => "r",
        Alignment::Center => "ctr",
        Alignment::Justify => "just",
        Alignment::Distributed => "dist",
    }
}

/// The event range holding each `a:t`'s content.
///
/// A range rather than a single index because the content of one `a:t` is a
/// *run* of events: quick-xml reports every character or entity reference
/// separately from the text around it, so `&#1575;` and its neighbours arrive
/// as three events describing one string.
fn text_content_ranges(para: &[Event<'static>]) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, event) in para.iter().enumerate() {
        match event {
            Event::Start(e) if e.name().0 == "a:t" => start = Some(i + 1),
            Event::End(e) if e.name().0 == "a:t" => {
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
fn read_content(events: &[Event<'static>]) -> String {
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
fn write_text(text: &str) -> Event<'static> {
    Event::Text(quick_xml::events::BytesText::from_escaped(
        quick_xml::escape::escape(text).into_owned(),
    ))
}

/// Remove explicit bidi controls at the given offsets into the paragraph's text.
///
/// The offsets index the concatenation of every run, which is what the scanner
/// produced and therefore what the domain reasoned about. They are applied back
/// to front so that removing one does not move the next.
fn remove_controls(para: &mut Vec<Event<'static>>, offsets: &[usize]) {
    let ranges = text_content_ranges(para);
    let mut runs: Vec<String> = ranges
        .iter()
        .map(|r| read_content(&para[r.clone()]))
        .collect();
    let before = runs.clone();

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

    replace_content(para, &ranges, &before, &runs);
}

/// Splice edited run text back into the paragraph, touching only the runs that
/// actually changed. Back to front, so a replacement does not move the ranges
/// still to be written.
fn replace_content(
    para: &mut Vec<Event<'static>>,
    ranges: &[std::ops::Range<usize>],
    before: &[String],
    after: &[String],
) {
    for (i, range) in ranges.iter().enumerate().rev() {
        if before[i] != after[i] {
            para.splice(range.clone(), [write_text(&after[i])]);
        }
    }
}

/// Strip a typed list marker, and the whitespace after it, from the start of
/// the paragraph's first run.
fn strip_leading_marker(para: &mut Vec<Event<'static>>, marker: char) {
    let ranges = text_content_ranges(para);
    let Some(first) = ranges.first().cloned() else {
        return;
    };
    let text = read_content(&para[first.clone()]);
    let trimmed = text.trim_start();
    if !trimmed.starts_with(marker) {
        return;
    }
    let leading = &text[..text.len() - trimmed.len()];
    let rest = trimmed[marker.len_utf8()..].trim_start();
    para.splice(first, [write_text(&format!("{leading}{rest}"))]);
}

/// Apply `f` to every run-property element in the paragraph, creating `a:rPr`
/// for runs that lack one.
///
/// Children are visited back to front: creating an `a:rPr` shifts every index
/// after it, and the ones still to be visited must not move.
fn for_each_run_property(
    para: &mut Vec<Event<'static>>,
    mut f: impl FnMut(&mut Vec<Event<'static>>, usize),
) {
    for i in direct_children(para).into_iter().rev() {
        match name_of(&para[i]).unwrap_or_default().as_str() {
            "a:r" => {
                let rpr = child_or_insert(para, i, A_R_ORDER, "a:rPr");
                f(para, rpr);
            }
            "a:endParaRPr" => f(para, i),
            "a:pPr" => {
                if let Some(default) = find_direct_child(para, i, "a:defRPr") {
                    f(para, default);
                }
            }
            _ => {}
        }
    }
}

/// Apply every repair for one paragraph.
fn apply_to_paragraph(para: &mut Vec<Event<'static>>, fixes: &[Fix]) -> Result<()> {
    // Text first: these replace text events in place, so they neither move nor
    // are moved by the structural edits that follow.
    for fix in fixes {
        match fix {
            Fix::RemoveControls(offsets) => remove_controls(para, offsets),
            Fix::ConvertLiteralBullet { marker } => strip_leading_marker(para, *marker),
            Fix::NormalizePresentationForms => {
                return Err(Error::Format(
                    "normalising presentation forms needs NFKC, which mirsam-core does not \
                     yet provide; see docs/PLAN.md M1 1.2"
                        .into(),
                ));
            }
            _ => {}
        }
    }

    // Direction is resolved before alignment, because a direction-relative
    // alignment cannot be lowered onto DrawingML without knowing it.
    let declared_rtl = fixes.iter().find_map(|f| match f {
        Fix::SetDirection(d) => Some(*d == Direction::Rtl),
        _ => None,
    });

    for fix in fixes {
        match fix {
            Fix::SetDirection(direction) => {
                let ppr = ensure_ppr(para);
                let value = if *direction == Direction::Rtl {
                    "1"
                } else {
                    "0"
                };
                edit_tag(para, ppr, |tag| set_attribute(tag, "rtl", value));
            }

            Fix::SetAlignment(alignment) => {
                let ppr = ensure_ppr(para);
                let rtl = declared_rtl.unwrap_or_else(|| {
                    let (Event::Start(tag) | Event::Empty(tag)) = &para[ppr] else {
                        return false;
                    };
                    get_attribute(tag, "rtl").is_some_and(|v| matches!(v.as_str(), "1" | "true"))
                });
                let value = drawingml_alignment(*alignment, rtl);
                edit_tag(para, ppr, |tag| set_attribute(tag, "algn", value));
            }

            Fix::SetLanguage(tag_value) => {
                for_each_run_property(para, |events, at| {
                    edit_tag(events, at, |tag| set_attribute(tag, "lang", tag_value));
                });
            }

            Fix::SetComplexFont(typeface) => {
                for_each_run_property(para, |events, at| {
                    let cs = child_or_insert(events, at, RPR_ORDER, "a:cs");
                    edit_tag(events, cs, |tag| set_attribute(tag, "typeface", typeface));
                });
            }

            Fix::ConvertLiteralBullet { marker } => {
                let ppr = ensure_ppr(para);
                // A hanging indent, so wrapped lines clear the marker.
                edit_tag(para, ppr, |tag| {
                    set_attribute(tag, "marR", &BULLET_INDENT_EMU.to_string())
                });
                edit_tag(para, ppr, |tag| {
                    set_attribute(tag, "indent", &(-BULLET_INDENT_EMU).to_string())
                });
                let bullet = child_or_insert(para, ppr, PPR_ORDER, "a:buChar");
                let marker = marker.to_string();
                edit_tag(para, bullet, |tag| set_attribute(tag, "char", &marker));
            }

            Fix::RemoveControls(_) | Fix::NormalizePresentationForms => {}
        }
    }

    Ok(())
}

/// Every `a:p` in the part with its event range, numbered as the scanner
/// numbers them: 1-based, counting paragraphs that produced no text unit.
fn paragraph_ranges(events: &[Event<'static>]) -> Vec<(usize, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut index = 0usize;
    for i in 0..events.len() {
        if name_of(&events[i]).as_deref() == Some("a:p") {
            index += 1;
            out.push((index, element_range(events, i)));
        }
    }
    out
}

/// Apply repairs to a part, leaving every token they do not address untouched.
pub fn apply(part: &str, xml: &str, fixes: &PartFixes) -> Result<String> {
    if fixes.is_empty() {
        return passthrough(part, xml);
    }

    let mut events = read_events(part, xml)?;

    if let Some(missing) = fixes
        .keys()
        .find(|k| !paragraph_ranges(&events).iter().any(|(i, _)| i == *k))
    {
        return Err(Error::Format(format!(
            "{part}: no paragraph {missing}; the document and the report disagree"
        )));
    }

    // Back to front: splicing a paragraph changes every index after it.
    for (index, range) in paragraph_ranges(&events).into_iter().rev() {
        let Some(list) = fixes.get(&index) else {
            continue;
        };
        let mut para: Vec<Event<'static>> = events[range.clone()].to_vec();
        apply_to_paragraph(&mut para, list)?;
        events.splice(range, para);
    }

    write_events(part, &events)
}
