//! The Excel adapter (PLAN §5.3).
//!
//! The questions this file asks are the ones the module documentation answers
//! in prose: is a cell a paragraph, is a worksheet a container, what does a
//! cell inherit and from where, what does a repair append rather than edit, and
//! what survives a repair untouched.

use mirsam_core::{
    Alignment, Bullet, Direction, DocumentReader, DocumentWriter, Fix, Repair, Resolved, UnitId,
    UnitKind,
};
use mirsam_ooxml::workbook::Workbook;
use mirsam_ooxml::{Package, XlsxDocument, xlsx};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const ARABIC: &str = "ارتفع الأداء في الربع الرابع";
const SHEET: &str = "xl/worksheets/sheet1.xml";

// ------------------------------------------------------------ building a book

/// A workbook written to disk, and removed with the value.
struct Book {
    dir: PathBuf,
    path: PathBuf,
}

impl Drop for Book {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The parts every workbook here needs, with the four a case varies supplied.
struct Parts<'a> {
    sheet: &'a str,
    styles: &'a str,
    strings: &'a str,
    /// `docProps/core.xml`'s `dc:language`, if the book states one.
    language: Option<&'a str>,
    /// A second worksheet, for the cases about more than one.
    sheet2: Option<&'a str>,
    /// `xl/workbook.xml`'s `<definedNames>`, for the case that asserts they
    /// survive a repair.
    defined_names: &'a str,
}

impl Default for Parts<'_> {
    fn default() -> Self {
        Self {
            sheet: "",
            styles: EMPTY_STYLES,
            strings: "",
            language: None,
            sheet2: None,
            defined_names: "",
        }
    }
}

/// `xl/styles.xml` with the one format record every workbook has.
const EMPTY_STYLES: &str = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><name val="Calibri"/></font></fonts><cellStyleXfs count="1"><xf numFmtId="0" fontId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" xfId="0"/></cellXfs></styleSheet>"#;

const SPREADSHEETML: &str = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

