//! Word's style chain (PLAN M3 3.3, and its table styles — 3.4).
//!
//! `w:docDefaults`, the named styles a paragraph reaches through `w:pStyle`
//! and `w:rStyle`, the table style a `w:tbl` and the paragraphs in its cells
//! reach through `w:tblStyle`, the `w:basedOn` walk above them, and the theme
//! a `@w:cstheme` points at. Every assertion is on the resolved `TextUnit`,
//! because what this file is asking is whether a Word paragraph reaches the
//! rules carrying the same evidence a PowerPoint one does — a value, and the
//! part and property that supplied it.
//!
//! Nothing here reaches into `mirsam-core`. A case that needed a core change
//! to pass would be PLAN §3.5's answer, not this file's.

use mirsam_core::DocumentReader;
use mirsam_core::text::{Origin, Resolved, TextUnit};
use mirsam_core::{Alignment, Bullet, Direction, Engine};
use mirsam_ooxml::StyleSheet;
use mirsam_ooxml::docx::{DocxDocument, scan_xml, scan_xml_with};
use mirsam_ooxml::inherit::FontScheme;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const PART: &str = "word/document.xml";
const STYLES: &str = "word/styles.xml";
const THEME: &str = "word/theme/theme1.xml";

const W: &str = r#" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const A: &str = r#" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;

const ARABIC: &str = "مرحبا بالعالم";

/// A `word/document.xml` holding one paragraph with the given `w:pPr`.
fn document(p_pr: &str) -> String {
    format!(
        "<w:document{W}><w:body><w:p>{p_pr}<w:r><w:t>{ARABIC}</w:t></w:r></w:p></w:body></w:document>"
    )
}

/// The same, with the run carrying properties of its own.
fn document_with_run(p_pr: &str, r_pr: &str) -> String {
    format!(
        "<w:document{W}><w:body><w:p>{p_pr}<w:r>{r_pr}<w:t>{ARABIC}</w:t></w:r></w:p></w:body></w:document>"
    )
}

/// A `word/styles.xml` around whatever sources a case needs.
fn styles(body: &str) -> String {
    format!("<w:styles{W}>{body}</w:styles>")
}

/// A theme naming one typeface in each complex-script slot.
fn theme(major: &str, minor: &str) -> String {
    format!(
        "<a:theme{A}><a:themeElements><a:fontScheme name=\"Office\">\
         <a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/>\
         <a:cs typeface=\"{major}\"/></a:majorFont>\
         <a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/>\
         <a:cs typeface=\"{minor}\"/></a:minorFont>\
         </a:fontScheme></a:themeElements></a:theme>"
    )
}

/// The one unit of a document, resolved against a stylesheet.
fn resolve(p_pr: &str, sheet: &StyleSheet) -> TextUnit {
    resolve_with_run(p_pr, "", sheet)
}

/// The same, with run properties on the paragraph's single run.
fn resolve_with_run(p_pr: &str, r_pr: &str, sheet: &StyleSheet) -> TextUnit {
    let xml = document_with_run(p_pr, r_pr);
    let mut units = scan_xml_with(PART, &xml, Some(sheet)).expect("the part did not parse");
    assert_eq!(units.len(), 1, "expected exactly one unit from {xml}");
    units.remove(0)
}

/// Read a stylesheet out of one hand-written part.
fn sheet(body: &str) -> StyleSheet {
    StyleSheet::parse(STYLES, &styles(body)).expect("the stylesheet did not parse")
}

