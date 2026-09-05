//! WordprocessingML's repair vocabulary, and the `DocumentWriter` port over it.
//!
//! The first half is `rewrite.rs`'s opposite number: one test per `Fix`, each
//! asserting the **whole** rewritten part. Full-string equality is deliberate
//! for the reason stated there — "the diff contains exactly the intended change
//! and nothing else" is only actually asserted by a test that any unintended
//! byte fails.
//!
//! The second half is `writer.rs`'s: a repair planned against a unit id reaches
//! the paragraph it names, through the part it lives in, and nothing else in
//! the package moves.

use mirsam_core::error::Error;
use mirsam_core::text::{Alignment, Direction};
use mirsam_core::{DocumentReader, DocumentWriter, Fix, Repair, UnitId};
use mirsam_ooxml::DocxDocument;
use mirsam_ooxml::word::{Bullets, PartFixes, PartPlan, apply, apply_plan, bullet_list};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PART: &str = "word/document.xml";

fn rewrite(xml: &str, fixes: Vec<Fix>) -> String {
    let mut part = PartFixes::new();
    part.insert(1, fixes);
    apply(PART, xml, &part).expect("rewrite failed")
}

fn assert_rewrite(input: &str, fixes: Vec<Fix>, expected: &str) {
    assert_eq!(rewrite(input, fixes), expected);
}