const OFFICE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn scratch() -> PathBuf {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mirsam-xlsx-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(parts: &Parts<'_>) -> Book {
    let dir = scratch();
    let path = dir.join("book.xlsx");

    let mut sheets = String::from(r#"<sheet name="Q4" sheetId="1" r:id="rId1"/>"#);
    let mut items: Vec<(String, String)> = Vec::new();
    let mut relationships = vec![("worksheet", "worksheets/sheet1.xml")];
    if parts.sheet2.is_some() {
        sheets.push_str(r#"<sheet name="Notes" sheetId="2" r:id="rId2"/>"#);
        relationships.push(("worksheet", "worksheets/sheet2.xml"));
    }
    relationships.push(("styles", "styles.xml"));
    if !parts.strings.is_empty() {
        relationships.push(("sharedStrings", "sharedStrings.xml"));
    }

    items.push((
        "[Content_Types].xml".into(),
        r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#.into(),
    ));
    items.push((
        "_rels/.rels".into(),
        rels(&[("officeDocument", "xl/workbook.xml")]),
    ));
    items.push((
        "xl/workbook.xml".into(),
        format!(
            "<workbook {SPREADSHEETML}><sheets>{sheets}</sheets>{}</workbook>",
            parts.defined_names
        ),
    ));
    items.push(("xl/_rels/workbook.xml.rels".into(), rels(&relationships)));
    items.push((SHEET.into(), worksheet(parts.sheet)));
    if let Some(second) = parts.sheet2 {
        items.push(("xl/worksheets/sheet2.xml".into(), worksheet(second)));
    }
    items.push(("xl/styles.xml".into(), parts.styles.into()));
    if !parts.strings.is_empty() {
        items.push(("xl/sharedStrings.xml".into(), parts.strings.into()));
    }
    if let Some(tag) = parts.language {
        items.push((
            "docProps/core.xml".into(),
            format!(
                r#"<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:language>{tag}</dc:language></cp:coreProperties>"#
            ),
        ));
    }

    let mut zip = ZipWriter::new(File::create(&path).expect("creating the package"));
    let options = SimpleFileOptions::default();
    for (name, body) in &items {
        zip.start_file(name.as_str(), options).expect("a part");
        zip.write_all(body.as_bytes()).expect("writing a part");
    }
    zip.finish().expect("finishing the package");

    Book { dir, path }
}

fn worksheet(body: &str) -> String {
    format!("<worksheet {SPREADSHEETML}>{body}</worksheet>")
}

fn rels(items: &[(&str, &str)]) -> String {
    let mut xml = String::from(
        r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (index, (kind, target)) in items.iter().enumerate() {
        xml.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="{OFFICE}/{kind}" Target="{target}"/>"#,
            index + 1
        ));
    }
    xml.push_str("</Relationships>");
    xml
}

/// A `<sheetData>` holding one row of inline-string cells, one per column.
fn row(cells: &[&str]) -> String {
    let mut xml = String::from(r#"<sheetData><row r="1">"#);
    for (index, text) in cells.iter().enumerate() {
        let reference = format!("{}1", (b'A' + index as u8) as char);
        xml.push_str(&format!(
            r#"<c r="{reference}" t="inlineStr"><is><t>{text}</t></is></c>"#
        ));
    }
    xml.push_str("</row></sheetData>");
    xml
}

/// A `<sheetData>` holding one column of inline-string cells, one per row.
fn column(cells: &[&str]) -> String {
    let mut xml = String::from("<sheetData>");
    for (index, text) in cells.iter().enumerate() {
        xml.push_str(&format!(
            r#"<row r="{n}"><c r="A{n}" t="inlineStr"><is><t>{text}</t></is></c></row>"#,
            n = index + 1
        ));
    }
    xml.push_str("</sheetData>");
    xml
}

fn read(parts: &Parts<'_>) -> (Book, Vec<mirsam_core::TextUnit>, Vec<String>) {
    let book = write(parts);
    let mut document = XlsxDocument::open(&book.path).expect("the workbook opened");
    let units = document.scan().expect("the scan");
    let unread = document.unread_sources();
    (book, units, unread)
}

fn only(units: &[mirsam_core::TextUnit], kind: UnitKind) -> &mirsam_core::TextUnit {
    let found: Vec<_> = units.iter().filter(|u| u.kind == kind).collect();
    assert_eq!(found.len(), 1, "expected one {kind:?}: {units:#?}");
    found[0]
}

// ------------------------------------------------------------------ the shape

#[test]
fn a_cell_is_a_paragraph_and_names_the_sheet_and_reference_a_person_would_type() {
    let (_book, units, _) = read(&Parts {
        sheet: &column(&[ARABIC]),
        ..Parts::default()
    });
    let cell = only(&units, UnitKind::Paragraph);
    assert_eq!(cell.text, ARABIC);
    assert_eq!(cell.location.part, SHEET);
    assert_eq!(cell.location.paragraph, Some(1));
    assert_eq!(cell.location.container.as_deref(), Some("Q4!A1"));
    assert_eq!(cell.id.0, format!("{SHEET}#p1"));
}

#[test]
fn a_sheet_of_one_column_is_no_container_and_one_of_two_is() {
    let (_book, one, _) = read(&Parts {
        sheet: &column(&[ARABIC, "الربع"]),
        ..Parts::default()
    });
    assert!(
        !one.iter().any(|u| u.kind == UnitKind::Table),
        "a single column has no column order to get wrong: {one:#?}"
    );

    let (_book, two, _) = read(&Parts {
        sheet: &row(&["المؤشر", "الربع"]),
        ..Parts::default()
    });
    let grid = only(&two, UnitKind::Table);
    assert_eq!(grid.text, "المؤشر\nالربع");
    assert_eq!(grid.id.0, format!("{SHEET}#tbl1"));
    assert_eq!(grid.location.paragraph, None);
    assert_eq!(grid.location.container.as_deref(), Some("sheet \"Q4\""));
}

#[test]
fn a_cell_ordinal_counts_every_cell_including_the_numbers_between_them() {
    // The ordinal is the address a repair is written back to. One that skipped
    // the cells this adapter says nothing about would drift the moment a number
    // was typed beside the text.
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1"><v>42</v></c><c r="B1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        ..Parts::default()
    });
    let cell = only(&units, UnitKind::Paragraph);
    assert_eq!(cell.id.0, format!("{SHEET}#p2"), "{units:#?}");
}

// -------------------------------------------------------------- what it reads

#[test]
fn a_reading_order_the_cell_states_is_explicit_and_context_dependent_is_unset() {
    // `readingOrder="0"` is *context dependent*: it asks the application to
    // guess from the first strong character, which is what `Unset` already
    // says. Reading it as a decision would pass Arabic that is correct by luck.
    for (value, expected) in [
        ("2", Some(Resolved::Explicit(Direction::Rtl))),
        ("1", Some(Resolved::Explicit(Direction::Ltr))),
        ("0", None),
    ] {
        let styles = format!(
            r#"<styleSheet><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" applyAlignment="1"><alignment readingOrder="{value}"/></xf></cellXfs></styleSheet>"#
        );
        let sheet = format!(
            r#"<sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
        );
        let (_book, units, _) = read(&Parts {
            sheet: &sheet,
            styles: &styles,
            ..Parts::default()
        });
        let direction = &only(&units, UnitKind::Paragraph).props.direction;
        match expected {
            Some(expected) => assert_eq!(*direction, expected, "readingOrder={value}"),
            None => assert!(direction.is_unset(), "readingOrder={value}: {direction:?}"),
        }
    }
}

#[test]
fn a_cell_that_states_no_reading_order_inherits_the_sheets_and_names_it() {
    let sheet = format!(
        r#"<sheetViews><sheetView workbookViewId="0" rightToLeft="1"/></sheetViews>{}"#,
        column(&[ARABIC])
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        ..Parts::default()
    });
    let cell = only(&units, UnitKind::Paragraph);
    match &cell.props.direction {
        Resolved::Inherited(direction, origin) => {
            assert_eq!(*direction, Direction::Rtl);
            assert_eq!(origin.part, SHEET);
            assert_eq!(origin.property, "sheetView@rightToLeft");
        }
        other => panic!("expected the sheet's direction, got {other:?}"),
    }
}