/// The value and origin of an inherited property, or a panic naming what it
/// actually was.
fn inherited<T: std::fmt::Debug>(resolved: &Resolved<T>) -> (&T, &Origin) {
    match resolved {
        Resolved::Inherited(value, origin) => (value, origin),
        other => panic!("expected an inherited value, got {other:?}"),
    }
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

// ------------------------------------------------------------------ docDefaults

#[test]
fn doc_defaults_supply_what_the_paragraph_left_unset_and_name_where_they_came_from() {
    let sheet = sheet(
        "<w:docDefaults>\
           <w:pPrDefault><w:pPr><w:bidi/><w:jc w:val=\"right\"/></w:pPr></w:pPrDefault>\
           <w:rPrDefault><w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr></w:rPrDefault>\
         </w:docDefaults>",
    );
    let unit = resolve("", &sheet);

    let (direction, origin) = inherited(&unit.props.direction);
    assert_eq!(*direction, Direction::Rtl);
    assert_eq!(origin.part, STYLES);
    assert_eq!(origin.property, "docDefaults/pPrDefault/pPr@bidi");

    let (alignment, origin) = inherited(&unit.props.alignment);
    // `w:jc` is direction-relative in Word wherever it is written, including
    // in a style: `right` is the end edge, never a hard physical one.
    assert_eq!(*alignment, Alignment::End);
    assert_eq!(origin.property, "docDefaults/pPrDefault/pPr@jc");

    let (font, origin) = inherited(&unit.props.complex_font);
    assert_eq!(font, "Dubai");
    assert_eq!(origin.property, "docDefaults/rPrDefault/rPr/rFonts@cs");
}

#[test]
fn direct_formatting_beats_every_source_above_it() {
    let sheet = sheet(
        "<w:docDefaults><w:pPrDefault><w:pPr><w:bidi w:val=\"0\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:pPr><w:bidi w:val=\"0\"/></w:pPr></w:style>",
    );
    let unit = resolve("<w:pPr><w:bidi/></w:pPr>", &sheet);
    assert_eq!(unit.props.direction, Resolved::Explicit(Direction::Rtl));
}

#[test]
fn a_language_tag_is_never_resolved_through_the_chain() {
    // ADR 0007's agreement test is stated for direction and alignment and says
    // nothing about `lang`. Resolving one would be inventing the semantics.
    let sheet = sheet(
        "<w:docDefaults><w:rPrDefault><w:rPr>\
           <w:lang w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault></w:docDefaults>",
    );
    assert!(resolve("", &sheet).props.language.is_unset());
}

#[test]
fn the_latin_slot_is_never_resolved_through_the_chain() {
    // `complex-font-missing` fires only where a Latin font is chosen, so
    // inheriting a template's would manufacture its precondition on every
    // paragraph in every document.
    let sheet = sheet(
        "<w:docDefaults><w:rPrDefault><w:rPr>\
           <w:rFonts w:ascii=\"Calibri\" w:cs=\"Dubai\"/></w:rPr></w:rPrDefault></w:docDefaults>",
    );
    let unit = resolve("", &sheet);
    assert!(unit.props.latin_font.is_unset());
    assert!(!unit.props.complex_font.is_unset());
}

// ---------------------------------------------------------------- named styles

#[test]
fn a_paragraph_style_answers_before_doc_defaults_and_is_cited_by_id() {
    let sheet = sheet(
        "<w:docDefaults><w:pPrDefault><w:pPr><w:bidi w:val=\"0\"/></w:pPr></w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"paragraph\" w:styleId=\"ArabicBody\">\
           <w:pPr><w:bidi/></w:pPr></w:style>",
    );
    let unit = resolve("<w:pPr><w:pStyle w:val=\"ArabicBody\"/></w:pPr>", &sheet);

    let (direction, origin) = inherited(&unit.props.direction);
    assert_eq!(*direction, Direction::Rtl);
    assert_eq!(origin.property, "style[ArabicBody]/pPr@bidi");
}

#[test]
fn a_style_continues_from_the_one_it_is_based_on() {
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:styleId=\"Normal\">\
           <w:pPr><w:bidi/></w:pPr>\
           <w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Heading1\">\
           <w:basedOn w:val=\"Normal\"/>\
           <w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style>",
    );
    let unit = resolve("<w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>", &sheet);

    assert_eq!(
        inherited(&unit.props.alignment).1.property,
        "style[Heading1]/pPr@jc"
    );
    // The two properties `Heading1` does not state come from the style above
    // it, and each names *that* style rather than the one the paragraph asked
    // for: a reviewer opening `style[Heading1]` would find neither.
    assert_eq!(
        inherited(&unit.props.direction).1.property,
        "style[Normal]/pPr@bidi"
    );
    assert_eq!(
        inherited(&unit.props.complex_font).1.property,
        "style[Normal]/rPr/rFonts@cs"
    );
}

#[test]
fn a_based_on_cycle_terminates_at_the_repeat() {
    // A malformed stylesheet is one to report on, not one to hang on.
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:styleId=\"A\">\
           <w:basedOn w:val=\"B\"/><w:pPr><w:bidi/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"B\"><w:basedOn w:val=\"A\"/></w:style>",
    );
    let unit = resolve("<w:pPr><w:pStyle w:val=\"A\"/></w:pPr>", &sheet);
    assert_eq!(*inherited(&unit.props.direction).0, Direction::Rtl);
}

