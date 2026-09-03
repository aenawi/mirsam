//! Adapter conformance: does lowering PPTX into text units preserve the facts
//! the engine needs to reason correctly?

use mirsam_core::{Bullet, Direction, Engine, Resolved, Severity, UnitKind};
use mirsam_ooxml::pptx::scan_xml;

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

fn slide(body: &str) -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/></p:nvSpPr><p:txBody>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
    )
}

#[test]
fn extracts_text_and_shape_name() {
    let xml = slide(r#"<a:bodyPr/><a:p><a:r><a:rPr lang="ar-SA"/><a:t>مرحبا</a:t></a:r></a:p>"#);
    let units = scan_xml("ppt/slides/slide1.xml", &xml).unwrap();

    assert_eq!(units.len(), 1);
    assert_eq!(units[0].text, "مرحبا");
    assert_eq!(units[0].location.container.as_deref(), Some("Title 1"));
    assert_eq!(units[0].location.paragraph, Some(1));
}

#[test]
fn reads_explicit_paragraph_direction() {
    let xml =
        slide(r#"<a:bodyPr/><a:p><a:pPr rtl="1" algn="r"/><a:r><a:t>مرحبا</a:t></a:r></a:p>"#);
    let units = scan_xml("s.xml", &xml).unwrap();
    assert_eq!(units[0].props.direction, Resolved::Explicit(Direction::Rtl));
    assert!(units[0].props.alignment.is_explicit());
}

#[test]
fn body_direction_is_inherited_not_explicit() {
    // The distinction matters: an inherited value is the author's layout
    // choice and must never be reported as missing.
    let xml = slide(r#"<a:bodyPr rtlCol="1"/><a:p><a:r><a:t>مرحبا</a:t></a:r></a:p>"#);
    let units = scan_xml("s.xml", &xml).unwrap();
    assert_eq!(
        units[0].props.direction,
        Resolved::Inherited(Direction::Rtl)
    );
    assert!(!units[0].props.direction.is_explicit());
}

#[test]
fn detects_native_bullets() {
    let xml = slide(
        r#"<a:bodyPr/><a:p><a:pPr rtl="1"><a:buChar char="•"/></a:pPr><a:r><a:t>بند</a:t></a:r></a:p>"#,
    );
    let units = scan_xml("s.xml", &xml).unwrap();
    assert_eq!(units[0].props.bullet, Bullet::Native);
}

#[test]
fn skips_empty_paragraphs() {
    let xml = slide(r#"<a:bodyPr/><a:p><a:endParaRPr lang="ar-SA"/></a:p>"#);
    assert!(scan_xml("s.xml", &xml).unwrap().is_empty());
}

#[test]
fn flags_ltr_declared_on_rtl_text_as_an_error() {
    // The flagship case: direction is declared, and declared wrongly.
    let xml = slide(
        r#"<a:bodyPr/><a:p><a:pPr rtl="0"/><a:r><a:rPr lang="ar-SA"/><a:t>ارتفع الأداء بنسبة 25% في Q4 2026.</a:t></a:r></a:p>"#,
    );
    let units = scan_xml("s.xml", &xml).unwrap();
    let report = Engine::with_default_rules().audit(&units);

    let mismatch = report
        .diagnostics
        .iter()
        .find(|d| d.rule.0 == "direction-mismatch")
        .expect("direction-mismatch should fire");

    assert_eq!(mismatch.severity, Severity::Error);
    assert!(mismatch.fixable);
    // The finding carries proof, not just an assertion.
    assert!(mismatch.evidence.visual_declared.is_some());
    assert_ne!(
        mismatch.evidence.visual_declared,
        mismatch.evidence.visual_expected
    );
}

#[test]
fn correctly_marked_arabic_produces_no_errors() {
    let xml = slide(
        r#"<a:bodyPr rtlCol="1"/><a:p><a:pPr rtl="1" algn="r"/><a:r><a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr><a:t>ارتفع الأداء بنسبة 25% في Q4 2026.</a:t></a:r></a:p>"#,
    );
    let units = scan_xml("s.xml", &xml).unwrap();
    let report = Engine::with_default_rules().audit(&units);

    assert_eq!(
        report.count(Severity::Error),
        0,
        "{:#?}",
        report.diagnostics
    );
    assert!(!report.is_blocking(true), "{:#?}", report.diagnostics);
}

#[test]
fn inherited_alignment_is_not_a_finding() {
    // Regression guard for the false positive that motivated this design:
    // a placeholder inheriting alignment from its layout is correct.
    let xml = slide(
        r#"<a:bodyPr rtlCol="1"/><a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"/><a:t>عنوان</a:t></a:r></a:p>"#,
    );
    let units = scan_xml("s.xml", &xml).unwrap();
    let report = Engine::with_default_rules().audit(&units);

    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule.0 == "alignment-incoherent"),
        "unset alignment must not be reported"
    );
}

// --------------------------------------------------------------------- tables

fn slide_with_table(tblpr: &str) -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Table 4"/></p:nvGraphicFramePr><a:graphic><a:graphicData><a:tbl>{tblpr}<a:tblGrid/><a:tr><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>المؤشر</a:t></a:r></a:p></a:txBody></a:tc><a:tc><a:txBody><a:bodyPr/><a:p><a:r><a:t>الربع الثالث</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#
    )
}

#[test]
fn a_table_is_a_unit_of_its_own_kind_beside_its_cells() {
    let units = scan_xml("s.xml", &slide_with_table("")).unwrap();
    assert_eq!(units.len(), 3, "{units:#?}");
    // The cells first, as paragraphs, then the table that closes after them.
    assert!(units[..2].iter().all(|u| u.kind == UnitKind::Paragraph));
    let table = &units[2];
    assert_eq!(table.kind, UnitKind::Table);
    assert_eq!(table.id.0, "s.xml#tbl1");
    assert_eq!(table.text, "المؤشر\nالربع الثالث");
    assert_eq!(table.props.direction, Resolved::Unset);
    assert_eq!(table.location.container.as_deref(), Some("Table 4"));
    assert_eq!(table.location.paragraph, None);
}

#[test]
fn a_tables_own_direction_is_explicit() {
    let units = scan_xml("s.xml", &slide_with_table(r#"<a:tblPr rtl="1"/>"#)).unwrap();
    assert_eq!(units[2].props.direction, Resolved::Explicit(Direction::Rtl));
    let units = scan_xml("s.xml", &slide_with_table(r#"<a:tblPr firstRow="1"/>"#)).unwrap();
    assert_eq!(units[2].props.direction, Resolved::Unset);
}

#[test]
fn an_arabic_table_with_no_direction_is_reported_and_its_cells_are_not_blamed() {
    let units = scan_xml("s.xml", &slide_with_table("")).unwrap();
    let report = Engine::with_default_rules().audit(&units);
    let on_table: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.unit.0 == "s.xml#tbl1")
        .map(|d| d.rule.0)
        .collect();
    assert_eq!(on_table, ["table-direction"], "{report:#?}");
}