#[test]
fn a_cell_that_states_its_own_reading_order_takes_nothing_from_the_sheet() {
    let styles = r#"<styleSheet><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" applyAlignment="1"><alignment readingOrder="2"/></xf></cellXfs></styleSheet>"#;
    let sheet = format!(
        r#"<sheetViews><sheetView workbookViewId="0" rightToLeft="0"/></sheetViews><sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        styles,
        ..Parts::default()
    });
    assert_eq!(
        only(&units, UnitKind::Paragraph).props.direction,
        Resolved::Explicit(Direction::Rtl)
    );
}

#[test]
fn a_named_cell_style_supplies_what_the_record_does_not_and_says_where_from() {
    let styles = r#"<styleSheet><cellStyleXfs count="1"><xf applyAlignment="1"><alignment horizontal="center"/></xf></cellStyleXfs><cellXfs count="1"><xf xfId="0"/></cellXfs></styleSheet>"#;
    let (_book, units, _) = read(&Parts {
        sheet: &column(&[ARABIC]),
        styles,
        ..Parts::default()
    });
    match &only(&units, UnitKind::Paragraph).props.alignment {
        Resolved::Inherited(alignment, origin) => {
            assert_eq!(*alignment, Alignment::Center);
            assert_eq!(origin.part, "xl/styles.xml");
            assert_eq!(origin.property, "cellStyleXfs/xf/alignment@horizontal");
        }
        other => panic!("expected the cell style's alignment, got {other:?}"),
    }
}

#[test]
fn alignment_excel_would_not_apply_is_not_read() {
    // `applyAlignment="0"` means Excel ignores the alignment beside it.
    // Reporting `alignment-incoherent` on formatting nobody sees is the false
    // positive this project treats as worse than a miss.
    let styles = r#"<styleSheet><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" applyAlignment="0"><alignment horizontal="left"/></xf></cellXfs></styleSheet>"#;
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        styles,
        ..Parts::default()
    });
    assert!(only(&units, UnitKind::Paragraph).props.alignment.is_unset());
}

#[test]
fn general_alignment_is_unset_rather_than_a_direction_relative_edge() {
    // `general` puts text at the start edge and numbers at the end one: it is
    // Excel's default by data type, not an edge anybody chose.
    let styles = r#"<styleSheet><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" applyAlignment="1"><alignment horizontal="general"/></xf></cellXfs></styleSheet>"#;
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        styles,
        ..Parts::default()
    });
    assert!(only(&units, UnitKind::Paragraph).props.alignment.is_unset());
}