#[test]
fn the_default_paragraph_style_answers_a_paragraph_that_names_none() {
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:pPr><w:bidi/></w:pPr></w:style>",
    );
    assert_eq!(
        inherited(&resolve("", &sheet).props.direction).1.property,
        "style[Normal]/pPr@bidi"
    );
}

#[test]
fn a_paragraph_that_names_a_style_does_not_also_take_the_default_one() {
    // ECMA-376 §17.7.2: the default paragraph style applies to paragraphs that
    // reference no style. A walk that consulted it anyway would report
    // `Normal`'s direction on a paragraph laid out by a style that states
    // none — a value no reader will see.
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:pPr><w:bidi/></w:pPr></w:style>\
         <w:style w:type=\"paragraph\" w:styleId=\"Quote\">\
           <w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style>",
    );
    let unit = resolve("<w:pPr><w:pStyle w:val=\"Quote\"/></w:pPr>", &sheet);
    assert_eq!(*inherited(&unit.props.alignment).0, Alignment::Center);
    assert!(unit.props.direction.is_unset());
}

#[test]
fn only_a_paragraph_style_can_be_the_documents_default_one() {
    let sheet = sheet(
        "<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\">\
           <w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr></w:style>",
    );
    // The character style is still reachable by name; it is simply not what a
    // paragraph naming no style falls back to.
    assert!(resolve("", &sheet).props.complex_font.is_unset());
    let unit = resolve_with_run(
        "",
        "<w:rPr><w:rStyle w:val=\"DefaultParagraphFont\"/></w:rPr>",
        &sheet,
    );
    assert_eq!(*inherited(&unit.props.complex_font).0, "Dubai");
}

#[test]
fn a_character_style_answers_before_the_paragraph_style() {
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:rPr><w:rFonts w:cs=\"Times New Roman\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"ArabicRun\">\
           <w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr></w:style>",
    );
    let unit = resolve_with_run("", "<w:rPr><w:rStyle w:val=\"ArabicRun\"/></w:rPr>", &sheet);
    assert_eq!(
        inherited(&unit.props.complex_font).1.property,
        "style[ArabicRun]/rPr/rFonts@cs"
    );
}

#[test]
fn a_link_between_the_halves_of_one_style_is_not_a_hop() {
    // Word writes the run properties into *both* halves of a linked style.
    // Following `w:link` would resolve a value that is already stated where
    // the walk is looking, and on a document whose halves disagree it would
    // prefer the half Word does not apply.
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:styleId=\"Heading1\">\
           <w:link w:val=\"Heading1Char\"/><w:pPr><w:bidi/></w:pPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"Heading1Char\">\
           <w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr></w:style>",
    );
    let unit = resolve("<w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>", &sheet);
    assert_eq!(*inherited(&unit.props.direction).0, Direction::Rtl);
    assert!(unit.props.complex_font.is_unset());
}

#[test]
fn a_style_with_no_id_is_reachable_by_nothing_and_supplies_nothing() {
    let sheet = sheet("<w:style w:type=\"paragraph\"><w:pPr><w:bidi/></w:pPr></w:style>");
    assert!(sheet.is_empty());
    assert!(resolve("", &sheet).props.direction.is_unset());
}

// ------------------------------------------------------ what is not a paragraph

#[test]
fn a_table_styles_own_alignment_is_not_the_paragraphs() {
    // `w:tblPr` carries a `w:jc` of its own and `w:tblStylePr` a whole `w:pPr`,
    // and both sit inside a `w:style`. A reader matching on element names
    // alone would report a table's alignment as a paragraph's, and the
    // conditional formatting of a header row as the style's own.
    let sheet = sheet(
        "<w:style w:type=\"table\" w:styleId=\"Grid\">\
           <w:tblPr><w:jc w:val=\"center\"/><w:bidiVisual/></w:tblPr>\
           <w:tblStylePr w:type=\"firstRow\">\
             <w:pPr><w:bidi/><w:jc w:val=\"right\"/></w:pPr>\
             <w:rPr><w:rFonts w:cs=\"Dubai\"/></w:rPr>\
           </w:tblStylePr>\
         </w:style>",
    );

    let unit = resolve("<w:pPr><w:pStyle w:val=\"Grid\"/></w:pPr>", &sheet);
    assert!(unit.props.direction.is_unset());
    assert!(unit.props.alignment.is_unset());
    assert!(unit.props.complex_font.is_unset());

    // The one thing that `w:tblPr` does state is the table's own column
    // order, and it reaches a table rather than a paragraph.
    let mut direction = Resolved::Unset;
    sheet.resolve_table(Some("Grid"), &mut direction);
    assert_eq!(*inherited(&direction).0, Direction::Rtl);
}

