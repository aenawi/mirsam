//! One test per `Fix` variant, each asserting the **whole** rewritten part.
//!
//! Full-string equality is deliberate. PLAN M1 1.2 asks that "the diff contains
//! exactly the intended change and nothing else", and the only assertion that
//! actually says that is one where any unintended byte — a normalised quote, a
//! resolved character reference, a moved child — fails the test.

use mirsam_core::Fix;
use mirsam_core::text::{Alignment, Direction};
use mirsam_ooxml::rewrite::{Inherited, PartFixes, apply, apply_with};

fn rewrite(xml: &str, fixes: Vec<Fix>) -> String {
    let mut part = PartFixes::new();
    part.insert(1, fixes);
    apply("s.xml", xml, &part).expect("rewrite failed")
}

fn assert_rewrite(input: &str, fixes: Vec<Fix>, expected: &str) {
    assert_eq!(rewrite(input, fixes), expected);
}

// ------------------------------------------------------------------ direction

#[test]
fn set_direction_creates_a_ppr_when_the_paragraph_has_none() {
    // a:pPr is first in CT_TextParagraph, before any run.
    assert_rewrite(
        r#"<a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetDirection(Direction::Rtl)],
        r#"<a:p><a:pPr rtl="1"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn set_direction_replaces_the_value_and_leaves_neighbouring_attributes_alone() {
    // `algn='l'` is single-quoted. Rebuilding the tag from parsed attributes
    // would re-quote it — a change the repair never asked for.
    assert_rewrite(
        r#"<a:p><a:pPr rtl="0" algn='l'/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetDirection(Direction::Rtl)],
        r#"<a:p><a:pPr rtl="1" algn='l'/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

// ------------------------------------------------------------------ alignment

#[test]
fn set_alignment_lowers_start_onto_the_reading_side() {
    // `Start` is direction-relative: the right edge in RTL. Applied together
    // with the direction it is being aligned against.
    assert_rewrite(
        r#"<a:p><a:pPr rtl="0" algn='l'/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![
            Fix::SetDirection(Direction::Rtl),
            Fix::SetAlignment(Alignment::Start),
        ],
        r#"<a:p><a:pPr rtl="1" algn='r'/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn set_alignment_reads_the_direction_already_on_the_paragraph() {
    assert_rewrite(
        r#"<a:p><a:pPr rtl="1"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetAlignment(Alignment::Start)],
        r#"<a:p><a:pPr rtl="1" algn="r"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn start_and_end_are_opposite_edges() {
    // Guards the lowering itself: if these ever agree, the direction-relative
    // distinction has been lost.
    let ltr = r#"<a:p><a:pPr rtl="0"/><a:r><a:t>hi</a:t></a:r></a:p>"#;
    assert!(rewrite(ltr, vec![Fix::SetAlignment(Alignment::Start)]).contains(r#"algn="l""#));
    assert!(rewrite(ltr, vec![Fix::SetAlignment(Alignment::End)]).contains(r#"algn="r""#));

    let rtl = r#"<a:p><a:pPr rtl="1"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#;
    assert!(rewrite(rtl, vec![Fix::SetAlignment(Alignment::Start)]).contains(r#"algn="r""#));
    assert!(rewrite(rtl, vec![Fix::SetAlignment(Alignment::End)]).contains(r#"algn="l""#));
}

#[test]
fn set_alignment_lowers_against_an_inherited_direction() {
    // The paragraph declares no direction; its body is right-to-left, which
    // the scanner reports as inherited. Lowering `Start` against the paragraph
    // alone would pick the left edge and reproduce the defect being repaired.
    let xml = r#"<a:p><a:pPr algn="l"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#;
    let mut part = PartFixes::new();
    part.insert(1, vec![Fix::SetAlignment(Alignment::Start)]);
    let mut inherited = Inherited::new();
    inherited.insert(1, Direction::Rtl);

    assert_eq!(
        apply_with("s.xml", xml, &part, &inherited).unwrap(),
        r#"<a:p><a:pPr algn="r"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
    // And without the hint, the rewriter has nothing to go on but the
    // paragraph: this is the case the adapter exists to prevent.
    assert_eq!(
        apply("s.xml", xml, &part).unwrap(),
        r#"<a:p><a:pPr algn="l"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn the_paragraphs_own_direction_outranks_an_inherited_one() {
    let xml = r#"<a:p><a:pPr rtl="0" algn="r"/><a:r><a:t>hi</a:t></a:r></a:p>"#;
    let mut part = PartFixes::new();
    part.insert(1, vec![Fix::SetAlignment(Alignment::Start)]);
    let mut inherited = Inherited::new();
    inherited.insert(1, Direction::Rtl);

    assert_eq!(
        apply_with("s.xml", xml, &part, &inherited).unwrap(),
        r#"<a:p><a:pPr rtl="0" algn="l"/><a:r><a:t>hi</a:t></a:r></a:p>"#,
    );
}

// ------------------------------------------------------------------- language

#[test]
fn set_language_rewrites_every_run_and_the_paragraph_default() {
    assert_rewrite(
        r#"<a:p><a:pPr><a:defRPr lang="en-US"/></a:pPr><a:r><a:rPr lang="en-US" b="1"/><a:t>مرحبا</a:t></a:r><a:r><a:rPr lang="en-US"/><a:t>سلام</a:t></a:r></a:p>"#,
        vec![Fix::SetLanguage("ar-SA".into())],
        r#"<a:p><a:pPr><a:defRPr lang="ar-SA"/></a:pPr><a:r><a:rPr lang="ar-SA" b="1"/><a:t>مرحبا</a:t></a:r><a:r><a:rPr lang="ar-SA"/><a:t>سلام</a:t></a:r></a:p>"#,
    );
}

#[test]
fn set_language_creates_an_rpr_for_a_run_that_has_none() {
    // a:rPr precedes a:t in CT_RegularTextRun.
    assert_rewrite(
        r#"<a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetLanguage("ar-SA".into())],
        r#"<a:p><a:r><a:rPr lang="ar-SA"/><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

// ----------------------------------------------------------------------- font

#[test]
fn set_complex_font_inserts_a_cs_in_schema_position() {
    // CT_TextCharacterProperties is an xsd:sequence: a:cs sits after a:latin
    // and a:ea, before a:sym. A correct element in the wrong position is a file
    // PowerPoint refuses to open.
    assert_rewrite(
        r#"<a:p><a:r><a:rPr lang="ar-SA"><a:latin typeface="Calibri"/><a:sym typeface="Wingdings"/></a:rPr><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetComplexFont("Dubai".into())],
        r#"<a:p><a:r><a:rPr lang="ar-SA"><a:latin typeface="Calibri"/><a:cs typeface="Dubai"/><a:sym typeface="Wingdings"/></a:rPr><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn set_complex_font_replaces_an_existing_cs() {
    assert_rewrite(
        r#"<a:p><a:r><a:rPr><a:cs typeface="Arial"/></a:rPr><a:t>مرحبا</a:t></a:r></a:p>"#,
        vec![Fix::SetComplexFont("Dubai".into())],
        r#"<a:p><a:r><a:rPr><a:cs typeface="Dubai"/></a:rPr><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

// --------------------------------------------------------------------- bullet

#[test]
fn convert_literal_bullet_strips_the_glyph_and_adds_a_native_list() {
    assert_rewrite(
        r#"<a:p><a:pPr/><a:r><a:rPr lang="ar-SA"/><a:t>• بند أول</a:t></a:r></a:p>"#,
        vec![Fix::ConvertLiteralBullet { marker: '•' }],
        r#"<a:p><a:pPr marR="342900" indent="-342900"><a:buChar char="•"/></a:pPr><a:r><a:rPr lang="ar-SA"/><a:t>بند أول</a:t></a:r></a:p>"#,
    );
}

#[test]
fn convert_literal_bullet_places_buchar_after_bufont() {
    // a:buFont precedes the a:buNone|a:buAutoNum|a:buChar choice.
    assert_rewrite(
        r#"<a:p><a:pPr><a:buFont typeface="Arial"/><a:tabLst/></a:pPr><a:r><a:t>• بند</a:t></a:r></a:p>"#,
        vec![Fix::ConvertLiteralBullet { marker: '•' }],
        r#"<a:p><a:pPr marR="342900" indent="-342900"><a:buFont typeface="Arial"/><a:buChar char="•"/><a:tabLst/></a:pPr><a:r><a:t>بند</a:t></a:r></a:p>"#,
    );
}

// ------------------------------------------------------------------- controls

#[test]
fn remove_controls_deletes_the_control_and_keeps_the_text() {
    assert_rewrite(
        "<a:p><a:r><a:t>بند أول\u{200F}</a:t></a:r></a:p>",
        vec![Fix::RemoveControls(vec![13])],
        r#"<a:p><a:r><a:t>بند أول</a:t></a:r></a:p>"#,
    );
}

#[test]
fn remove_controls_reaches_a_control_written_as_a_character_reference() {
    // quick-xml reports `&#x200F;` as its own event, separate from the text
    // around it. Treating only Event::Text as content would miss this entirely.
    assert_rewrite(
        r#"<a:p><a:r><a:t>بند أول&#x200F;</a:t></a:r></a:p>"#,
        vec![Fix::RemoveControls(vec![13])],
        r#"<a:p><a:r><a:t>بند أول</a:t></a:r></a:p>"#,
    );
}

#[test]
fn a_run_with_no_repair_keeps_its_character_references_verbatim() {
    // The whole point: resolving `&#1585;` in an untouched run would be a
    // change nobody asked for, even though the text is unaffected.
    assert_rewrite(
        "<a:p><a:r><a:t>&#1585;&#1587;&#1605;</a:t></a:r><a:r><a:t>بند\u{200F}</a:t></a:r></a:p>",
        // "رسم" is 6 bytes, "بند" another 6, so the control sits at 12.
        vec![Fix::RemoveControls(vec![12])],
        r#"<a:p><a:r><a:t>&#1585;&#1587;&#1605;</a:t></a:r><a:r><a:t>بند</a:t></a:r></a:p>"#,
    );
}

#[test]
fn controls_are_removed_before_the_marker_is_stripped_whatever_the_order_given() {
    // The offsets in `RemoveControls` index the text as scanned. Stripping
    // "• " first would shift the mark four bytes left of where the offset
    // says it is, and the removal would miss. The rewriter must not depend on
    // the order the planner happened to emit.
    //
    // "• " is 4 bytes, "بند أول" 13, so the mark sits at 17.
    assert_rewrite(
        "<a:p><a:r><a:t>• بند أول\u{200F}</a:t></a:r></a:p>",
        vec![
            Fix::ConvertLiteralBullet { marker: '•' },
            Fix::RemoveControls(vec![17]),
        ],
        r#"<a:p><a:pPr marR="342900" indent="-342900"><a:buChar char="•"/></a:pPr><a:r><a:t>بند أول</a:t></a:r></a:p>"#,
    );
}

// ------------------------------------------------------------------ the rest

#[test]
fn normalising_presentation_forms_reports_that_it_is_not_implemented() {
    // Silently succeeding would let `repair` claim it fixed something it did
    // not. Reporting honestly is the whole discipline of this project.
    let mut part = PartFixes::new();
    part.insert(1, vec![Fix::NormalizePresentationForms]);
    let err = apply("s.xml", r#"<a:p><a:r><a:t>ﻣﺮﺣﺒﺎ</a:t></a:r></a:p>"#, &part).unwrap_err();
    assert!(format!("{err}").contains("NFKC"), "unhelpful: {err}");
}

#[test]
fn paragraphs_are_numbered_as_the_scanner_numbers_them() {
    // The scanner counts every a:p, including ones that produce no unit. If the
    // rewriter counted differently a repair would land on the wrong paragraph.
    let xml = r#"<a:p><a:endParaRPr/></a:p><a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#;
    let mut part = PartFixes::new();
    part.insert(2, vec![Fix::SetDirection(Direction::Rtl)]);
    assert_eq!(
        apply("s.xml", xml, &part).unwrap(),
        r#"<a:p><a:endParaRPr/></a:p><a:p><a:pPr rtl="1"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn a_repair_naming_a_paragraph_that_is_not_there_is_an_error() {
    let mut part = PartFixes::new();
    part.insert(9, vec![Fix::SetDirection(Direction::Rtl)]);
    let err = apply("s.xml", r#"<a:p><a:r><a:t>م</a:t></a:r></a:p>"#, &part).unwrap_err();
    assert!(format!("{err}").contains("no paragraph 9"), "{err}");
}

#[test]
fn several_repairs_on_one_paragraph_compose() {
    assert_rewrite(
        r#"<a:p><a:pPr rtl="0" algn='l'/><a:r><a:rPr lang="en-US"/><a:t>• بند أول</a:t></a:r></a:p>"#,
        vec![
            Fix::SetDirection(Direction::Rtl),
            Fix::SetAlignment(Alignment::Start),
            Fix::SetLanguage("ar-SA".into()),
            Fix::ConvertLiteralBullet { marker: '•' },
        ],
        r#"<a:p><a:pPr rtl="1" algn='r' marR="342900" indent="-342900"><a:buChar char="•"/></a:pPr><a:r><a:rPr lang="ar-SA"/><a:t>بند أول</a:t></a:r></a:p>"#,
    );
}
