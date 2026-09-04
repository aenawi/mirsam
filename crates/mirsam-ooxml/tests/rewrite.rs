//! One test per `Fix` variant, each asserting the **whole** rewritten part.
//!
//! Full-string equality is deliberate. PLAN M1 1.2 asks that "the diff contains
//! exactly the intended change and nothing else", and the only assertion that
//! actually says that is one where any unintended byte — a normalised quote, a
//! resolved character reference, a moved child — fails the test.

use mirsam_core::Fix;
use mirsam_core::text::{Alignment, Direction};
use mirsam_ooxml::chart::ChartText;
use mirsam_ooxml::rewrite::{Inherited, PartFixes, PartPlan, apply, apply_plan, apply_with};

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

// --------------------------------------------------------- presentation forms

#[test]
fn normalise_presentation_forms_maps_each_form_to_its_logical_letters() {
    // Five contextual forms, one letter each: the word as any keyboard
    // would have stored it.
    assert_rewrite(
        r#"<a:p><a:r><a:t>ﻣﺮﺣﺒﺎ</a:t></a:r></a:p>"#,
        vec![Fix::NormalizePresentationForms],
        r#"<a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn normalise_presentation_forms_leaves_neighbouring_text_alone() {
    // Everything whole-string NFKC would also have changed, beside one form:
    // alef + combining madda as the author typed it, a Latin ligature, a
    // superscript, a word ligature, a byte-order mark. Only the form moves.
    assert_rewrite(
        "<a:p><a:r><a:t>\u{0627}\u{0653} \u{FB01} \u{00B2} \u{FDFA} \u{FEFF} \u{FEF2}</a:t></a:r></a:p>",
        vec![Fix::NormalizePresentationForms],
        "<a:p><a:r><a:t>\u{0627}\u{0653} \u{FB01} \u{00B2} \u{FDFA} \u{FEFF} \u{064A}</a:t></a:r></a:p>",
    );
}

#[test]
fn normalise_presentation_forms_recomposes_hamza() {
    // A lam-alef-with-hamza ligature comes back as lam + U+0623, the
    // precomposed letter, not as lam + alef + combining hamza.
    assert_rewrite(
        "<a:p><a:r><a:t>\u{FEF7}</a:t></a:r></a:p>",
        vec![Fix::NormalizePresentationForms],
        "<a:p><a:r><a:t>\u{0644}\u{0623}</a:t></a:r></a:p>",
    );
}

#[test]
fn a_run_without_a_form_keeps_its_character_references_when_another_is_normalised() {
    assert_rewrite(
        r#"<a:p><a:r><a:t>&#1585;&#1587;&#1605;</a:t></a:r><a:r><a:t>ﻣﺮﺣﺒﺎ</a:t></a:r></a:p>"#,
        vec![Fix::NormalizePresentationForms],
        r#"<a:p><a:r><a:t>&#1585;&#1587;&#1605;</a:t></a:r><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

#[test]
fn controls_are_removed_before_forms_are_normalised_whatever_the_order_given() {
    // Each form is three bytes and the letter it becomes is two, so
    // normalising first would leave the control's offset five bytes past
    // where the mark now sits. "ﻣﺮﺣﺒﺎ" is 15 bytes; the mark is at 15.
    assert_rewrite(
        "<a:p><a:r><a:t>ﻣﺮﺣﺒﺎ\u{200F}</a:t></a:r></a:p>",
        vec![
            Fix::NormalizePresentationForms,
            Fix::RemoveControls(vec![15]),
        ],
        r#"<a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#,
    );
}

// --------------------------------------------------------------------- tables

fn rewrite_table(xml: &str, table: usize, fixes: Vec<Fix>) -> String {
    let mut plan = PartPlan::default();
    plan.tables.insert(table, fixes);
    apply_plan("s.xml", xml, &plan, &Inherited::new()).expect("rewrite failed")
}

const CELL: &str =
    r#"<a:tr><a:tc><a:txBody><a:p><a:r><a:t>المؤشر</a:t></a:r></a:p></a:txBody></a:tc></a:tr>"#;

#[test]
fn set_direction_on_a_table_creates_tblpr_as_its_first_child() {
    // CT_Table is a sequence: tblPr, tblGrid, tr. A tblPr after the grid is
    // a file PowerPoint repairs.
    assert_eq!(
        rewrite_table(
            &format!("<a:tbl><a:tblGrid/>{CELL}</a:tbl>"),
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(r#"<a:tbl><a:tblPr rtl="1"/><a:tblGrid/>{CELL}</a:tbl>"#),
    );
}

#[test]
fn set_direction_on_a_table_edits_an_existing_tblpr_in_place() {
    assert_eq!(
        rewrite_table(
            &format!(
                r#"<a:tbl><a:tblPr firstRow="1" bandRow='1'><a:tableStyleId>x</a:tableStyleId></a:tblPr><a:tblGrid/>{CELL}</a:tbl>"#
            ),
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(
            r#"<a:tbl><a:tblPr firstRow="1" bandRow='1' rtl="1"><a:tableStyleId>x</a:tableStyleId></a:tblPr><a:tblGrid/>{CELL}</a:tbl>"#
        ),
    );
    assert_eq!(
        rewrite_table(
            &format!(r#"<a:tbl><a:tblPr rtl="0"/><a:tblGrid/>{CELL}</a:tbl>"#),
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(r#"<a:tbl><a:tblPr rtl="1"/><a:tblGrid/>{CELL}</a:tbl>"#),
    );
}

#[test]
fn tables_are_numbered_in_document_order_and_the_others_are_untouched() {
    let two = format!("<a:tbl><a:tblGrid/>{CELL}</a:tbl><a:tbl><a:tblGrid/>{CELL}</a:tbl>");
    assert_eq!(
        rewrite_table(&two, 2, vec![Fix::SetDirection(Direction::Rtl)]),
        format!(
            r#"<a:tbl><a:tblGrid/>{CELL}</a:tbl><a:tbl><a:tblPr rtl="1"/><a:tblGrid/>{CELL}</a:tbl>"#
        ),
    );
}

#[test]
fn a_cell_paragraph_and_its_table_are_repaired_together() {
    // The paragraph is the first a:p in the part; the table's tblPr is
    // created after the paragraph edit and must not disturb it.
    let mut plan = PartPlan::default();
    plan.paragraphs
        .insert(1, vec![Fix::SetDirection(Direction::Rtl)]);
    plan.tables
        .insert(1, vec![Fix::SetDirection(Direction::Rtl)]);
    assert_eq!(
        apply_plan(
            "s.xml",
            &format!("<a:tbl><a:tblGrid/>{CELL}</a:tbl>"),
            &plan,
            &Inherited::new()
        )
        .unwrap(),
        r#"<a:tbl><a:tblPr rtl="1"/><a:tblGrid/><a:tr><a:tc><a:txBody><a:p><a:pPr rtl="1"/><a:r><a:t>المؤشر</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl>"#,
    );
}

#[test]
fn only_direction_can_be_set_on_a_table() {
    let mut plan = PartPlan::default();
    plan.tables
        .insert(1, vec![Fix::SetLanguage("ar-SA".into())]);
    let err = apply_plan(
        "s.xml",
        &format!("<a:tbl><a:tblGrid/>{CELL}</a:tbl>"),
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("table"), "{err}");
}

#[test]
fn a_plan_naming_a_table_that_is_not_there_is_an_error() {
    let mut plan = PartPlan::default();
    plan.tables
        .insert(2, vec![Fix::SetDirection(Direction::Rtl)]);
    let err = apply_plan(
        "s.xml",
        &format!("<a:tbl><a:tblGrid/>{CELL}</a:tbl>"),
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("no table 2"), "{err}");
}

// -------------------------------------------------------------------- columns

fn rewrite_columns(xml: &str, body: usize, fixes: Vec<Fix>) -> String {
    let mut plan = PartPlan::default();
    plan.columns.insert(body, fixes);
    apply_plan("s.xml", xml, &plan, &Inherited::new()).expect("rewrite failed")
}

const COLUMN_TEXT: &str = r#"<a:p><a:r><a:t>الفقرة</a:t></a:r></a:p>"#;

#[test]
fn set_direction_on_a_body_adds_rtlcol_and_leaves_the_rest_of_the_tag_alone() {
    assert_eq!(
        rewrite_columns(
            &format!(r#"<p:txBody><a:bodyPr numCol="2" anchor='t'/>{COLUMN_TEXT}</p:txBody>"#),
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(
            r#"<p:txBody><a:bodyPr numCol="2" anchor='t' rtlCol="1"/>{COLUMN_TEXT}</p:txBody>"#
        ),
    );
}

#[test]
fn set_direction_on_a_body_replaces_a_column_direction_already_there() {
    assert_eq!(
        rewrite_columns(
            &format!(r#"<p:txBody><a:bodyPr numCol="2" rtlCol="1"/>{COLUMN_TEXT}</p:txBody>"#),
            1,
            vec![Fix::SetDirection(Direction::Ltr)],
        ),
        format!(r#"<p:txBody><a:bodyPr numCol="2" rtlCol="0"/>{COLUMN_TEXT}</p:txBody>"#),
    );
}

#[test]
fn bodies_are_numbered_in_document_order_including_the_single_column_ones() {
    // The scanner numbers every a:bodyPr, so body 2 here is the columned one
    // even though body 1 produced no unit.
    let two = format!(
        r#"<p:txBody><a:bodyPr/>{COLUMN_TEXT}</p:txBody><p:txBody><a:bodyPr numCol="2"/>{COLUMN_TEXT}</p:txBody>"#
    );
    assert_eq!(
        rewrite_columns(&two, 2, vec![Fix::SetDirection(Direction::Rtl)]),
        format!(
            r#"<p:txBody><a:bodyPr/>{COLUMN_TEXT}</p:txBody><p:txBody><a:bodyPr numCol="2" rtlCol="1"/>{COLUMN_TEXT}</p:txBody>"#
        ),
    );
}

#[test]
fn a_paragraph_and_the_body_it_sits_in_are_repaired_together() {
    let mut plan = PartPlan::default();
    plan.paragraphs
        .insert(1, vec![Fix::SetDirection(Direction::Rtl)]);
    plan.columns
        .insert(1, vec![Fix::SetDirection(Direction::Rtl)]);
    assert_eq!(
        apply_plan(
            "s.xml",
            &format!(r#"<p:txBody><a:bodyPr numCol="2"/>{COLUMN_TEXT}</p:txBody>"#),
            &plan,
            &Inherited::new()
        )
        .unwrap(),
        r#"<p:txBody><a:bodyPr numCol="2" rtlCol="1"/><a:p><a:pPr rtl="1"/><a:r><a:t>الفقرة</a:t></a:r></a:p></p:txBody>"#,
    );
}

#[test]
fn only_direction_can_be_set_on_a_body() {
    let mut plan = PartPlan::default();
    plan.columns
        .insert(1, vec![Fix::SetLanguage("ar-SA".into())]);
    let err = apply_plan(
        "s.xml",
        &format!(r#"<p:txBody><a:bodyPr numCol="2"/>{COLUMN_TEXT}</p:txBody>"#),
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("text body"), "{err}");
}

#[test]
fn a_plan_naming_a_body_that_is_not_there_is_an_error() {
    let mut plan = PartPlan::default();
    plan.columns
        .insert(2, vec![Fix::SetDirection(Direction::Rtl)]);
    let err = apply_plan(
        "s.xml",
        &format!(r#"<p:txBody><a:bodyPr numCol="2"/>{COLUMN_TEXT}</p:txBody>"#),
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("no text body 2"), "{err}");
}

// --------------------------------------------------------- chart containers

fn rewrite_chart_part(xml: &str, kind: ChartText, index: usize, fixes: Vec<Fix>) -> String {
    let mut plan = PartPlan::default();
    plan.chart_text.insert((kind, index), fixes);
    apply_plan("ppt/charts/chart1.xml", xml, &plan, &Inherited::new()).expect("rewrite failed")
}

/// The root a chart part has, declaring both prefixes as every chart an
/// application writes does.
const CHART_HEAD: &str = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">"#;

/// Rewrite a fragment inside a real chart root, and give back the fragment,
/// so an assertion says only what happened to the container.
fn rewrite_chart(xml: &str, kind: ChartText, index: usize, fixes: Vec<Fix>) -> String {
    let part = format!("{CHART_HEAD}{xml}</c:chartSpace>");
    let out = rewrite_chart_part(&part, kind, index, fixes);
    out.strip_prefix(CHART_HEAD)
        .and_then(|s| s.strip_suffix("</c:chartSpace>"))
        .expect("the root was rewritten")
        .to_string()
}

/// The created `c:txPr`, as every test below expects to find it.
const TXPR: &str =
    r#"<c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/><a:endParaRPr/></a:p></c:txPr>"#;

#[test]
fn set_direction_on_an_axis_creates_txpr_in_schema_position() {
    // CT_CatAx is a sequence: c:txPr belongs after c:spPr and before
    // c:crossAx, and an axis carrying it anywhere else is a chart
    // PowerPoint will not draw.
    assert_eq!(
        rewrite_chart(
            r#"<c:catAx><c:axId val="1"/><c:axPos val="b"/><c:crossAx val="2"/></c:catAx>"#,
            ChartText::CategoryAxis,
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(
            r#"<c:catAx><c:axId val="1"/><c:axPos val="b"/>{TXPR}<c:crossAx val="2"/></c:catAx>"#
        ),
    );
}

#[test]
fn set_direction_edits_an_axis_that_already_states_one() {
    assert_eq!(
        rewrite_chart(
            r#"<c:catAx><c:axId val="1"/><c:txPr><a:bodyPr rot='0'/><a:lstStyle/><a:p><a:pPr rtl="0" algn='l'/></a:p></c:txPr></c:catAx>"#,
            ChartText::CategoryAxis,
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        r#"<c:catAx><c:axId val="1"/><c:txPr><a:bodyPr rot='0'/><a:lstStyle/><a:p><a:pPr rtl="1" algn='l'/></a:p></c:txPr></c:catAx>"#,
    );
}

#[test]
fn set_direction_creates_the_paragraph_properties_a_txpr_lacks() {
    assert_eq!(
        rewrite_chart(
            r#"<c:legend><c:legendPos val="r"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="ar-SA"/></a:p></c:txPr></c:legend>"#,
            ChartText::Legend,
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        r#"<c:legend><c:legendPos val="r"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr rtl="1"/><a:endParaRPr lang="ar-SA"/></a:p></c:txPr></c:legend>"#,
    );
}

#[test]
fn a_legends_txpr_lands_after_its_position_and_before_its_extensions() {
    assert_eq!(
        rewrite_chart(
            r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/><c:extLst/></c:legend>"#,
            ChartText::Legend,
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(
            r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/>{TXPR}<c:extLst/></c:legend>"#
        ),
    );
}

#[test]
fn data_labels_take_their_txpr_before_the_flags_that_choose_what_they_show() {
    assert_eq!(
        rewrite_chart(
            r#"<c:dLbls><c:showVal val="0"/><c:showCatName val="1"/></c:dLbls>"#,
            ChartText::DataLabels,
            1,
            vec![Fix::SetDirection(Direction::Rtl)],
        ),
        format!(r#"<c:dLbls>{TXPR}<c:showVal val="0"/><c:showCatName val="1"/></c:dLbls>"#),
    );
}

#[test]
fn containers_are_numbered_per_kind_and_the_others_are_untouched() {
    let two = r#"<c:catAx><c:axId val="1"/></c:catAx><c:catAx><c:axId val="3"/></c:catAx>"#;
    assert_eq!(
        rewrite_chart(
            two,
            ChartText::CategoryAxis,
            2,
            vec![Fix::SetDirection(Direction::Rtl)]
        ),
        format!(
            r#"<c:catAx><c:axId val="1"/></c:catAx><c:catAx><c:axId val="3"/>{TXPR}</c:catAx>"#
        ),
    );
}

#[test]
fn a_chart_that_declares_no_drawingml_prefix_gets_one_on_the_element_created() {
    // Nothing else in such a part uses the `a:` prefix, so a c:txPr written
    // without a declaration would be a document no parser can read.
    let bare = r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:legend><c:legendPos val="r"/></c:legend></c:chartSpace>"#;
    let out = rewrite_chart_part(
        bare,
        ChartText::Legend,
        1,
        vec![Fix::SetDirection(Direction::Rtl)],
    );
    assert!(
        out.contains(r#"<c:txPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">"#),
        "{out}"
    );

    // And when the part already declares it, nothing is added.
    let out = rewrite_chart(
        r#"<c:legend><c:legendPos val="r"/></c:legend>"#,
        ChartText::Legend,
        1,
        vec![Fix::SetDirection(Direction::Rtl)],
    );
    assert_eq!(
        out,
        format!("<c:legend><c:legendPos val=\"r\"/>{TXPR}</c:legend>")
    );
}

#[test]
fn only_direction_can_be_set_on_a_chart_container() {
    let mut plan = PartPlan::default();
    plan.chart_text.insert(
        (ChartText::CategoryAxis, 1),
        vec![Fix::SetLanguage("ar-SA".into())],
    );
    let err = apply_plan(
        "c.xml",
        r#"<c:catAx><c:axId val="1"/></c:catAx>"#,
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("category axis"), "{err}");
}

#[test]
fn a_plan_naming_a_container_that_is_not_there_is_an_error() {
    let mut plan = PartPlan::default();
    plan.chart_text.insert(
        (ChartText::Legend, 1),
        vec![Fix::SetDirection(Direction::Rtl)],
    );
    let err = apply_plan(
        "c.xml",
        r#"<c:catAx><c:axId val="1"/></c:catAx>"#,
        &plan,
        &Inherited::new(),
    )
    .unwrap_err();
    assert!(format!("{err}").contains("no legend 1"), "{err}");
}

// ------------------------------------------------------------------ the rest

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