#[test]
fn a_drawingml_style_source_in_a_word_part_supplies_nothing() {
    // The mirror of `token.rs`'s claim, run in this direction: an `a:lvl1pPr`
    // is PowerPoint's style vocabulary, and a reader that answered it here
    // would be one module reading both formats through one set of names.
    let sheet = StyleSheet::parse(
        STYLES,
        &format!(
            "<w:styles{W}{A}><w:style w:type=\"paragraph\" w:styleId=\"Normal\">\
               <a:lvl1pPr rtl=\"1\" algn=\"r\"/></w:style></w:styles>"
        ),
    )
    .expect("the stylesheet did not parse");
    assert!(sheet.is_empty());
}

// ----------------------------------------------------------------- theme fonts

#[test]
fn a_theme_reference_in_a_style_resolves_and_names_the_theme() {
    // The theme is named rather than `styles.xml`, because the theme is where
    // the typeface a reader will see is written (invariant 6). A finding
    // citing `styles.xml minorBidi` sends a reviewer to a file that names no
    // font.
    let scheme = FontScheme::parse(THEME, &theme("Sakkal Majalla", "Dubai")).unwrap();
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:rPr><w:rFonts w:cstheme=\"minorBidi\"/></w:rPr></w:style>",
    )
    .with_theme(THEME, scheme);

    let unit = resolve("", &sheet);
    let (font, origin) = inherited(&unit.props.complex_font);
    assert_eq!(font, "Dubai");
    assert_eq!(origin.part, THEME);
    assert_eq!(origin.property, "fontScheme/minorFont/cs@typeface");
}

#[test]
fn a_theme_reference_a_paragraph_writes_itself_resolves_the_same_way() {
    let scheme = FontScheme::parse(THEME, &theme("Sakkal Majalla", "Dubai")).unwrap();
    let sheet = sheet("").with_theme(THEME, scheme);

    let unit = resolve_with_run(
        "",
        "<w:rPr><w:rFonts w:cstheme=\"majorBidi\"/></w:rPr>",
        &sheet,
    );
    let (font, origin) = inherited(&unit.props.complex_font);
    assert_eq!(font, "Sakkal Majalla");
    assert_eq!(origin.property, "fontScheme/majorFont/cs@typeface");
}

#[test]
fn a_theme_reference_wins_over_the_name_cached_beside_it_and_falls_back_to_it() {
    // Word writes both: `@w:cstheme` is what it renders, and `@w:cs` is the
    // resolved value it caches for consumers without theme support.
    let with_theme = sheet("").with_theme(
        THEME,
        FontScheme::parse(THEME, &theme("Sakkal Majalla", "Dubai")).unwrap(),
    );
    let unit = resolve_with_run(
        "",
        "<w:rPr><w:rFonts w:cs=\"Arial\" w:cstheme=\"minorBidi\"/></w:rPr>",
        &with_theme,
    );
    assert_eq!(*inherited(&unit.props.complex_font).0, "Dubai");

    // With no theme part in the package, the cached name is what a reader
    // actually gets — and it is the paragraph's own statement, not an
    // inherited one.
    let unit = resolve_with_run(
        "",
        "<w:rPr><w:rFonts w:cs=\"Arial\" w:cstheme=\"minorBidi\"/></w:rPr>",
        &sheet(""),
    );
    assert_eq!(unit.props.complex_font, Resolved::Explicit("Arial".into()));
}