/// The error a rewrite refused with.
fn refusal(xml: &str, fixes: Vec<Fix>) -> String {
    let mut part = PartFixes::new();
    part.insert(1, fixes);
    match apply(PART, xml, &part) {
        Err(Error::Format(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ------------------------------------------------------------------ direction

#[test]
fn set_direction_creates_a_ppr_when_the_paragraph_has_none() {
    // `w:pPr` is first in CT_P, before any run.
    assert_rewrite(
        r#"<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        vec![Fix::SetDirection(Direction::Rtl)],
        r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
    );
}

#[test]
fn left_to_right_is_written_out_because_an_empty_bidi_element_means_on() {
    // The whole reason `w:val` is always written. `<w:bidi/>` is *true*, so a
    // repair setting a paragraph left-to-right by creating an empty element
    // would write the opposite of what it was asked for.
    assert_rewrite(
        r#"<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>hi</w:t></w:r></w:p>"#,
        vec![Fix::SetDirection(Direction::Ltr)],
        r#"<w:p><w:pPr><w:bidi w:val="0"/></w:pPr><w:r><w:t>hi</w:t></w:r></w:p>"#,
    );
}

#[test]
fn set_direction_lands_in_schema_position_among_the_children_already_there() {
    // CT_PPrBase is a sequence: `w:pStyle` precedes `w:bidi`, `w:bidi`
    // precedes `w:jc`, and the paragraph mark's `w:rPr` comes after all three.
    // An element in the wrong place is a document Word offers to repair.
    assert_rewrite(
        concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="Body"/><w:jc w:val="end"/>"#,
            r#"<w:rPr><w:lang w:bidi="ar-SA"/></w:rPr></w:pPr>"#,
            r#"<w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
        vec![Fix::SetDirection(Direction::Rtl)],
        concat!(
            r#"<w:p><w:pPr><w:pStyle w:val="Body"/><w:bidi w:val="1"/><w:jc w:val="end"/>"#,
            r#"<w:rPr><w:lang w:bidi="ar-SA"/></w:rPr></w:pPr>"#,
            r#"<w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn set_direction_replaces_the_value_and_leaves_neighbouring_attributes_alone() {
    // `w:val='0'` is single-quoted on the neighbour. Rebuilding the tag from
    // parsed attributes would re-quote it — a change the repair never asked for.
    assert_rewrite(
        r#"<w:p><w:pPr><w:bidi w:val='0'/><w:jc w:val='end'/></w:pPr></w:p>"#,
        vec![Fix::SetDirection(Direction::Rtl)],
        r#"<w:p><w:pPr><w:bidi w:val='1'/><w:jc w:val='end'/></w:pPr></w:p>"#,
    );
}

// ------------------------------------------------------------------ alignment

#[test]
fn alignment_is_written_relative_because_word_reads_it_relative() {
    // The claim that makes this rewriter simpler than DrawingML's: `w:jc` is
    // evaluated against the paragraph's own `w:bidi`, so `Start` is `start`
    // whichever way the paragraph runs and there is nothing to lower it
    // against. Two paragraphs, opposite directions, one value.
    for direction in ["0", "1"] {
        assert_rewrite(
            &format!(r#"<w:p><w:pPr><w:bidi w:val="{direction}"/></w:pPr></w:p>"#),
            vec![Fix::SetAlignment(Alignment::Start)],
            &format!(
                r#"<w:p><w:pPr><w:bidi w:val="{direction}"/><w:jc w:val="start"/></w:pPr></w:p>"#
            ),
        );
    }
}

#[test]
fn start_and_end_are_opposite_edges() {
    let paragraph = r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr></w:p>"#;
    assert!(rewrite(paragraph, vec![Fix::SetAlignment(Alignment::Start)]).contains(r#""start""#));
    assert!(rewrite(paragraph, vec![Fix::SetAlignment(Alignment::End)]).contains(r#""end""#));
}

#[test]
fn a_physical_edge_is_refused_rather_than_lowered_onto_the_edge_it_means_today() {
    // Word has no spelling for "the left of the page whatever the direction",
    // which is the refusal the conformance suite states for the reading side.
    // Writing `end` on a right-to-left paragraph would land left today and
    // move the moment somebody changed the direction — a different claim from
    // the one the fix makes.
    for physical in [Alignment::Left, Alignment::Right] {
        let message = refusal(
            r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr></w:p>"#,
            vec![Fix::SetAlignment(physical)],
        );
        assert!(message.contains("physical"), "{message}");
    }
}

#[test]
fn every_alignment_word_can_state_is_one_this_adapter_reads_back() {
    // The two halves have to agree: a value written here that the reader
    // lowered onto something else would make a repair look like it failed.
    for (alignment, expected) in [
        (Alignment::Start, Alignment::Start),
        (Alignment::End, Alignment::End),
        (Alignment::Center, Alignment::Center),
        (Alignment::Justify, Alignment::Justify),
        (Alignment::Distributed, Alignment::Distributed),
    ] {
        let written = rewrite(
            r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
            vec![Fix::SetAlignment(alignment)],
        );
        let units = mirsam_ooxml::docx::scan_xml(PART, &written).expect("scan failed");
        assert_eq!(
            units[0].props.alignment.effective().copied(),
            Some(expected),
            "{alignment:?} in {written}"
        );
    }
}

// ------------------------------------------------------------------- language

#[test]
fn set_language_writes_the_complex_script_slot_of_every_run() {
    assert_rewrite(
        r#"<w:p><w:r><w:t>مرحبا</w:t></w:r><w:r><w:t>ألف</w:t></w:r></w:p>"#,
        vec![Fix::SetLanguage("ar-SA".into())],
        concat!(
            r#"<w:p><w:r><w:rPr><w:lang w:bidi="ar-SA"/></w:rPr><w:t>مرحبا</w:t></w:r>"#,
            r#"<w:r><w:rPr><w:lang w:bidi="ar-SA"/></w:rPr><w:t>ألف</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn set_language_leaves_the_latin_tag_exactly_as_it_was() {
    // `@w:val` is the Latin language and `@w:bidi` the complex-script one.
    // Arabic tagged `en-US` in the first and `ar-SA` in the second is
    // correctly tagged, and this repair has nothing to say about the Latin.
    assert_rewrite(
        r#"<w:p><w:r><w:rPr><w:lang w:val="en-US"/></w:rPr><w:t>مرحبا</w:t></w:r></w:p>"#,
        vec![Fix::SetLanguage("ar-SA".into())],
        concat!(
            r#"<w:p><w:r><w:rPr><w:lang w:val="en-US" w:bidi="ar-SA"/></w:rPr>"#,
            r#"<w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn the_paragraph_mark_is_edited_where_it_exists() {
    assert_rewrite(
        concat!(
            r#"<w:p><w:pPr><w:rPr><w:rFonts w:cs="Dubai"/></w:rPr></w:pPr>"#,
            r#"<w:r><w:rPr/><w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
        vec![Fix::SetLanguage("ar-SA".into())],
        concat!(
            r#"<w:p><w:pPr><w:rPr><w:rFonts w:cs="Dubai"/><w:lang w:bidi="ar-SA"/></w:rPr></w:pPr>"#,
            r#"<w:r><w:rPr><w:lang w:bidi="ar-SA"/></w:rPr><w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn the_paragraph_mark_is_not_created_where_it_does_not_exist() {
    // The mark is the pilcrow's own formatting, and a paragraph that never
    // stated any is not made to — the same restraint the DrawingML rewriter
    // shows an absent `a:endParaRPr`.
    let written = rewrite(
        r#"<w:p><w:pPr><w:jc w:val="end"/></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        vec![Fix::SetLanguage("ar-SA".into())],
    );
    assert_eq!(
        written,
        concat!(
            r#"<w:p><w:pPr><w:jc w:val="end"/></w:pPr>"#,
            r#"<w:r><w:rPr><w:lang w:bidi="ar-SA"/></w:rPr><w:t>مرحبا</w:t></w:r></w:p>"#,
        )
    );
}

// ----------------------------------------------------------------------- font

#[test]
fn set_complex_font_fills_the_slot_beside_the_latin_one() {
    assert_rewrite(
        r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Calibri"/></w:rPr><w:t>مرحبا</w:t></w:r></w:p>"#,
        vec![Fix::SetComplexFont("Dubai".into())],
        concat!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Calibri" w:cs="Dubai"/></w:rPr>"#,
            r#"<w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn set_complex_font_removes_the_theme_reference_it_would_otherwise_contradict() {
    // `@w:cstheme` is what Word renders and `@w:cs` the value it caches beside
    // it. Writing the typeface into the cache and leaving the reference
    // standing would change what the file says and not what a reader sees —
    // a repair that reports success and fixes nothing.
    assert_rewrite(
        concat!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Calibri" w:cstheme="minorBidi"/></w:rPr>"#,
            r#"<w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
        vec![Fix::SetComplexFont("Dubai".into())],
        concat!(
            r#"<w:p><w:r><w:rPr><w:rFonts w:ascii="Calibri" w:cs="Dubai"/></w:rPr>"#,
            r#"<w:t>مرحبا</w:t></w:r></w:p>"#,
        ),
    );
}

#[test]
fn set_complex_font_creates_the_run_properties_in_schema_position() {
    // `w:rPr` is first in CT_R, and `w:rFonts` second in CT_RPr.
    assert_rewrite(
        r#"<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        vec![Fix::SetComplexFont("Dubai".into())],
        r#"<w:p><w:r><w:rPr><w:rFonts w:cs="Dubai"/></w:rPr><w:t>مرحبا</w:t></w:r></w:p>"#,
    );
}

// ----------------------------------------------------------------------- text

#[test]
fn remove_controls_deletes_exactly_the_offsets_it_was_given() {
    // U+202B RIGHT-TO-LEFT EMBEDDING at 0, U+202C POP at the end.
    let text = "\u{202b}مرحبا\u{202c}";
    let last = text.len() - "\u{202c}".len();
    assert_rewrite(
        &format!(r#"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"#),
        vec![Fix::RemoveControls(vec![0, last])],
        r#"<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
    );
}

#[test]
fn normalizing_presentation_forms_leaves_a_run_that_has_none_verbatim() {
    // The second run holds a character reference. A run the mapping does not
    // change is not rewritten at all, so `&#1605;` survives as it was written
    // rather than being resolved into the letter it names.
    assert_rewrite(
        r#"<w:p><w:r><w:t>&#xFE8D;</w:t></w:r><w:r><w:t>&#1605;</w:t></w:r></w:p>"#,
        vec![Fix::NormalizePresentationForms],
        r#"<w:p><w:r><w:t>ا</w:t></w:r><w:r><w:t>&#1605;</w:t></w:r></w:p>"#,
    );
}

#[test]
fn stripping_a_marker_that_leaves_whitespace_marks_the_run_as_significant() {
    // WordprocessingML collapses a `w:t`'s leading and trailing whitespace
    // unless the run says otherwise, so a repair that exposes a space has to
    // say so or it silently deletes one it never meant to touch.
    let mut plan = PartPlan::default();
    plan.paragraphs
        .insert(1, vec![Fix::ConvertLiteralBullet { marker: '•' }]);
    let bullets = Bullets::from([('•', "1".to_string())]);

    let written = apply_plan(
        PART,
        r#"<w:p><w:r><w:t>  • نمو</w:t></w:r></w:p>"#,
        &plan,
        &bullets,
    )
    .expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>"#,
            r#"<w:r><w:t xml:space="preserve">  نمو</w:t></w:r></w:p>"#,
        )
    );
}

#[test]
fn a_run_this_repair_did_not_rewrite_keeps_the_tag_it_had() {
    // The other half of the case above: only runs whose text actually changed
    // are marked, so the second run's tag comes out byte for byte.
    let mut plan = PartPlan::default();
    plan.paragraphs
        .insert(1, vec![Fix::ConvertLiteralBullet { marker: '•' }]);
    let bullets = Bullets::from([('•', "1".to_string())]);

    let written = apply_plan(
        PART,
        r#"<w:p><w:r><w:t>• نمو</w:t></w:r><w:r><w:t> في قطاع</w:t></w:r></w:p>"#,
        &plan,
        &bullets,
    )
    .expect("rewrite failed");

    assert!(written.contains(r#"<w:t> في قطاع</w:t>"#), "{written}");
}

// -------------------------------------------------------------------- bullets

#[test]
fn converting_a_typed_bullet_points_the_paragraph_at_a_list_the_document_defines() {
    // DrawingML answers this with an attribute on the paragraph. Word cannot:
    // a list is a reference into the numbering part, and the indent comes from
    // the definition rather than from anything written here.
    let mut plan = PartPlan::default();
    plan.paragraphs
        .insert(1, vec![Fix::ConvertLiteralBullet { marker: '•' }]);
    let bullets = Bullets::from([('•', "3".to_string())]);

    let written = apply_plan(
        PART,
        r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>• نمو</w:t></w:r></w:p>"#,
        &plan,
        &bullets,
    )
    .expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="3"/></w:numPr>"#,
            r#"<w:bidi w:val="1"/></w:pPr><w:r><w:t>نمو</w:t></w:r></w:p>"#,
        )
    );
}

#[test]
fn a_typed_bullet_with_no_list_to_join_is_refused_rather_than_pointed_at_nothing() {
    let message = refusal(
        r#"<w:p><w:r><w:t>• نمو</w:t></w:r></w:p>"#,
        vec![Fix::ConvertLiteralBullet { marker: '•' }],
    );
    assert!(message.contains("no bulleted list"), "{message}");
}

const NUMBERING: &str = concat!(
    r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    // 0: a numbered list, which is not a bullet.
    r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
    r#"<w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum>"#,
    // 1: a bullet drawing a square.
    r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0">"#,
    r#"<w:numFmt w:val="bullet"/><w:lvlText w:val="▪"/></w:lvl></w:abstractNum>"#,
    // 2: a bullet drawing the marker people actually type.
    r#"<w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0">"#,
    r#"<w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:abstractNum>"#,
    r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#,
    r#"<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>"#,
    r#"<w:num w:numId="3"><w:abstractNumId w:val="2"/></w:num>"#,
    r#"</w:numbering>"#,
);

#[test]
fn the_list_chosen_is_one_that_draws_the_marker_the_author_typed() {
    assert_eq!(
        bullet_list("word/numbering.xml", NUMBERING, '•').unwrap(),
        Some("3".to_string())
    );
}

#[test]
fn any_bullet_list_will_do_when_none_draws_that_marker() {
    // The defect being repaired is the glyph sitting in the text, not which
    // glyph it is, so a marker no definition draws still gets converted — to
    // the first bulleted list in the document, in document order.
    assert_eq!(
        bullet_list("word/numbering.xml", NUMBERING, '-').unwrap(),
        Some("2".to_string())
    );
}

#[test]
fn a_numbered_list_is_not_a_bullet_list() {
    let only_decimal = concat!(
        r#"<w:numbering><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
        r#"<w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>"#,
        r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"#,
    );
    assert_eq!(
        bullet_list("word/numbering.xml", only_decimal, '•').unwrap(),
        None
    );
}

#[test]
fn a_list_that_overrides_its_definition_is_not_offered() {
    // A `w:lvlOverride` can restate the level's format, so a list accepted on
    // the strength of its abstract while the override said otherwise would be
    // a list drawing something else entirely.
    let overridden = concat!(
        r#"<w:numbering><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0">"#,
        r#"<w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl></w:abstractNum>"#,
        r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/>"#,
        r#"<w:lvlOverride w:ilvl="0"><w:lvl w:ilvl="0">"#,
        r#"<w:numFmt w:val="decimal"/></w:lvl></w:lvlOverride></w:num></w:numbering>"#,
    );
    assert_eq!(
        bullet_list("word/numbering.xml", overridden, '•').unwrap(),
        None
    );
}

// ------------------------------------------------------- what is not this unit

#[test]
fn a_paragraph_in_a_text_box_is_not_rewritten_by_the_paragraph_around_it() {
    // A `w:txbxContent` nests whole paragraphs inside a run. The inner one is
    // a unit of its own, with its own text and its own ordinal, and its runs
    // are not the outer paragraph's to edit — an offset the domain computed
    // for the outer paragraph indexes the outer paragraph's runs alone.
    let part = concat!(
        r#"<w:body><w:p><w:r><w:pict><w:txbxContent>"#,
        r#"<w:p><w:r><w:t>&#xFE8D;</w:t></w:r></w:p>"#,
        r#"</w:txbxContent></w:pict></w:r><w:r><w:t>&#xFE8D;</w:t></w:r></w:p></w:body>"#,
    );

    let mut fixes = PartFixes::new();
    fixes.insert(1, vec![Fix::NormalizePresentationForms]);
    let written = apply(PART, part, &fixes).expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:body><w:p><w:r><w:pict><w:txbxContent>"#,
            r#"<w:p><w:r><w:t>&#xFE8D;</w:t></w:r></w:p>"#,
            r#"</w:txbxContent></w:pict></w:r><w:r><w:t>ا</w:t></w:r></w:p></w:body>"#,
        )
    );
}

#[test]
fn a_nested_paragraph_is_repaired_under_its_own_ordinal() {
    // The other side of the case above. The outer paragraph opens first, so it
    // is paragraph 1 and the one inside the box is paragraph 2 — exactly how
    // the reader numbers them.
    let part = concat!(
        r#"<w:body><w:p><w:r><w:pict><w:txbxContent>"#,
        r#"<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
        r#"</w:txbxContent></w:pict></w:r></w:p></w:body>"#,
    );

    let mut fixes = PartFixes::new();
    fixes.insert(2, vec![Fix::SetDirection(Direction::Rtl)]);
    let written = apply(PART, part, &fixes).expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:body><w:p><w:r><w:pict><w:txbxContent>"#,
            r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
            r#"</w:txbxContent></w:pict></w:r></w:p></w:body>"#,
        )
    );
}

#[test]
fn paragraphs_in_a_fallback_are_not_counted_because_the_reader_does_not_read_them() {
    // `mc:Fallback` spells out the same paragraph as the `mc:Choice` beside
    // it, and the reader skips it. A rewriter that counted both would put
    // paragraph 2's repair on the fallback's copy of paragraph 1 — a report
    // and a document that had come apart while both looked right.
    let part = concat!(
        r#"<w:body><mc:AlternateContent>"#,
        r#"<mc:Choice Requires="wps"><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></mc:Choice>"#,
        r#"<mc:Fallback><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></mc:Fallback>"#,
        r#"</mc:AlternateContent><w:p><w:r><w:t>ألف</w:t></w:r></w:p></w:body>"#,
    );

    let mut fixes = PartFixes::new();
    fixes.insert(2, vec![Fix::SetDirection(Direction::Rtl)]);
    let written = apply(PART, part, &fixes).expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:body><mc:AlternateContent>"#,
            r#"<mc:Choice Requires="wps"><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></mc:Choice>"#,
            r#"<mc:Fallback><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></mc:Fallback>"#,
            r#"</mc:AlternateContent>"#,
            r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>ألف</w:t></w:r></w:p></w:body>"#,
        )
    );
}

// --------------------------------------------------------------------- tables

fn table_plan(fixes: Vec<Fix>) -> PartPlan {
    let mut plan = PartPlan::default();
    plan.tables.insert(1, fixes);
    plan
}

#[test]
fn a_table_direction_lands_between_the_style_and_the_width() {
    // CT_TblPrBase is a sequence: `w:tblStyle`, then `w:bidiVisual`, then
    // `w:tblW`.
    let written = apply_plan(
        PART,
        concat!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="Plain"/>"#,
            r#"<w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tr/></w:tbl>"#,
        ),
        &table_plan(vec![Fix::SetDirection(Direction::Rtl)]),
        &Bullets::new(),
    )
    .expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:tbl><w:tblPr><w:tblStyle w:val="Plain"/><w:bidiVisual w:val="1"/>"#,
            r#"<w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tr/></w:tbl>"#,
        )
    );
}

#[test]
fn a_table_with_no_properties_gets_them_before_the_grid() {
    let written = apply_plan(
        PART,
        r#"<w:tbl><w:tblGrid><w:gridCol w:w="100"/></w:tblGrid><w:tr/></w:tbl>"#,
        &table_plan(vec![Fix::SetDirection(Direction::Rtl)]),
        &Bullets::new(),
    )
    .expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:tbl><w:tblPr><w:bidiVisual w:val="1"/></w:tblPr>"#,
            r#"<w:tblGrid><w:gridCol w:w="100"/></w:tblGrid><w:tr/></w:tbl>"#,
        )
    );
}

#[test]
fn anything_but_a_direction_on_a_table_is_refused() {
    // A table's one property this tool reasons about is its column order.
    // Anything else a plan names on one is a mistake upstream, and guessing
    // at it would put a change in the document that no finding asked for.
    let error = apply_plan(
        PART,
        r#"<w:tbl><w:tblPr/><w:tr/></w:tbl>"#,
        &table_plan(vec![Fix::SetLanguage("ar-SA".into())]),
        &Bullets::new(),
    );
    let Err(Error::Format(message)) = error else {
        panic!("a language on a table should be refused");
    };
    assert!(message.contains("only its direction"), "{message}");
}

#[test]
fn a_cells_paragraph_and_its_table_are_both_repaired_in_one_pass() {
    // The composition the ordering exists for: editing the paragraph moves the
    // table's range, so the table's own repair is located again afterwards.
    let mut plan = table_plan(vec![Fix::SetDirection(Direction::Rtl)]);
    plan.paragraphs
        .insert(1, vec![Fix::SetDirection(Direction::Rtl)]);

    let written = apply_plan(
        PART,
        r#"<w:tbl><w:tblPr/><w:tr><w:tc><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        &plan,
        &Bullets::new(),
    )
    .expect("rewrite failed");

    assert_eq!(
        written,
        concat!(
            r#"<w:tbl><w:tblPr><w:bidiVisual w:val="1"/></w:tblPr><w:tr><w:tc>"#,
            r#"<w:p><w:pPr><w:bidi w:val="1"/></w:pPr><w:r><w:t>مرحبا</w:t></w:r></w:p>"#,
            r#"</w:tc></w:tr></w:tbl>"#,
        )
    );
}

// ---------------------------------------------------------------- the part

#[test]
fn an_empty_plan_reproduces_the_part() {
    let part = r#"<?xml version="1.0"?><w:body><w:p><w:r><w:t>مرحبا</w:t></w:r></w:p></w:body>"#;
    assert_eq!(
        apply(PART, part, &PartFixes::new()).expect("rewrite failed"),
        part
    );
}

#[test]
fn a_plan_naming_a_paragraph_the_part_does_not_have_is_refused() {
    let mut fixes = PartFixes::new();
    fixes.insert(9, vec![Fix::SetDirection(Direction::Rtl)]);
    let Err(Error::Format(message)) = apply(PART, r#"<w:p/>"#, &fixes) else {
        panic!("a plan that does not fit the document should be refused");
    };
    assert!(message.contains("no paragraph 9"), "{message}");
}

// ------------------------------------------------------- the DocumentWriter port

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mirsam-word-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every entry's name and raw compressed bytes, in package order, never
/// decompressed.
fn raw_entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let file = std::fs::File::open(path).unwrap();
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut entry = archive.by_index_raw(i).unwrap();
            let mut raw = Vec::new();
            entry.read_to_end(&mut raw).unwrap();
            (entry.name().to_string(), raw)
        })
        .collect()
}

#[test]
fn a_repair_reaches_the_paragraph_it_names_and_no_other_entry_moves() {
    let scratch = Scratch::new("one-paragraph");
    let output = scratch.join("repaired.docx");

    let mut document = DocxDocument::open(fixture("quarterly-review.docx")).unwrap();
    let staged = document
        .apply(&[Repair::new(
            &UnitId(format!("{PART}#p3")),
            Fix::SetDirection(Direction::Rtl),
        )])
        .unwrap();
    assert_eq!(staged, 1);
    document.write(&output).unwrap();

    let before = raw_entries(&fixture("quarterly-review.docx"));
    let after = raw_entries(&output);
    assert_eq!(
        before.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        after.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "the repair added or removed an entry"
    );
    for ((name, was), (_, now)) in before.iter().zip(&after) {
        if name == PART {
            assert_ne!(was, now, "{name} carried the repair and did not change");
        } else {
            assert_eq!(was, now, "{name} is not the part the repair named");
        }
    }

    let mut repaired = DocxDocument::open(&output).unwrap();
    let units = repaired.scan().unwrap();
    let unit = units
        .iter()
        .find(|u| u.id.0 == format!("{PART}#p3"))
        .unwrap();
    assert_eq!(
        unit.props.direction.effective().copied(),
        Some(Direction::Rtl)
    );
}

#[test]
fn a_unit_id_this_adapter_did_not_issue_is_refused_and_stages_nothing() {
    let mut document = DocxDocument::open(fixture("quarterly-review.docx")).unwrap();
    let error = document.apply(&[Repair::new(
        &UnitId("ppt/slides/slide1.xml#cols1".into()),
        Fix::SetDirection(Direction::Rtl),
    )]);
    let Err(Error::Format(message)) = error else {
        panic!("an id from another adapter should be refused");
    };
    assert!(
        message.contains("not a unit this adapter produced"),
        "{message}"
    );
}

#[test]
fn a_physical_edge_is_not_a_repair_this_format_supports() {
    let document = DocxDocument::open(fixture("quarterly-review.docx")).unwrap();
    assert!(!document.supports(&Fix::SetAlignment(Alignment::Left)));
    assert!(!document.supports(&Fix::SetAlignment(Alignment::Right)));
    assert!(document.supports(&Fix::SetAlignment(Alignment::Start)));
}

#[test]
fn a_typed_bullet_is_supported_exactly_where_the_document_defines_a_list() {
    let with_list = DocxDocument::open(fixture("quarterly-review.docx")).unwrap();
    assert!(with_list.supports(&Fix::ConvertLiteralBullet { marker: '•' }));

    let scratch = Scratch::new("no-list");
    let without = scratch.join("plain.docx");
    write_minimal_package(&without);
    let without = DocxDocument::open(&without).unwrap();
    assert!(!without.supports(&Fix::ConvertLiteralBullet { marker: '•' }));
    // And everything else still is: the refusal is about the list, not about
    // the document.
    assert!(without.supports(&Fix::SetDirection(Direction::Rtl)));
}

/// A Word package with one paragraph and no numbering part.
fn write_minimal_package(path: &Path) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();

    zip.start_file("[Content_Types].xml", options).unwrap();
    zip.write_all(
        br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
    )
    .unwrap();
    zip.start_file(PART, options).unwrap();
    zip.write_all(
        concat!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
            r#"<w:body><w:p><w:r><w:t>• نمو</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .as_bytes(),
    )
    .unwrap();
    zip.finish().unwrap();
}