#[test]
fn one_font_answers_for_every_script_so_both_slots_hold_it() {
    // The reason `complex-font-missing` cannot fire on a workbook: there is no
    // pair of slots to fill unevenly.
    let styles = r#"<styleSheet><fonts count="2"><font><name val="Calibri"/></font><font><name val="Dubai"/></font></fonts><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="2"><xf xfId="0"/><xf xfId="0" fontId="1" applyFont="1"/></cellXfs></styleSheet>"#;
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let (_book, units, _) = read(&Parts {
        sheet: &sheet,
        styles,
        ..Parts::default()
    });
    let props = &only(&units, UnitKind::Paragraph).props;
    assert_eq!(props.complex_font, Resolved::Explicit("Dubai".into()));
    assert_eq!(props.latin_font, props.complex_font);
}

#[test]
fn the_workbooks_language_is_the_cells_and_names_the_part_that_states_it() {
    let (_book, units, _) = read(&Parts {
        sheet: &column(&[ARABIC]),
        language: Some("ar-SA"),
        ..Parts::default()
    });
    match &only(&units, UnitKind::Paragraph).props.language {
        Resolved::Inherited(tag, origin) => {
            assert_eq!(tag, "ar-SA");
            assert_eq!(origin.part, "docProps/core.xml");
        }
        other => panic!("expected the workbook's language, got {other:?}"),
    }
}

#[test]
fn excel_has_no_list_so_a_cell_never_carries_a_native_bullet() {
    let (_book, units, _) = read(&Parts {
        sheet: &column(&["• بند أول"]),
        ..Parts::default()
    });
    assert_eq!(only(&units, UnitKind::Paragraph).props.bullet, Bullet::None);
}

#[test]
fn a_shared_string_is_resolved_and_its_runs_are_one_string() {
    let strings = format!(
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2"><si><t>{ARABIC}</t></si><si><r><t>الربع </t></r><r><t>الرابع</t></r></si></sst>"#
    );
    let sheet = r#"<sheetData><row r="1"><c r="A1" t="s"><v>1</v></c></row></sheetData>"#;
    let (_book, units, _) = read(&Parts {
        sheet,
        strings: &strings,
        ..Parts::default()
    });
    assert_eq!(only(&units, UnitKind::Paragraph).text, "الربع الرابع");
}

#[test]
fn a_phonetic_guide_is_not_part_of_the_text() {
    let strings = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><t>المؤشر</t><rPh sb="0" eb="1"><t>ignored</t></rPh></si></sst>"#;
    let sheet = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#;
    let (_book, units, _) = read(&Parts {
        sheet,
        strings,
        ..Parts::default()
    });
    assert_eq!(only(&units, UnitKind::Paragraph).text, "المؤشر");
}

// ------------------------------------------------------------ what it refuses

#[test]
fn a_formula_result_is_no_unit_and_is_named_as_a_source_that_was_not_read() {
    // The cached value is not the document's text: Excel recomputes it on open.
    // Standing rule 4 says the omission has to be sayable, so it is.
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" t="str"><f>B1&amp;C1</f><v>{ARABIC}</v></c></row></sheetData>"#
    );
    let (_book, units, unread) = read(&Parts {
        sheet: &sheet,
        ..Parts::default()
    });
    assert!(units.is_empty(), "{units:#?}");
    assert_eq!(unread, vec!["Q4!A1".to_string()]);
}

#[test]
fn a_formula_result_with_no_arabic_in_it_is_not_named() {
    // There is nothing there this tool would have judged, so naming it would
    // be noise rather than an honest `NOT RUN`.
    let sheet =
        r#"<sheetData><row r="1"><c r="A1" t="str"><f>B1+C1</f><v>Total</v></c></row></sheetData>"#;
    let (_book, _units, unread) = read(&Parts {
        sheet,
        ..Parts::default()
    });
    assert!(unread.is_empty(), "{unread:?}");
}