#[test]
fn a_theme_slot_naming_no_typeface_resolves_to_nothing() {
    // The stock Office theme states `<a:cs typeface=""/>`, which is a theme
    // naming no complex-script font. Reporting the empty string as a font
    // would claim a typeface nobody has.
    let scheme = FontScheme::parse(THEME, &theme("", "")).unwrap();
    let sheet = sheet(
        "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
           <w:rPr><w:rFonts w:cstheme=\"minorBidi\"/></w:rPr></w:style>",
    )
    .with_theme(THEME, scheme);
    assert!(resolve("", &sheet).props.complex_font.is_unset());
}

#[test]
fn a_theme_slot_name_is_never_mistaken_for_a_typeface() {
    // `minorBidi` names a slot. A reader that took it for a font would put
    // `complex_font: "minorBidi"` in a report, which names no font anyone has
    // — the WordprocessingML spelling of the `+mn-cs` defect.
    let sheet = sheet("");
    let unit = resolve_with_run(
        "",
        "<w:rPr><w:rFonts w:cstheme=\"minorBidi\"/></w:rPr>",
        &sheet,
    );
    assert!(unit.props.complex_font.is_unset());
}

// ---------------------------------------------------------------------- lists

#[test]
fn a_list_supplied_by_a_style_silences_the_literal_bullet_rule() {
    // Word's own list styles carry `w:numPr`, so a paragraph in one has a real
    // list even though its `w:pPr` says nothing about one. Reporting a typed
    // glyph there would be invariant 2 — a rule firing on formatting the
    // author chose — reached through the chain.
    let source = "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\">\
                    <w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"3\"/></w:numPr></w:pPr>\
                  </w:style>";
    let p_pr = "<w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:bidi/></w:pPr>";
    let xml = format!(
        "<w:document{W}><w:body><w:p>{p_pr}<w:r><w:t>• {ARABIC}</w:t></w:r></w:p></w:body></w:document>"
    );

    let resolved = scan_xml_with(PART, &xml, Some(&sheet(source))).unwrap();
    assert_eq!(resolved[0].props.bullet, Bullet::Native);
    assert!(!findings(&resolved).contains(&"literal-bullet".to_string()));

    // Without the stylesheet the same paragraph has no list at all, which is
    // the difference this item makes.
    let unresolved = scan_xml(PART, &xml).unwrap();
    assert_eq!(unresolved[0].props.bullet, Bullet::None);
    assert!(findings(&unresolved).contains(&"literal-bullet".to_string()));
}

#[test]
fn a_paragraph_that_removes_its_list_has_none_to_inherit() {
    // `w:numId w:val="0"` says the opposite of the element enclosing it: it
    // *removes* the list the style supplies. A paragraph that suppressed its
    // list and then typed a glyph is exactly the defect the rule reports.
    let source = "<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\">\
                    <w:pPr><w:numPr><w:numId w:val=\"3\"/></w:numPr></w:pPr></w:style>";
    let p_pr = "<w:pPr><w:pStyle w:val=\"ListParagraph\"/><w:bidi/>\
                <w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"0\"/></w:numPr></w:pPr>";
    let xml = format!(
        "<w:document{W}><w:body><w:p>{p_pr}<w:r><w:t>• {ARABIC}</w:t></w:r></w:p></w:body></w:document>"
    );

    let units = scan_xml_with(PART, &xml, Some(&sheet(source))).unwrap();
    assert_eq!(units[0].props.bullet, Bullet::Suppressed);
    assert!(findings(&units).contains(&"literal-bullet".to_string()));
}

// -------------------------------------------------------------- table styles

/// A `word/document.xml` holding one Arabic table over the given `w:tblPr`.
fn table_document(tbl_pr: &str) -> String {
    format!(
        "<w:document{W}><w:body><w:tbl>{tbl_pr}<w:tr><w:tc>\
         <w:p><w:r><w:t>{ARABIC}</w:t></w:r></w:p>\
         </w:tc></w:tr></w:tbl></w:body></w:document>"
    )
}

/// The table unit of such a document, resolved against a stylesheet.
fn resolve_table(tbl_pr: &str, sheet: &StyleSheet) -> TextUnit {
    let xml = table_document(tbl_pr);
    scan_xml_with(PART, &xml, Some(sheet))
        .expect("the part did not parse")
        .into_iter()
        .find(|u| u.kind == mirsam_core::UnitKind::Table)
        .unwrap_or_else(|| panic!("no table unit from {xml}"))
}

