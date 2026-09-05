//! The `DocumentWriter` port on PPTX.
//!
//! `rewrite.rs` proves each fix lands correctly inside one paragraph and
//! `roundtrip.rs` proves an untouched part survives a package rewrite. This
//! suite covers the join: a repair planned against the scanner's units reaches
//! the paragraph it names, through the part it lives in, and nothing else in
//! the package moves.

use mirsam_core::text::Direction;
use mirsam_core::{
    DocumentReader, DocumentWriter, Engine, Fix, Repair, RepairOptions, Severity, UnitId,
};
use mirsam_ooxml::{Package, PptxDocument};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mirsam-writer-{tag}-{}", std::process::id()));
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

fn unit(part: &str, paragraph: usize) -> UnitId {
    UnitId(format!("{part}#p{paragraph}"))
}

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

/// A one-slide package with the given text body, written with the `zip`
/// crate. Enough for the writer to open and repair; the round-trip guarantee
/// is proven elsewhere against a fixture this crate did not write.
fn scratch_deck(path: &Path, body: &str) {
    let slide = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld {NS}><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Body 1"/></p:nvSpPr><p:txBody>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
    );
    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/></Types>"#;

    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("[Content_Types].xml", options).unwrap();
    zip.write_all(content_types.as_bytes()).unwrap();
    zip.start_file("ppt/slides/slide1.xml", options).unwrap();
    zip.write_all(slide.as_bytes()).unwrap();
    zip.finish().unwrap();
}

// ------------------------------------------------------------------ targeting

#[test]
fn a_repair_lands_on_the_paragraph_it_names_and_touches_nothing_else() {
    let src = fixture("torture.pptx");
    let scratch = Scratch::new("target");
    let out = scratch.join("out.pptx");
    let slide = "ppt/slides/slide1.xml";

    let mut doc = PptxDocument::open(&src).unwrap();
    let repair = Repair::new(&unit(slide, 1), Fix::SetDirection(Direction::Rtl));
    assert_eq!(doc.apply(&[repair]).unwrap(), 1);
    doc.write(&out).unwrap();

    // Every other entry is the same compressed bytes, in the same order.
    let before = raw_entries(&src);
    let after = raw_entries(&out);
    assert_eq!(before.len(), after.len(), "entry count changed");
    for ((name, b), (name_after, a)) in before.iter().zip(&after) {
        assert_eq!(name, name_after, "entry order changed");
        if name != slide {
            assert!(b == a, "{name}: raw bytes changed, and no repair named it");
        }
    }

    // The edited part differs by exactly the attribute the repair set.
    let original = Package::open(&src).unwrap().read_text(slide).unwrap();
    let rewritten = Package::open(&out).unwrap().read_text(slide).unwrap();
    assert_eq!(
        rewritten,
        original.replacen(
            r#"<a:pPr rtl="0" algn='l'/>"#,
            r#"<a:pPr rtl="1" algn='l'/>"#,
            1
        )
    );
}

