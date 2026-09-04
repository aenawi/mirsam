//! Is the token scaffold actually format-neutral?
//!
//! `rewrite.rs` exercises it through DrawingML, which cannot answer that
//! question: a scaffold with `a:t` or `a:pPr` baked into it passes every one of
//! those tests. So every case here is **WordprocessingML** — `w:p`, `w:pPr`,
//! `w:bidi`, `w:jc`, `w:t` — a vocabulary no module in this crate reads yet.
//! A scaffold that has learned a DrawingML name fails here, and that is the
//! whole point of the file (PLAN M3 3.1).
//!
//! The assertions are on the whole rewritten string, as `rewrite.rs`'s are:
//! any unintended byte — a normalised quote, a resolved character reference, a
//! moved child — has to fail, or "exactly the intended change and nothing
//! else" is not a claim being tested.

use mirsam_ooxml::token;
use quick_xml::events::{BytesEnd, BytesStart, Event};

const PART: &str = "word/document.xml";

/// `CT_PPrBase`, in schema sequence order — the subset these tests place
/// children against. `w:bidi` precedes `w:jc`, and both follow `w:pStyle`.
const W_PPR_ORDER: &[&str] = &[
    "w:pStyle",
    "w:numPr",
    "w:bidi",
    "w:spacing",
    "w:ind",
    "w:jc",
    "w:rPr",
];

/// `CT_P`: the paragraph's properties precede every run.
const W_P_ORDER: &[&str] = &["w:pPr"];

/// The element a run's characters live in, in Word rather than PowerPoint.
const W_T: &str = "w:t";

fn read(xml: &str) -> Vec<Event<'static>> {
    token::read_events(PART, xml).expect("the part did not parse")
}

fn write(events: &[Event<'static>]) -> String {
    token::write_events(PART, events).expect("the part did not serialise")
}

// ------------------------------------------------------------------ identity

#[test]
fn passthrough_preserves_quoting_prefixes_and_character_references() {
    // Single quotes, a namespace prefix named by an mc:Ignorable string, an
    // unresolved character reference, and a self-closing tag with a space
    // before its slash. Every one of these is something a DOM round-trip
    // changes.
    let xml = concat!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        r#"<w:p xmlns:w="ns/w" xmlns:mc="ns/mc" mc:Ignorable='w14'>"#,
        r#"<w:pPr><w:jc w:val='both' /></w:pPr>"#,
        r#"<w:r><w:t>&#1605;&#1585;&#1581;&#1576;&#1575;</w:t></w:r>"#,
        r#"</w:p>"#,
    );
    assert_eq!(token::passthrough(PART, xml).unwrap(), xml);
}

// ---------------------------------------------------------------- attributes

#[test]
fn an_attribute_value_is_spliced_in_place_and_its_neighbours_left_alone() {
    let mut events = read(r#"<w:p><w:pPr><w:bidi w:val='0' w:foo="x"/></w:pPr></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let bidi = token::find_direct_child(&events, ppr, "w:bidi").expect("no w:bidi");
    token::edit_tag(&mut events, bidi, |tag| {
        token::set_attribute(tag, "w:val", "1")
    });

    // `w:foo` keeps its double quotes, `w:val` keeps its single ones.
    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:bidi w:val='1' w:foo="x"/></w:pPr></w:p>"#
    );
}

#[test]
fn an_absent_attribute_is_appended_before_the_tags_trailing_whitespace() {
    let mut events = read(r#"<w:p><w:pPr><w:jc w:val="left" /></w:pPr></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let jc = token::find_direct_child(&events, ppr, "w:jc").expect("no w:jc");
    token::edit_tag(&mut events, jc, |tag| {
        token::set_attribute(tag, "w:extra", "y")
    });

    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:jc w:val="left" w:extra="y" /></w:pPr></w:p>"#
    );
}