#[test]
fn a_table_style_supplies_the_column_order_the_table_did_not_state() {
    // Reading a styled right-to-left table as undeclared would report
    // `container-direction` on every correctly-styled Arabic table in the
    // document — invariant 2 reached through the adapter.
    let sheet = sheet(
        "<w:style w:type=\"table\" w:styleId=\"ArabicGrid\">\
           <w:tblPr><w:bidiVisual/></w:tblPr></w:style>",
    );
    let unit = resolve_table(
        "<w:tblPr><w:tblStyle w:val=\"ArabicGrid\"/></w:tblPr>",
        &sheet,
    );

    let (direction, origin) = inherited(&unit.props.direction);
    assert_eq!(*direction, Direction::Rtl);
    assert_eq!(origin.part, STYLES);
    assert_eq!(origin.property, "style[ArabicGrid]/tblPr@bidiVisual");
    assert!(!findings(&[unit]).contains(&"container-direction".to_string()));
}

#[test]
fn a_table_that_states_its_own_column_order_does_not_take_the_styles() {
    let sheet = sheet(
        "<w:style w:type=\"table\" w:styleId=\"LatinGrid\">\
           <w:tblPr><w:bidiVisual w:val=\"0\"/></w:tblPr></w:style>",
    );
    let unit = resolve_table(
        "<w:tblPr><w:tblStyle w:val=\"LatinGrid\"/><w:bidiVisual/></w:tblPr>",
        &sheet,
    );
    assert_eq!(unit.props.direction, Resolved::Explicit(Direction::Rtl));
}

#[test]
fn a_style_chain_answers_the_table_and_a_contradicting_answer_is_still_reported() {
    // The `w:basedOn` walk is the same one a paragraph takes, and the style
    // cited is the one that actually stated the value.
    let sheet = sheet(
        "<w:style w:type=\"table\" w:styleId=\"Base\">\
           <w:tblPr><w:bidiVisual w:val=\"0\"/></w:tblPr></w:style>\
         <w:style w:type=\"table\" w:styleId=\"Grid\">\
           <w:basedOn w:val=\"Base\"/></w:style>",
    );
    let unit = resolve_table("<w:tblPr><w:tblStyle w:val=\"Grid\"/></w:tblPr>", &sheet);

    let (direction, origin) = inherited(&unit.props.direction);
    assert_eq!(*direction, Direction::Ltr);
    assert_eq!(origin.property, "style[Base]/tblPr@bidiVisual");
    // A style that lays its tables out left to right is a default nobody
    // aimed at Arabic, and the reader still meets the columns reversed
    // (ADR 0007 §1).
    assert!(findings(&[unit]).contains(&"container-direction".to_string()));
}

#[test]
fn a_table_naming_no_style_takes_the_documents_default_table_style() {
    let sheet = sheet(
        "<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\">\
           <w:tblPr><w:bidiVisual/></w:tblPr></w:style>",
    );
    assert_eq!(
        *inherited(&resolve_table("", &sheet).props.direction).0,
        Direction::Rtl
    );
}

#[test]
fn a_table_style_answers_the_paragraphs_in_its_cells_below_their_own_style() {
    // Word's hierarchy puts a table style under the paragraph style and over
    // `w:docDefaults` (§17.7.2). A cell paragraph that names neither takes the
    // table style's, so an Arabic cell under a right-to-left table style is
    // the design doing its job and is silent.
    let sheet = sheet(
        "<w:docDefaults><w:pPrDefault><w:pPr><w:jc w:val=\"left\"/></w:pPr>\
         </w:pPrDefault></w:docDefaults>\
         <w:style w:type=\"table\" w:styleId=\"ArabicGrid\">\
           <w:tblPr><w:bidiVisual/></w:tblPr>\
           <w:pPr><w:bidi/><w:jc w:val=\"right\"/></w:pPr></w:style>",
    );
    let xml = table_document("<w:tblPr><w:tblStyle w:val=\"ArabicGrid\"/></w:tblPr>");
    let units = scan_xml_with(PART, &xml, Some(&sheet)).unwrap();
    let cell = units
        .iter()
        .find(|u| u.kind == mirsam_core::UnitKind::Paragraph)
        .unwrap();

    let (direction, origin) = inherited(&cell.props.direction);
    assert_eq!(*direction, Direction::Rtl);
    assert_eq!(origin.property, "style[ArabicGrid]/pPr@bidi");
    // The table style is nearer than `w:docDefaults`, which says the opposite
    // edge.
    assert_eq!(*inherited(&cell.props.alignment).0, Alignment::End);
    // Nothing about direction is left to report, on the cell or the table.
    let found = findings(&units);
    assert!(
        !found.iter().any(|r| r.starts_with("direction-")),
        "{found:?}"
    );
    assert!(
        !found.contains(&"container-direction".to_string()),
        "{found:?}"
    );
    assert!(!found.contains(&"alignment-unset".to_string()), "{found:?}");
}