#[test]
fn the_three_repairs_spreadsheetml_cannot_express_are_refused_and_the_rest_are_not() {
    let book = write(&Parts {
        sheet: &column(&[ARABIC]),
        ..Parts::default()
    });
    let document = XlsxDocument::open(&book.path).unwrap();
    for refused in [
        Fix::SetLanguage("ar-SA".into()),
        Fix::SetComplexFont("Dubai".into()),
        Fix::ConvertLiteralBullet { marker: '•' },
    ] {
        assert!(!document.supports(&refused), "{refused:?}");
    }
    for supported in [
        Fix::SetDirection(Direction::Rtl),
        Fix::SetAlignment(Alignment::Start),
        Fix::RemoveControls(vec![0]),
        Fix::RemoveTatweel(vec![0]),
        Fix::NormalizePresentationForms,
    ] {
        assert!(document.supports(&supported), "{supported:?}");
    }
}

// ----------------------------------------------------------------- repairing

/// Repair one workbook and hand back the package that came out.
fn repair(parts: &Parts<'_>, repairs: &[Repair]) -> (Book, Package) {
    let book = write(parts);
    let out = book.dir.join("repaired.xlsx");
    let mut document = XlsxDocument::open(&book.path).unwrap();
    document.scan().expect("the scan");
    let staged = document.apply(repairs).expect("the repair");
    assert_eq!(staged, repairs.len());
    document.write(&out).expect("writing the repaired copy");
    let package = Package::open(&out).expect("the repaired package opened");
    (book, package)
}

fn unit(index: usize) -> UnitId {
    UnitId(format!("{SHEET}#p{index}"))
}

