//! The DOCX reader (PLAN M3 3.2, and its tables — 3.4).
//!
//! Every case is WordprocessingML lowered onto the shared text model, and the
//! assertions are on `TextUnit` rather than on XML: what this file is asking is
//! whether Word's vocabulary reaches the rules as the same shape PowerPoint's
//! does. Nothing here reaches into `mirsam-core`; a case that needed a core
//! change would be PLAN §3.5's answer, not this file's.

use mirsam_core::DocumentReader;
use mirsam_core::text::TextUnit;
use mirsam_core::{Alignment, Bullet, Direction, Engine, Resolved, Severity, UnitKind};
use mirsam_ooxml::DocxDocument;
use mirsam_ooxml::docx::scan_xml;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const PART: &str = "word/document.xml";

const NS: &str = concat!(
    r#" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#,
    r#" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006""#,
);

/// A `word/document.xml` around whatever body a case needs.
fn document(body: &str) -> String {
    format!("<w:document{NS}><w:body>{body}</w:body></w:document>")
}

/// One paragraph: its properties, and one run of Arabic.
fn paragraph(p_pr: &str, text: &str) -> String {
    format!("<w:p>{p_pr}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

/// Every rule id the default engine reports on these units.
fn findings(units: &[TextUnit]) -> Vec<String> {
    Engine::with_default_rules()
        .audit(units)
        .diagnostics
        .iter()
        .map(|d| d.rule.0.to_string())
        .collect()
}

/// The units of a document whose single paragraph has the given properties.
fn scan_one(p_pr: &str, text: &str) -> TextUnit {
    let xml = document(&paragraph(p_pr, text));
    let mut units = scan_xml(PART, &xml).expect("the part did not parse");
    assert_eq!(units.len(), 1, "expected exactly one unit from {xml}");
    units.remove(0)
}

const ARABIC: &str = "مرحبا بالعالم";

// ------------------------------------------------------------------ the shape

#[test]
fn a_paragraph_becomes_one_unit_addressed_by_part_and_ordinal() {
    let unit = scan_one("", ARABIC);
    assert_eq!(unit.id.0, "word/document.xml#p1");
    assert_eq!(unit.kind, UnitKind::Paragraph);
    assert_eq!(unit.text, ARABIC);
    assert_eq!(unit.location.part, PART);
    assert_eq!(unit.location.paragraph, Some(1));
    // Word names no enclosing shape for body text, so there is nothing
    // truthful to put here. A table cell is the one thing it does name.
    assert_eq!(unit.location.container, None);
}

#[test]
fn text_is_joined_across_runs_and_character_references_are_resolved() {
    // `&#1605;` is how Word routinely writes Arabic. A reader that drops these
    // empties the run, and an empty run is not reported at all.
    let xml = document(concat!(
        "<w:p>",
        "<w:r><w:t xml:space=\"preserve\">&#1605;&#1585;</w:t></w:r>",
        "<w:r><w:t>&#1581;&#1576;&#1575;</w:t></w:r>",
        "</w:p>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    assert_eq!(units[0].text, "مرحبا");
}

#[test]
fn an_empty_paragraph_produces_no_unit_but_still_takes_its_ordinal() {
    // The ordinal is an address. A numbering that skipped the paragraphs with
    // nothing to say would move every id after the first blank line anyone
    // pressed Enter on.
    let xml = document(&format!(
        "{}{}{}",
        "<w:p/>",
        paragraph("", "   "),
        paragraph("", ARABIC),
    ));
    let units = scan_xml(PART, &xml).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].id.0, "word/document.xml#p3");
}

#[test]
fn a_drawingml_paragraph_is_not_a_word_paragraph() {
    // The mirror of `token.rs`'s claim in the other direction: a reader that
    // had learned `a:p` and `a:t` would read this, and it is not Word's text.
    let xml = document(r#"<a:p xmlns:a="ns/a"><a:r><a:t>مرحبا</a:t></a:r></a:p>"#);
    assert!(scan_xml(PART, &xml).unwrap().is_empty());
}

// -------------------------------------------------------------------- w:bidi

#[test]
fn a_bidi_element_with_no_value_is_on() {
    // `<w:bidi/>` is the form Word writes; reading a missing `w:val` as false
    // would report every correctly-marked Arabic paragraph in Word.
    let unit = scan_one("<w:pPr><w:bidi/></w:pPr>", ARABIC);
    assert_eq!(unit.props.direction, Resolved::Explicit(Direction::Rtl));
    let reported = findings(&[unit]);
    assert!(
        !reported.iter().any(|r| r.starts_with("direction-")),
        "{reported:?}"
    );
}

#[test]
fn bidi_reads_every_on_off_spelling() {
    for on in ["1", "true", "on"] {
        let p_pr = format!(r#"<w:pPr><w:bidi w:val="{on}"/></w:pPr>"#);
        assert_eq!(
            scan_one(&p_pr, ARABIC).props.direction,
            Resolved::Explicit(Direction::Rtl),
            "w:val={on} should be right-to-left"
        );
    }
    for off in ["0", "false", "off"] {
        let p_pr = format!(r#"<w:pPr><w:bidi w:val="{off}"/></w:pPr>"#);
        assert_eq!(
            scan_one(&p_pr, ARABIC).props.direction,
            Resolved::Explicit(Direction::Ltr),
            "w:val={off} should be left-to-right"
        );
    }
}

#[test]
fn arabic_declared_left_to_right_is_the_flagship_error() {
    let unit = scan_one(
        r#"<w:pPr><w:bidi w:val="0"/></w:pPr>"#,
        "ارتفع الأداء بنسبة 25% في Q4 2026.",
    );
    let report = Engine::with_default_rules().audit(&[unit]);
    let mismatch = report
        .diagnostics
        .iter()
        .find(|d| d.rule.0 == "direction-mismatch")
        .expect("direction-mismatch should fire");
    assert_eq!(mismatch.severity, Severity::Error);
    // The finding carries proof, not just an assertion.
    assert_ne!(
        mismatch.evidence.visual_declared,
        mismatch.evidence.visual_expected
    );
}

#[test]
fn arabic_with_no_bidi_of_its_own_is_unset_not_guessed() {
    // No style chain is read yet (§3.3), so an unstated property is honestly
    // absent. Reporting it as inherited would name a source nothing consulted.
    let unit = scan_one("<w:pPr><w:jc w:val=\"center\"/></w:pPr>", ARABIC);
    assert_eq!(unit.props.direction, Resolved::Unset);
    assert!(findings(&[unit]).contains(&"direction-unset".to_string()));
}

#[test]
fn a_sections_own_direction_is_not_the_paragraphs() {
    // The final `w:sectPr` lives inside a paragraph's `w:pPr`. Reading its
    // `w:bidi` as that paragraph's would put a right-to-left declaration on a
    // paragraph nobody marked.
    let unit = scan_one(
        r#"<w:pPr><w:sectPr><w:bidi/><w:jc w:val="center"/></w:sectPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.direction, Resolved::Unset);
    assert_eq!(unit.props.alignment, Resolved::Unset);
}

// ---------------------------------------------------------------------- w:jc

#[test]
fn word_alignment_is_direction_relative_so_left_is_the_start_edge() {
    // MS-OE376 Part 4 §2.3.1.13 note b: "Word evaluates the value of this
    // attribute based on the value of the bidi element: Left is the right side
    // of a right-to-left paragraph". So `left` is `start`, and a Word author
    // cannot write the hard left `alignment-incoherent` reports — mapping it
    // to `Alignment::Left` would manufacture that finding on every
    // left-aligned Arabic paragraph in Word.
    let unit = scan_one(r#"<w:pPr><w:jc w:val="left"/></w:pPr>"#, ARABIC);
    assert_eq!(unit.props.alignment, Resolved::Explicit(Alignment::Start));
    let reported = findings(&[unit]);
    assert!(
        !reported.contains(&"alignment-incoherent".to_string()),
        "{reported:?}"
    );
}

#[test]
fn every_jc_value_this_adapter_reads_is_coherent_with_right_to_left() {
    let cases = [
        ("start", Alignment::Start),
        ("left", Alignment::Start),
        ("numTab", Alignment::Start),
        ("end", Alignment::End),
        ("right", Alignment::End),
        ("center", Alignment::Center),
        ("both", Alignment::Justify),
        ("mediumKashida", Alignment::Justify),
        ("highKashida", Alignment::Justify),
        ("lowKashida", Alignment::Justify),
        ("distribute", Alignment::Distributed),
        ("thaiDistribute", Alignment::Distributed),
    ];
    for (value, expected) in cases {
        let p_pr = format!(r#"<w:pPr><w:jc w:val="{value}"/></w:pPr>"#);
        let unit = scan_one(&p_pr, ARABIC);
        assert_eq!(
            unit.props.alignment,
            Resolved::Explicit(expected),
            "w:jc w:val={value}"
        );
        assert!(
            !findings(&[unit]).contains(&"alignment-incoherent".to_string()),
            "w:jc w:val={value} was reported as a hard left, which Word has no spelling for"
        );
    }
}

#[test]
fn an_alignment_this_adapter_does_not_understand_stays_unset() {
    // Better an honest absence than a guess: `alignment-unset` is a note and
    // never blocks, while a wrong value would be argued about in a report.
    let unit = scan_one(r#"<w:pPr><w:jc w:val="somethingNew"/></w:pPr>"#, ARABIC);
    assert_eq!(unit.props.alignment, Resolved::Unset);
}

// -------------------------------------------------------------------- w:lang

#[test]
fn the_language_read_is_the_complex_script_one() {
    // `@w:val` is the Latin language and `@w:bidi` the complex-script one.
    // This paragraph is correctly tagged, and a reader that took `@w:val`
    // would report it as Arabic tagged `en-US`.
    let unit = scan_one(
        r#"<w:pPr><w:rPr><w:lang w:val="en-US" w:bidi="ar-SA"/></w:rPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.language, Resolved::Explicit("ar-SA".into()));
}

#[test]
fn a_latin_language_alone_leaves_the_complex_script_slot_empty() {
    let unit = scan_one(
        r#"<w:pPr><w:rPr><w:lang w:val="en-US"/></w:rPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.language, Resolved::Unset);
}

#[test]
fn the_language_comes_from_a_run_when_the_paragraph_mark_states_none() {
    let xml = document(concat!(
        "<w:p><w:r><w:rPr><w:lang w:bidi=\"ar-AE\"/></w:rPr>",
        "<w:t>مرحبا</w:t></w:r></w:p>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    assert_eq!(units[0].props.language, Resolved::Explicit("ar-AE".into()));
}

// ------------------------------------------------------------------ w:rFonts

#[test]
fn both_font_slots_are_read_from_their_own_attributes() {
    let unit = scan_one(
        r#"<w:pPr><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:cs="Dubai"/></w:rPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.latin_font, Resolved::Explicit("Calibri".into()));
    assert_eq!(unit.props.complex_font, Resolved::Explicit("Dubai".into()));
}

#[test]
fn a_theme_reference_is_not_recorded_as_a_typeface() {
    // `majorBidi` names the theme's font scheme, and the theme is §3.3.
    // Recording it would put a scheme name in a report as though it were a
    // font a reviewer could look for.
    let unit = scan_one(
        r#"<w:pPr><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:cstheme="majorBidi"/></w:rPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.complex_font, Resolved::Unset);
    assert_eq!(unit.props.latin_font, Resolved::Unset);
}

#[test]
fn an_empty_font_attribute_names_no_typeface() {
    let unit = scan_one(
        r#"<w:pPr><w:rPr><w:rFonts w:ascii="Calibri" w:cs=""/></w:rPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.complex_font, Resolved::Unset);
}

// ------------------------------------------------------------------- w:numPr

#[test]
fn a_native_list_is_not_a_typed_bullet() {
    let unit = scan_one(
        r#"<w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="3"/></w:numPr></w:pPr>"#,
        ARABIC,
    );
    assert_eq!(unit.props.bullet, Bullet::Native);
    assert_eq!(scan_one("", ARABIC).props.bullet, Bullet::None);
}

// ----------------------------------------------------------------- structure

#[test]
fn a_text_box_paragraph_does_not_swallow_the_paragraph_that_anchors_it() {
    // WordprocessingML paragraphs nest: `w:txbxContent` sits inside a run.
    // A reader holding one paragraph at a time loses the outer one's text
    // when the inner one closes.
    let xml = document(concat!(
        "<w:p><w:pPr><w:bidi/></w:pPr>",
        "<w:r><w:t>خارجي</w:t>",
        "<w:txbxContent><w:p><w:r><w:t>داخلي</w:t></w:r></w:p></w:txbxContent>",
        "</w:r></w:p>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    let texts: Vec<&str> = units.iter().map(|u| u.text.as_str()).collect();
    assert_eq!(texts, ["داخلي", "خارجي"]);
    // And the outer paragraph keeps its own ordinal and its own direction.
    assert_eq!(units[1].id.0, "word/document.xml#p1");
    assert_eq!(units[1].props.direction, Resolved::Explicit(Direction::Rtl));
    assert_eq!(units[0].id.0, "word/document.xml#p2");
    assert_eq!(units[0].props.direction, Resolved::Unset);
}

#[test]
fn a_fallback_is_not_read_beside_the_choice_it_stands_in_for() {
    // `mc:Choice` and `mc:Fallback` spell out the same text box. Reading both
    // reports every defect in it twice, from two unit ids that name one
    // paragraph.
    let xml = document(concat!(
        "<w:p><w:r><mc:AlternateContent>",
        "<mc:Choice Requires=\"wps\"><w:txbxContent>",
        "<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>",
        "</w:txbxContent></mc:Choice>",
        "<mc:Fallback><w:txbxContent>",
        "<w:p><w:r><w:t>مرحبا</w:t></w:r></w:p>",
        "</w:txbxContent></mc:Fallback>",
        "</mc:AlternateContent></w:r></w:p>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].text, "مرحبا");
}

// ------------------------------------------------------------------ tables

/// A one-row table over the given `w:tblPr` and cell texts.
fn table(tbl_pr: &str, cells: &[&str]) -> String {
    let cells: String = cells
        .iter()
        .map(|t| format!("<w:tc><w:p><w:r><w:t>{t}</w:t></w:r></w:p></w:tc>"))
        .collect();
    format!("<w:tbl>{tbl_pr}<w:tr>{cells}</w:tr></w:tbl>")
}

/// The Arabic table header three of the cases below share.
const CELLS: [&str; 3] = ["المؤشر", "الربع الثالث", "الربع الرابع"];

#[test]
fn a_table_is_a_container_of_its_own_addressed_by_part_and_ordinal() {
    let xml = document(&format!(
        "{}{}",
        table("", &CELLS),
        table("", &["Metric", "Q3"]),
    ));
    let units = scan_xml(PART, &xml).unwrap();
    let tables: Vec<_> = units.iter().filter(|u| u.kind == UnitKind::Table).collect();

    assert_eq!(tables.len(), 2, "{units:#?}");
    assert_eq!(tables[0].id.0, "word/document.xml#tbl1");
    assert_eq!(tables[1].id.0, "word/document.xml#tbl2");
    // A container's text is the text it lays out, one paragraph per line.
    assert_eq!(tables[0].text, CELLS.join("\n"));
    // The cells stay paragraphs in their own right.
    assert_eq!(
        units.iter().filter(|u| u.kind == UnitKind::Table).count(),
        2
    );
    assert_eq!(units.len(), 7);
}

#[test]
fn bidi_visual_is_the_tables_direction_and_a_missing_val_is_on() {
    for (tbl_pr, expected) in [
        ("<w:tblPr><w:bidiVisual/></w:tblPr>", Direction::Rtl),
        (
            r#"<w:tblPr><w:bidiVisual w:val="1"/></w:tblPr>"#,
            Direction::Rtl,
        ),
        (
            r#"<w:tblPr><w:bidiVisual w:val="0"/></w:tblPr>"#,
            Direction::Ltr,
        ),
    ] {
        let xml = document(&table(tbl_pr, &CELLS));
        let units = scan_xml(PART, &xml).unwrap();
        let found = units.iter().find(|u| u.kind == UnitKind::Table).unwrap();
        assert_eq!(
            found.props.direction,
            Resolved::Explicit(expected),
            "{tbl_pr}"
        );
    }
}

#[test]
fn an_arabic_table_laid_out_left_to_right_is_the_flagship_table_finding() {
    // The whole of §3.4: `w:bidiVisual` is needed exactly where the cells read
    // right to left, and its absence there reverses the columns for a reader.
    let arabic = document(&table("", &CELLS));
    let units = scan_xml(PART, &arabic).unwrap();
    assert!(findings(&units).contains(&"container-direction".to_string()));

    // And is not needed anywhere else: the same table declared right to left,
    // and an English table declaring nothing, are both silent.
    for xml in [
        document(&table("<w:tblPr><w:bidiVisual/></w:tblPr>", &CELLS)),
        document(&table("", &["Metric", "Q3", "Q4"])),
    ] {
        let units = scan_xml(PART, &xml).unwrap();
        assert!(
            !findings(&units).contains(&"container-direction".to_string()),
            "{xml}"
        );
    }
}

#[test]
fn a_paragraph_in_a_table_cell_is_still_a_paragraph_and_says_which_cell() {
    let xml = document(concat!(
        "<w:tbl><w:tr><w:tc>",
        "<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>خلية</w:t></w:r></w:p>",
        "</w:tc><w:tc>",
        "<w:p><w:pPr><w:bidi/></w:pPr><w:r><w:t>خلية أخرى</w:t></w:r></w:p>",
        "</w:tc></w:tr></w:tbl>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    let paragraphs: Vec<_> = units
        .iter()
        .filter(|u| u.kind == UnitKind::Paragraph)
        .collect();

    assert_eq!(paragraphs.len(), 2, "{units:#?}");
    assert_eq!(paragraphs[0].text, "خلية");
    assert_eq!(
        paragraphs[0].location.container.as_deref(),
        Some("table 1 row 1 cell 1")
    );
    assert_eq!(
        paragraphs[1].location.container.as_deref(),
        Some("table 1 row 1 cell 2")
    );
    // Body text outside every table names no cell, because Word names nothing
    // enclosing it.
    let body = scan_xml(PART, &document(&paragraph("", ARABIC))).unwrap();
    assert!(body[0].location.container.is_none());
}

#[test]
fn a_nested_table_is_its_own_container_and_the_outer_one_lays_it_out_too() {
    // A table in a cell of another is a container of its own — its columns run
    // whichever way its `w:tblPr` says — and the text in it is laid out by
    // both, so both are judged on it.
    let inner = table("<w:tblPr><w:bidiVisual/></w:tblPr>", &["داخلي"]);
    let xml = document(&format!(
        "<w:tbl><w:tr><w:tc>{inner}<w:p><w:r><w:t>خارجي</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
    ));
    let units = scan_xml(PART, &xml).unwrap();
    let tables: Vec<_> = units.iter().filter(|u| u.kind == UnitKind::Table).collect();

    assert_eq!(tables.len(), 2, "{units:#?}");
    // Closed innermost first, so the nested table is `#tbl2` and arrives first.
    assert_eq!(tables[0].id.0, "word/document.xml#tbl2");
    assert_eq!(tables[0].text, "داخلي");
    assert_eq!(
        tables[0].props.direction,
        Resolved::Explicit(Direction::Rtl)
    );
    assert_eq!(tables[1].id.0, "word/document.xml#tbl1");
    assert_eq!(tables[1].text, "داخلي\nخارجي");
    assert_eq!(tables[1].props.direction, Resolved::Unset);

    // The paragraph in the nested table names the cell nearest it.
    let inner_cell = units
        .iter()
        .find(|u| u.text == "داخلي" && u.kind == UnitKind::Paragraph)
        .unwrap();
    assert_eq!(
        inner_cell.location.container.as_deref(),
        Some("table 2 row 1 cell 1")
    );
}

#[test]
fn a_table_that_lays_out_no_text_is_no_unit() {
    let xml = document("<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>");
    let units = scan_xml(PART, &xml).unwrap();
    assert!(units.is_empty(), "{units:#?}");
}

#[test]
fn a_drawingml_table_is_not_a_word_table() {
    // `token.rs`'s claim in the other direction: an `a:tbl` sitting in a Word
    // part is PowerPoint's vocabulary and lays out nothing here.
    let xml = document(concat!(
        "<a:tbl><a:tr><a:tc><a:txBody>",
        "<a:p><a:r><a:t>المؤشر</a:t></a:r></a:p>",
        "</a:txBody></a:tc></a:tr></a:tbl>",
    ));
    assert!(scan_xml(PART, &xml).unwrap().is_empty());
}

#[test]
fn a_superseded_direction_in_a_revision_record_is_not_the_tables() {
    // `w:tblPrChange` holds the column order as it stood *before* a tracked
    // change, in the same element that states it now and written after it.
    // Reading it would report the layout somebody has already corrected.
    let tbl_pr = concat!(
        "<w:tblPr><w:bidiVisual/>",
        r#"<w:tblPrChange w:id="1" w:author="a" w:date="2026-01-01T00:00:00Z">"#,
        r#"<w:tblPr><w:bidiVisual w:val="0"/></w:tblPr></w:tblPrChange>"#,
        "</w:tblPr>",
    );
    let xml = document(&table(tbl_pr, &CELLS));
    let units = scan_xml(PART, &xml).unwrap();
    let found = units.iter().find(|u| u.kind == UnitKind::Table).unwrap();
    assert_eq!(found.props.direction, Resolved::Explicit(Direction::Rtl));
}

#[test]
fn a_superseded_paragraph_property_is_not_the_paragraphs_either() {
    // The same claim one level down: `w:pPrChange` carries a whole `w:pPr`,
    // and it is written after the one in force.
    let p_pr = concat!(
        "<w:pPr><w:bidi/>",
        r#"<w:pPrChange w:id="1" w:author="a" w:date="2026-01-01T00:00:00Z">"#,
        r#"<w:pPr><w:bidi w:val="0"/><w:jc w:val="center"/></w:pPr></w:pPrChange>"#,
        "</w:pPr>",
    );
    let unit = scan_one(p_pr, ARABIC);
    assert_eq!(unit.props.direction, Resolved::Explicit(Direction::Rtl));
    assert!(unit.props.alignment.is_unset(), "{unit:#?}");
}

// ------------------------------------------------------------------ the package

/// Write a minimal Word package: `[Content_Types].xml`, which is what makes a
/// ZIP an OOXML container, plus whatever parts a case needs.
fn package(dir: &Path, parts: &[(&str, &str)]) -> PathBuf {
    let path = dir.join("document.docx");
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    let options: SimpleFileOptions = SimpleFileOptions::default();
    zip.start_file("[Content_Types].xml", options).unwrap();
    zip.write_all(
        br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
    )
    .unwrap();
    for (name, body) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mirsam-docx-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_scan_reads_every_word_part_that_carries_text() {
    // A header and a footnote carry Arabic as readily as the body does, and a
    // reader that opened `document.xml` alone would report a document clean
    // while its running head was reversed.
    let scratch = Scratch::new("parts");
    let path = package(
        &scratch.0,
        &[
            ("word/document.xml", &document(&paragraph("", "المتن"))),
            ("word/header1.xml", &document(&paragraph("", "الترويسة"))),
            ("word/footnotes.xml", &document(&paragraph("", "الحاشية"))),
            // Carries `w:pPr` but no `w:p`: nothing to report, and it must not
            // become one.
            (
                "word/styles.xml",
                r#"<w:styles xmlns:w="ns/w"><w:style><w:pPr><w:bidi/></w:pPr></w:style></w:styles>"#,
            ),
            // Not a part this adapter reads, and not XML it could parse.
            ("word/_rels/document.xml.rels", "<Relationships/>"),
        ],
    );

    let mut doc = DocxDocument::open(&path).expect("the package did not open");
    assert_eq!(doc.format(), "docx");
    let units = doc.scan().expect("the scan failed");

    let mut seen: Vec<(&str, &str)> = units
        .iter()
        .map(|u| (u.location.part.as_str(), u.text.as_str()))
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [
            ("word/document.xml", "المتن"),
            ("word/footnotes.xml", "الحاشية"),
            ("word/header1.xml", "الترويسة"),
        ]
    );
    // Each part numbers its own paragraphs, so an id names one place.
    assert!(units.iter().all(|u| u.id.0.ends_with("#p1")));
}

#[test]
fn a_package_without_a_content_types_part_is_not_a_document() {
    let scratch = Scratch::new("not-ooxml");
    let path = scratch.0.join("plain.docx");
    fs::write(&path, b"this is not a ZIP archive").unwrap();
    assert!(DocxDocument::open(&path).is_err());
}
