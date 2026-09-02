//! Adapter conformance: does lowering PPTX into text units preserve the facts
//! the engine needs to reason correctly?

use mirsam_core::{Bullet, Direction, Engine, Resolved, Severity};
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