#[test]
fn repairs_are_grouped_by_part_and_reach_every_part_they_name() {
    let scratch = Scratch::new("parts");
    let out = scratch.join("out.pptx");
    let (slide, notes) = ("ppt/slides/slide1.xml", "ppt/notesSlides/notesSlide1.xml");

    let mut doc = PptxDocument::open(fixture("torture.pptx")).unwrap();
    let plan = [
        Repair::new(&unit(slide, 1), Fix::SetLanguage("ar-SA".into())),
        Repair::new(&unit(notes, 1), Fix::SetLanguage("ar-AE".into())),
        Repair::new(&unit(slide, 3), Fix::SetLanguage("ar-SA".into())),
    ];
    assert_eq!(doc.apply(&plan).unwrap(), 3);
    doc.write(&out).unwrap();

    let pkg = Package::open(&out).unwrap();
    let slide_xml = pkg.read_text(slide).unwrap();
    assert!(
        !slide_xml.contains(r#"lang="en-US""#),
        "a paragraph the plan named kept its old tag:\n{slide_xml}"
    );
    assert!(pkg.read_text(notes).unwrap().contains(r#"lang="ar-AE""#));
}

#[test]
fn repairs_applied_in_two_rounds_compose() {
    // Two `apply` calls on one part: the second must start from what the first
    // staged, not from the package, or the first round is silently lost.
    let scratch = Scratch::new("rounds");
    let out = scratch.join("out.pptx");
    let slide = "ppt/slides/slide1.xml";

    let mut doc = PptxDocument::open(fixture("torture.pptx")).unwrap();
    doc.apply(&[Repair::new(
        &unit(slide, 1),
        Fix::SetDirection(Direction::Rtl),
    )])
    .unwrap();
    doc.apply(&[Repair::new(
        &unit(slide, 1),
        Fix::SetLanguage("ar-SA".into()),
    )])
    .unwrap();
    doc.write(&out).unwrap();

    let xml = Package::open(&out).unwrap().read_text(slide).unwrap();
    assert!(xml.contains(r#"<a:pPr rtl="1" algn='l'/>"#), "{xml}");
    assert!(xml.contains(r#"<a:rPr lang="ar-SA" dirty="0"/>"#), "{xml}");
}

#[test]
fn a_repair_naming_a_unit_the_adapter_did_not_produce_is_an_error() {
    let mut doc = PptxDocument::open(fixture("torture.pptx")).unwrap();

    let foreign = Repair::new(
        &UnitId("slide-1/para-1".into()),
        Fix::SetDirection(Direction::Rtl),
    );
    let err = doc.apply(&[foreign]).unwrap_err();
    assert!(format!("{err}").contains("slide-1/para-1"), "{err}");

    let absent = Repair::new(
        &unit("ppt/slides/slide1.xml", 99),
        Fix::SetDirection(Direction::Rtl),
    );
    let err = doc.apply(&[absent]).unwrap_err();
    assert!(format!("{err}").contains("no paragraph 99"), "{err}");
}

#[test]
fn a_failed_round_stages_nothing() {
    // The first repair is fine; the second names a paragraph that is not there.
    // Neither may be staged, or `write` would produce a half-repaired file
    // after `apply` reported failure.
    let scratch = Scratch::new("atomic");
    let out = scratch.join("out.pptx");
    let slide = "ppt/slides/slide1.xml";

    let mut doc = PptxDocument::open(fixture("torture.pptx")).unwrap();
    let plan = [
        Repair::new(&unit(slide, 1), Fix::SetDirection(Direction::Rtl)),
        Repair::new(
            &unit("ppt/notesSlides/notesSlide1.xml", 99),
            Fix::SetDirection(Direction::Rtl),
        ),
    ];
    assert!(doc.apply(&plan).is_err());
    doc.write(&out).unwrap();

    let before = raw_entries(&fixture("torture.pptx"));
    let after = raw_entries(&out);
    assert!(
        before == after,
        "a repair from the failed round reached the output"
    );
}

#[test]
fn the_writer_expresses_every_fix_variant() {
    // Listed exhaustively rather than iterated, so adding a variant the
    // adapter cannot express fails here before it fails in a user's deck.
    let doc = PptxDocument::open(fixture("torture.pptx")).unwrap();
    for fix in [
        Fix::SetDirection(Direction::Rtl),
        Fix::SetAlignment(mirsam_core::text::Alignment::Start),
        Fix::SetLanguage("ar-SA".into()),
        Fix::SetComplexFont("Dubai".into()),
        Fix::RemoveControls(vec![0]),
        Fix::RemoveTatweel(vec![0]),
        Fix::ConvertLiteralBullet { marker: '•' },
        Fix::NormalizePresentationForms,
    ] {
        assert!(doc.supports(&fix), "{fix}");
    }
}

// ----------------------------------------------------------------- acceptance

#[test]
fn a_planned_repair_audits_clean_and_a_second_pass_is_a_fixed_point() {
    // PLAN M1 1.3 acceptance, at the port level: every fixable finding clears,
    // and repairing the repaired deck does nothing.
    let scratch = Scratch::new("acceptance");
    let (once, twice) = (scratch.join("once.pptx"), scratch.join("twice.pptx"));
    // Every opt-in repair the deck can take, so "every fixable finding
    // clears" means every finding: without `align` the alignment notes would
    // remain, reported and unrepaired by design.
    let engine = Engine::with_options(&RepairOptions {
        convert_bullets: true,
        strip_tatweel: true,
        align: true,
        ..RepairOptions::default()
    });

    let mut doc = PptxDocument::open(fixture("torture.pptx")).unwrap();
    let units = doc.scan().unwrap();
    assert!(
        engine.audit(&units).is_blocking(false),
        "the deck must start broken"
    );
    let plan = engine.plan(&units);
    assert!(!plan.is_empty());
    assert!(plan.iter().all(|r| doc.supports(&r.fix)), "{plan:#?}");
    assert_eq!(doc.apply(&plan).unwrap(), plan.len());
    doc.write(&once).unwrap();

    let mut repaired = PptxDocument::open(&once).unwrap();
    let units = repaired.scan().unwrap();
    let report = engine.audit(&units);
    assert!(
        report.diagnostics.is_empty(),
        "findings survived the repair:\n{:#?}",
        report.diagnostics
    );

    let again = engine.plan(&units);
    assert!(again.is_empty(), "a second pass still had work: {again:#?}");
    assert_eq!(repaired.apply(&again).unwrap(), 0);
    repaired.write(&twice).unwrap();
    assert!(
        std::fs::read(&once).unwrap() == std::fs::read(&twice).unwrap(),
        "a second repair changed the bytes"
    );
}

#[test]
fn a_padded_heading_comes_back_as_the_word_it_was() {
    // PLAN §4.4's repair, end to end: planned from the scanner's units,
    // through the writer, and re-read from disk. The heading is العنوان with
    // five tatweel typed onto the end of it, in a run of its own — which is
    // how it arrives when the author pads text that was already formatted.
    let scratch = Scratch::new("tatweel");
    let (src, out) = (scratch.join("in.pptx"), scratch.join("out.pptx"));
    scratch_deck(
        &src,
        r#"<a:bodyPr rtlCol="1"/><a:p><a:pPr rtl="1" algn="r"/><a:r><a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr><a:t>العنوان</a:t></a:r><a:r><a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr><a:t>&#x640;&#x640;&#x640;&#x640;&#x640;</a:t></a:r></a:p>"#,
    );

    let engine = Engine::with_options(&RepairOptions {
        strip_tatweel: true,
        ..RepairOptions::default()
    });
    let mut doc = PptxDocument::open(&src).unwrap();
    let units = doc.scan().unwrap();
    assert_eq!(units[0].text, "العنوان\u{640}\u{640}\u{640}\u{640}\u{640}");

    let plan = engine.plan(&units);
    assert_eq!(plan.len(), 1, "{plan:#?}");
    assert_eq!(doc.apply(&plan).unwrap(), 1);
    doc.write(&out).unwrap();

    let mut repaired = PptxDocument::open(&out).unwrap();
    let units = repaired.scan().unwrap();
    assert_eq!(units[0].text, "العنوان", "the padding survived the repair");
    assert!(engine.audit(&units).diagnostics.is_empty());
}

#[test]
fn tatweel_the_engine_did_not_call_padding_is_never_written_out_of_a_deck() {
    // The repair's own limit, asserted where it would actually bite: a fatha
    // written on a tatweel is the character doing its job, and a repair that
    // deleted it would drop the mark onto whatever came before.
    let scratch = Scratch::new("tatweel-kept");
    let src = scratch.join("in.pptx");
    scratch_deck(
        &src,
        r#"<a:bodyPr rtlCol="1"/><a:p><a:pPr rtl="1" algn="r"/><a:r><a:rPr lang="ar-SA"><a:cs typeface="Dubai"/></a:rPr><a:t>&#x640;&#x64E; الفتحة</a:t></a:r></a:p>"#,
    );

    let engine = Engine::with_options(&RepairOptions {
        strip_tatweel: true,
        ..RepairOptions::default()
    });
    let mut doc = PptxDocument::open(&src).unwrap();
    let units = doc.scan().unwrap();
    assert!(engine.audit(&units).diagnostics.is_empty());
    assert!(engine.plan(&units).is_empty());
}

// ------------------------------------------------------------------ lowering

#[test]
fn a_direction_relative_repair_lowers_against_the_inherited_direction() {
    // The paragraph is left-aligned inside a right-to-left body and declares
    // no direction of its own. `Start` must become the right edge, which only
    // works if the writer tells the rewriter what the paragraph inherits.
    let scratch = Scratch::new("inherit");
    let (src, out) = (scratch.join("in.pptx"), scratch.join("out.pptx"));
    scratch_deck(
        &src,
        r#"<a:bodyPr rtlCol="1"/><a:p><a:pPr algn="l"/><a:r><a:rPr lang="ar-SA"/><a:t>ارتفع الأداء</a:t></a:r></a:p>"#,
    );

    let engine = Engine::with_default_rules();
    let mut doc = PptxDocument::open(&src).unwrap();
    let units = doc.scan().unwrap();
    let plan = engine.plan(&units);
    assert!(
        plan.iter().any(|r| matches!(r.fix, Fix::SetAlignment(_))),
        "{plan:#?}"
    );
    assert!(
        !plan.iter().any(|r| matches!(r.fix, Fix::SetDirection(_))),
        "an inherited direction must not be repaired: {plan:#?}"
    );
    doc.apply(&plan).unwrap();
    doc.write(&out).unwrap();

    let xml = Package::open(&out)
        .unwrap()
        .read_text("ppt/slides/slide1.xml")
        .unwrap();
    assert!(xml.contains(r#"<a:pPr algn="r"/>"#), "{xml}");

    let mut repaired = PptxDocument::open(&out).unwrap();
    let report = engine.audit(&repaired.scan().unwrap());
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule.0 == "alignment-incoherent"),
        "{:#?}",
        report.diagnostics
    );
}

#[test]
fn a_chosen_typeface_fills_the_complex_script_slot() {
    let scratch = Scratch::new("font");
    let (src, out) = (scratch.join("in.pptx"), scratch.join("out.pptx"));
    scratch_deck(
        &src,
        r#"<a:bodyPr/><a:p><a:pPr rtl="1"/><a:r><a:rPr lang="ar-SA"><a:latin typeface="Calibri"/></a:rPr><a:t>مرحبا</a:t></a:r></a:p>"#,
    );

    // Without a typeface the finding is reported and nothing is proposed.
    let unchosen = Engine::with_default_rules();
    let mut doc = PptxDocument::open(&src).unwrap();
    let units = doc.scan().unwrap();
    let report = unchosen.audit(&units);
    let finding = report
        .diagnostics
        .iter()
        .find(|d| d.rule.0 == "complex-font-missing")
        .expect("complex-font-missing should fire");
    assert!(!finding.fixable);
    assert!(
        unchosen.plan(&units).is_empty(),
        "{:#?}",
        unchosen.plan(&units)
    );

    // With one, it is repaired.
    let chosen = Engine::with_options(&RepairOptions {
        complex_font: Some("Dubai".into()),
        ..RepairOptions::default()
    });
    let plan = chosen.plan(&units);
    assert_eq!(plan.len(), 1, "{plan:#?}");
    doc.apply(&plan).unwrap();
    doc.write(&out).unwrap();

    let xml = Package::open(&out)
        .unwrap()
        .read_text("ppt/slides/slide1.xml")
        .unwrap();
    assert!(
        xml.contains(r#"<a:latin typeface="Calibri"/><a:cs typeface="Dubai"/>"#),
        "{xml}"
    );
    let mut repaired = PptxDocument::open(&out).unwrap();
    assert_eq!(
        chosen
            .audit(&repaired.scan().unwrap())
            .count(Severity::Warning),
        0
    );
}