#[test]
fn an_attribute_value_is_read_back_still_escaped() {
    let events = read(r#"<w:p><w:pPr><w:pStyle w:val="a&amp;b"/></w:pPr></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let style = token::find_direct_child(&events, ppr, "w:pStyle").expect("no w:pStyle");
    let Event::Empty(tag) = &events[style] else {
        panic!("w:pStyle is not an empty element");
    };
    assert_eq!(
        token::get_attribute(tag, "w:val").as_deref(),
        Some("a&amp;b")
    );
    assert_eq!(token::get_attribute(tag, "w:missing"), None);
}

// -------------------------------------------------------------- schema order

#[test]
fn a_created_child_lands_in_schema_position_not_at_the_end() {
    let mut events =
        read(r#"<w:p><w:pPr><w:pStyle w:val="Body"/><w:jc w:val="left"/></w:pPr></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let bidi = token::child_or_insert(&mut events, ppr, W_PPR_ORDER, "w:bidi");
    token::edit_tag(&mut events, bidi, |tag| {
        token::set_attribute(tag, "w:val", "1")
    });

    // Between w:pStyle and w:jc, which is where CT_PPrBase says it goes.
    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:pStyle w:val="Body"/><w:bidi w:val="1"/><w:jc w:val="left"/></w:pPr></w:p>"#
    );
}

#[test]
fn an_empty_parent_is_expanded_so_a_child_can_go_inside_it() {
    let mut events = read(r#"<w:p><w:pPr/><w:r><w:t>م</w:t></w:r></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let bidi = token::child_or_insert(&mut events, ppr, W_PPR_ORDER, "w:bidi");
    token::edit_tag(&mut events, bidi, |tag| {
        token::set_attribute(tag, "w:val", "1")
    });

    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>م</w:t></w:r></w:p>"#
    );
}

#[test]
fn a_paragraph_with_no_properties_gets_them_before_its_first_run() {
    let mut events = read(r#"<w:p><w:r><w:t>م</w:t></w:r></w:p>"#);
    let ppr = token::child_or_insert(&mut events, 0, W_P_ORDER, "w:pPr");
    let bidi = token::child_or_insert(&mut events, ppr, W_PPR_ORDER, "w:bidi");
    token::edit_tag(&mut events, bidi, |tag| {
        token::set_attribute(tag, "w:val", "1")
    });

    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>م</w:t></w:r></w:p>"#
    );
}

#[test]
fn a_whole_element_is_inserted_in_the_position_its_first_event_ranks_at() {
    let mut events = read(r#"<w:p><w:pPr><w:jc w:val="left"/></w:pPr></w:p>"#);
    let ppr = token::find_direct_child(&events, 0, "w:pPr").expect("no w:pPr");
    let mut level = BytesStart::new("w:ilvl");
    level.push_attribute(("w:val", "0"));
    token::insert_children(
        &mut events,
        ppr,
        W_PPR_ORDER,
        vec![
            Event::Start(BytesStart::new("w:numPr")),
            Event::Empty(level),
            Event::End(BytesEnd::new("w:numPr")),
        ],
    );

    assert_eq!(
        write(&events),
        r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr><w:jc w:val="left"/></w:pPr></w:p>"#
    );
}

// ------------------------------------------------------------------ elements

#[test]
fn elements_are_numbered_one_based_in_document_order_including_empty_ones() {
    let events = read(concat!(
        r#"<w:body>"#,
        r#"<w:p><w:r><w:t>one</w:t></w:r></w:p>"#,
        r#"<w:p/>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>three</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"</w:body>"#,
    ));

    let paragraphs = token::element_ranges(&events, "w:p");
    assert_eq!(
        paragraphs.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "the empty paragraph must be counted, or the scanner's numbering drifts"
    );

    // The third is the one inside the table, and its range covers it whole.
    let (_, third) = paragraphs[2].clone();
    assert_eq!(
        write(&events[third]),
        r#"<w:p><w:r><w:t>three</w:t></w:r></w:p>"#
    );
    assert_eq!(token::element_ranges(&events, "w:tbl").len(), 1);
}

// ---------------------------------------------------------------------- text

#[test]
fn only_the_runs_that_changed_are_rewritten() {
    // The second run holds the marker being replaced; the first and third keep
    // their character references verbatim because nothing in them changed.
    let mut events = read(concat!(
        r#"<w:p>"#,
        r#"<w:r><w:t>&#1605;</w:t></w:r>"#,
        r#"<w:r><w:t>X</w:t></w:r>"#,
        r#"<w:r><w:t>&#1576;</w:t></w:r>"#,
        r#"</w:p>"#,
    ));
    token::map_runs(&mut events, W_T, |text| text.replace('X', "Y"));

    assert_eq!(
        write(&events),
        concat!(
            r#"<w:p>"#,
            r#"<w:r><w:t>&#1605;</w:t></w:r>"#,
            r#"<w:r><w:t>Y</w:t></w:r>"#,
            r#"<w:r><w:t>&#1576;</w:t></w:r>"#,
            r#"</w:p>"#,
        )
    );
}

#[test]
fn an_offset_into_the_concatenated_runs_finds_the_character_in_its_own_run() {
    // "ab" then RLM (U+200F, three bytes) then "cd": the control is at byte
    // offset 2 of the concatenation, which is offset 0 of the second run.
    let mut events = read(concat!(
        r#"<w:p>"#,
        r#"<w:r><w:t>ab</w:t></w:r>"#,
        "<w:r><w:t>\u{200f}cd</w:t></w:r>",
        r#"</w:p>"#,
    ));
    token::remove_at_offsets(&mut events, W_T, &[2]);

    assert_eq!(
        write(&events),
        r#"<w:p><w:r><w:t>ab</w:t></w:r><w:r><w:t>cd</w:t></w:r></w:p>"#
    );
}

#[test]
fn several_offsets_are_removed_back_to_front_so_none_of_them_shifts() {
    // Two controls in one run: RLM at byte offset 1, and LRM at 7 — three
    // bytes for the RLM plus `bcd` — which is where a scanner counting bytes
    // of the original text reports it.
    let mut events = read("<w:p><w:r><w:t>a\u{200f}bcd\u{200e}e</w:t></w:r></w:p>");
    token::remove_at_offsets(&mut events, W_T, &[1, 7]);

    assert_eq!(write(&events), r#"<w:p><w:r><w:t>abcde</w:t></w:r></w:p>"#);
}

#[test]
fn a_leading_marker_and_the_space_after_it_leave_the_first_run() {
    let mut events = read(r#"<w:p><w:r><w:t>•  بند</w:t></w:r><w:r><w:t> • ثان</w:t></w:r></w:p>"#);
    token::strip_leading_marker(&mut events, W_T, '•');

    // Only the first run: a marker in the middle of the paragraph is text.
    assert_eq!(
        write(&events),
        r#"<w:p><w:r><w:t>بند</w:t></w:r><w:r><w:t> • ثان</w:t></w:r></w:p>"#
    );
}

#[test]
fn a_paragraph_that_does_not_start_with_the_marker_is_untouched() {
    let xml = r#"<w:p><w:r><w:t>بند •</w:t></w:r></w:p>"#;
    let mut events = read(xml);
    token::strip_leading_marker(&mut events, W_T, '•');
    assert_eq!(write(&events), xml);
}

#[test]
fn text_is_read_back_unescaped_and_written_back_escaped() {
    let events = read(r#"<w:p><w:r><w:t>a &amp; &#1605;</w:t></w:r></w:p>"#);
    let ranges = token::text_content_ranges(&events, W_T);
    assert_eq!(
        token::read_runs(&events, &ranges),
        vec!["a & م".to_string()]
    );
    assert_eq!(write(&[token::write_text("a & b")]), "a &amp; b");
}

#[test]
fn a_text_element_of_another_vocabulary_is_not_run_text_here() {
    // The scaffold reads the element it was given and no other: `a:t` in this
    // part is a foreign element, not a Word run.
    let mut events = read(r#"<w:p><a:t>X</a:t><w:r><w:t>X</w:t></w:r></w:p>"#);
    token::map_runs(&mut events, W_T, |text| text.replace('X', "Y"));
    assert_eq!(
        write(&events),
        r#"<w:p><a:t>X</a:t><w:r><w:t>Y</w:t></w:r></w:p>"#
    );
}