#[test]
fn a_paragraph_outside_every_table_takes_no_table_style() {
    // Otherwise the document's default table style would format every
    // paragraph in the document, which is a value no reader will see.
    let sheet = sheet(
        "<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\">\
           <w:pPr><w:bidi/></w:pPr></w:style>",
    );
    assert!(resolve("", &sheet).props.direction.is_unset());
}

// -------------------------------------------------------------- the package

/// Write a minimal Word package around whatever parts a case needs.
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

/// One `<Relationship>` of a standard type.
fn entry(id: &str, kind: &str, target: &str) -> String {
    format!(
        "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
    )
}

fn rels(entries: &str) -> String {
    format!(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{entries}</Relationships>"
    )
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("mirsam-style-{name}-{}", std::process::id()));
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
fn a_scan_resolves_every_part_against_the_stylesheet_the_relationships_name() {
    // Both the stylesheet and the theme are reached by the relationship that
    // points at them, never by their conventional path — so they are stored
    // here under names no reader could guess. A reader hard-coding
    // `word/styles.xml` resolves nothing on this package and reports every
    // paragraph in it undeclared.
    let scratch = Scratch::new("by-relationship");
    let path = package(
        &scratch.0,
        &[
            (
                "_rels/.rels",
                &rels(&entry("rId1", "officeDocument", "word/main.xml")),
            ),
            (
                "word/_rels/main.xml.rels",
                &rels(&format!(
                    "{}{}{}",
                    entry("rId1", "styles", "look-here.xml"),
                    entry("rId2", "theme", "../t/one.xml"),
                    entry("rId3", "header", "head.xml"),
                )),
            ),
            ("word/main.xml", &document("")),
            ("word/head.xml", &document("")),
            (
                "word/look-here.xml",
                &styles(
                    "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">\
                       <w:pPr><w:bidi/></w:pPr>\
                       <w:rPr><w:rFonts w:cstheme=\"minorBidi\"/></w:rPr></w:style>",
                ),
            ),
            ("t/one.xml", &theme("Sakkal Majalla", "Dubai")),
        ],
    );

    let mut doc = DocxDocument::open(&path).expect("the package did not open");
    let units = doc.scan().expect("the scan failed");
    assert_eq!(
        units.len(),
        2,
        "the body and the header are both text parts"
    );

    for unit in &units {
        let (direction, origin) = inherited(&unit.props.direction);
        assert_eq!(*direction, Direction::Rtl);
        assert_eq!(origin.part, "word/look-here.xml");
        assert_eq!(origin.property, "style[Normal]/pPr@bidi");

        let (font, origin) = inherited(&unit.props.complex_font);
        assert_eq!(font, "Dubai");
        assert_eq!(origin.part, "t/one.xml");
    }
    // Nothing is reported about the two properties the chain answered. The
    // language tag is deliberately never resolved (ADR 0007), and no source
    // here states a `w:jc`, so those two notes are the honest remainder.
    let mut reported = findings(&units);
    reported.sort();
    reported.dedup();
    assert_eq!(reported, ["alignment-unset", "language-missing"]);
}

#[test]
fn a_package_naming_no_stylesheet_resolves_nothing_rather_than_failing() {
    let scratch = Scratch::new("no-styles");
    let path = package(
        &scratch.0,
        &[
            (
                "_rels/.rels",
                &rels(&entry("rId1", "officeDocument", "word/document.xml")),
            ),
            (PART, &document("<w:pPr><w:bidi/></w:pPr>")),
        ],
    );

    let mut doc = DocxDocument::open(&path).expect("the package did not open");
    let units = doc.scan().expect("the scan failed");
    assert_eq!(units.len(), 1);
    assert_eq!(units[0].props.direction, Resolved::Explicit(Direction::Rtl));
    assert!(units[0].props.complex_font.is_unset());
}