#[test]
fn a_cell_repair_appends_a_format_record_and_leaves_every_other_cell_pointing_at_the_old_one() {
    // The whole reason this adapter appends. Two cells share record 0; one is
    // repaired, and the other must come out exactly as it went in.
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{ARABIC}</t></is></c><c r="B1" t="inlineStr"><is><t>Performance</t></is></c></row></sheetData>"#
    );
    let parts = Parts {
        sheet: &sheet,
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::SetDirection(Direction::Rtl))],
    );

    let sheet = package.read_text(SHEET).unwrap();
    assert!(
        sheet.contains(r#"<c r="A1" t="inlineStr" s="1">"#),
        "{sheet}"
    );
    assert!(
        sheet.contains(r#"<c r="B1" t="inlineStr">"#),
        "the untouched cell moved: {sheet}"
    );

    let styles = package.read_text("xl/styles.xml").unwrap();
    assert!(
        styles.contains(r#"<alignment readingOrder="2"/>"#),
        "{styles}"
    );
    assert!(styles.contains(r#"<cellXfs count="2">"#), "{styles}");
}

#[test]
fn two_cells_wanting_the_same_repair_share_one_appended_record() {
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{ARABIC}</t></is></c><c r="B1" t="inlineStr"><is><t>{ARABIC}</t></is></c></row></sheetData>"#
    );
    let parts = Parts {
        sheet: &sheet,
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[
            Repair::new(&unit(1), Fix::SetDirection(Direction::Rtl)),
            Repair::new(&unit(2), Fix::SetDirection(Direction::Rtl)),
        ],
    );
    let styles = package.read_text("xl/styles.xml").unwrap();
    assert!(
        styles.contains(r#"<cellXfs count="2">"#),
        "one record, not two: {styles}"
    );
    let sheet = package.read_text(SHEET).unwrap();
    assert_eq!(sheet.matches(r#"s="1""#).count(), 2, "{sheet}");
}

#[test]
fn a_direction_relative_alignment_is_lowered_against_the_direction_being_repaired_to() {
    // `horizontal` names physical edges, so `start` under right-to-left is
    // `right`. Lowering it against left-to-right would write the defect back.
    let parts = Parts {
        sheet: &column(&[ARABIC]),
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[
            Repair::new(&unit(1), Fix::SetDirection(Direction::Rtl)),
            Repair::new(&unit(1), Fix::SetAlignment(Alignment::Start)),
        ],
    );
    let styles = package.read_text("xl/styles.xml").unwrap();
    assert!(styles.contains(r#"horizontal="right""#), "{styles}");
    assert!(styles.contains(r#"readingOrder="2""#), "{styles}");
}

#[test]
fn a_start_edge_is_lowered_against_the_direction_the_cell_inherits_from_its_sheet() {
    // The case the rule actually plans: `alignment-unset` fires on a cell whose
    // direction is inherited from the sheet and agrees with the text, so no
    // `SetDirection` comes with it. Lowering `start` against left-to-right —
    // the cell's own record says nothing — would write the very hard left edge
    // `alignment-incoherent` exists to report.
    let sheet = format!(
        r#"<sheetViews><sheetView workbookViewId="0" rightToLeft="1"/></sheetViews>{}"#,
        column(&[ARABIC])
    );
    let parts = Parts {
        sheet: &sheet,
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::SetAlignment(Alignment::Start))],
    );
    let styles = package.read_text("xl/styles.xml").unwrap();
    assert!(
        styles.contains(r#"horizontal="right""#),
        "the start edge was lowered against the wrong direction: {styles}"
    );
}

#[test]
fn the_grids_direction_is_written_on_the_sheet_view_and_created_where_there_is_none() {
    let parts = Parts {
        sheet: &row(&["المؤشر", "الربع"]),
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[Repair::new(
            &UnitId(format!("{SHEET}#tbl1")),
            Fix::SetDirection(Direction::Rtl),
        )],
    );
    let sheet = package.read_text(SHEET).unwrap();
    assert!(
        sheet.contains(r#"<sheetView workbookViewId="0" rightToLeft="1"/>"#),
        "{sheet}"
    );
    // And in schema position: `sheetViews` precedes `sheetData`.
    let views = sheet.find("<sheetViews>").expect("the created element");
    let data = sheet.find("<sheetData>").expect("the cells");
    assert!(views < data, "{sheet}");
}

#[test]
fn a_shared_string_repair_appends_a_string_and_leaves_the_original_for_the_cells_still_using_it() {
    const PADDED: &str = "العنوان\u{0640}\u{0640}\u{0640}\u{0640}\u{0640}";
    let strings = format!(
        r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="1"><si><t>{PADDED}</t></si></sst>"#
    );
    let sheet = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>0</v></c></row></sheetData>"#;
    let parts = Parts {
        sheet,
        strings: &strings,
        ..Parts::default()
    };
    // العنوان is seven characters of two bytes each.
    let offsets: Vec<usize> = (0..5).map(|n| 14 + n * 2).collect();
    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::RemoveTatweel(offsets))],
    );

    let strings = package.read_text("xl/sharedStrings.xml").unwrap();
    assert!(
        strings.contains(&format!("<si><t>{PADDED}</t></si>")),
        "{strings}"
    );
    assert!(strings.contains("<si><t>العنوان</t></si>"), "{strings}");
    assert!(strings.contains(r#"uniqueCount="2""#), "{strings}");

    let sheet = package.read_text(SHEET).unwrap();
    assert!(sheet.contains(r#"<c r="A1" t="s"><v>1</v></c>"#), "{sheet}");
    assert!(
        sheet.contains(r#"<c r="B1" t="s"><v>0</v></c>"#),
        "the other cell was repointed: {sheet}"
    );
}

#[test]
fn a_shared_string_keeps_its_runs_through_a_repair() {
    // Cloned rather than rebuilt from its text: a string whose runs carry their
    // own formatting keeps it, which invariant 3 requires of everything a fix
    // does not address.
    let strings = r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1"><si><r><rPr><b/></rPr><t>الربع</t></r><r><t>\u{202E}الرابع</t></r></si></sst>"#;
    let strings = strings.replace("\\u{202E}", "\u{202E}");
    let sheet = r#"<sheetData><row r="1"><c r="A1" t="s"><v>0</v></c></row></sheetData>"#;
    let parts = Parts {
        sheet,
        strings: &strings,
        ..Parts::default()
    };
    // الربع is five characters of two bytes each; the control follows them.
    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::RemoveControls(vec![10]))],
    );
    let out = package.read_text("xl/sharedStrings.xml").unwrap();
    // Two strings now, each with its run formatting: the original, which cells
    // this repair said nothing about may still be pointing at, and the copy.
    assert_eq!(out.matches("<rPr><b/></rPr>").count(), 2, "{out}");
    assert_eq!(
        out.matches('\u{202E}').count(),
        1,
        "the control left the copy and stayed in the original: {out}"
    );
    assert!(out.contains("<r><t>الرابع</t></r>"), "{out}");
}

#[test]
fn a_repair_leaves_formulas_and_defined_names_exactly_as_they_were() {
    // PLAN §5.3's acceptance. Neither is anything a repair addresses, and the
    // guarantee is mechanical rather than careful: `xl/workbook.xml` is never
    // in the set of parts a plan can name, and a cell repair edits `@s` on a
    // `<c>` and nothing beside it.
    let sheet = format!(
        r#"<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{ARABIC}</t></is></c><c r="B1"><f>SUM(C1:C9)</f><v>42</v></c></row></sheetData>"#
    );
    let names = r#"<definedNames><definedName name="Total">Q4!$B$1</definedName></definedNames>"#;
    let parts = Parts {
        sheet: &sheet,
        defined_names: names,
        ..Parts::default()
    };
    let original = write(&parts);
    let before = Package::open(&original.path).unwrap();
    let workbook_before = before.read_bytes("xl/workbook.xml").unwrap();
    drop(before);

    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::SetDirection(Direction::Rtl))],
    );
    assert_eq!(
        package.read_bytes("xl/workbook.xml").unwrap(),
        workbook_before,
        "the workbook part was rewritten"
    );
    let sheet = package.read_text(SHEET).unwrap();
    assert!(sheet.contains("<f>SUM(C1:C9)</f>"), "{sheet}");
    assert!(sheet.contains(r#"<c r="B1"><f>"#), "{sheet}");
    drop(original);
}

#[test]
fn a_repaired_workbook_re_reads_as_repaired() {
    // The claim the repair path actually makes: the file on disk, opened again
    // through the same port a later `audit` would take.
    let parts = Parts {
        sheet: &column(&[ARABIC]),
        ..Parts::default()
    };
    let (_book, package) = repair(
        &parts,
        &[Repair::new(&unit(1), Fix::SetDirection(Direction::Rtl))],
    );
    let mut document = XlsxDocument::open(package.path()).unwrap();
    let units = document.scan().unwrap();
    assert_eq!(
        only(&units, UnitKind::Paragraph).props.direction,
        Resolved::Explicit(Direction::Rtl)
    );
}

#[test]
fn a_repair_naming_a_cell_the_sheet_does_not_hold_is_refused() {
    let book = write(&Parts {
        sheet: &column(&[ARABIC]),
        ..Parts::default()
    });
    let mut document = XlsxDocument::open(&book.path).unwrap();
    document.scan().unwrap();
    assert!(
        document
            .apply(&[Repair::new(&unit(9), Fix::SetDirection(Direction::Rtl))])
            .is_err()
    );
}

// -------------------------------------------------------- the in-memory path

#[test]
fn a_worksheet_can_be_scanned_without_a_package() {
    let workbook = Workbook::default();
    let units = xlsx::scan_xml("Q4", SHEET, &worksheet(&column(&[ARABIC])), &workbook).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].text, ARABIC);
    assert!(units[0].props.direction.is_unset());
}

#[test]
fn a_second_sheet_is_a_second_part_and_its_units_name_it() {
    let (_book, units, _) = read(&Parts {
        sheet: &column(&[ARABIC]),
        sheet2: Some(&column(&["الربع"])),
        ..Parts::default()
    });
    let parts: Vec<&str> = units.iter().map(|u| u.location.part.as_str()).collect();
    assert_eq!(parts, [SHEET, "xl/worksheets/sheet2.xml"]);
    for unit in &units {
        assert!(unit.id.0.starts_with(&unit.location.part), "{}", unit.id);
    }
}

#[test]
fn a_document_that_is_not_an_ooxml_package_is_refused() {
    let dir = scratch();
    let path = dir.join("book.xlsx");
    std::fs::write(&path, b"not a zip").unwrap();
    assert!(XlsxDocument::open(&path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The format name, which the conformance suite and the CLI both rely on.
#[test]
fn the_adapter_names_itself_xlsx() {
    let book = write(&Parts::default());
    let document = XlsxDocument::open(&book.path).unwrap();
    assert_eq!(document.format(), "xlsx");
    assert_eq!(document.path(), Path::new(&book.path));
}
