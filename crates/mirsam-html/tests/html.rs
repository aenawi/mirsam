//! What the HTML adapter reads (PLAN §5.1).
//!
//! These are the format's own questions — does a `dir` on `<body>` reach a
//! paragraph, does the stylesheet beat it, is a `<table>` a container. The
//! question of whether the *answers* agree with the other adapters' is
//! `mirsam-conformance`'s, and is asked in the shared model's vocabulary
//! there rather than in HTML's here.

use mirsam_core::{
    Alignment, Bullet, Direction, DocumentReader, Inset, Resolved, SpanBidi, TextUnit, UnitKind,
    text::Origin,
};
use mirsam_html::HtmlDocument;

const ARABIC: &str = "ارتفع الأداء في الربع الرابع";

fn scan(html: &str) -> Vec<TextUnit> {
    HtmlDocument::from_source("page.html", html)
        .scan()
        .expect("HTML always parses")
}

/// The units a document produces, by the text they carry.
fn unit<'a>(units: &'a [TextUnit], text: &str) -> &'a TextUnit {
    units
        .iter()
        .find(|unit| unit.text == text)
        .unwrap_or_else(|| panic!("no unit saying {text:?} in {:?}", texts(units)))
}

fn texts(units: &[TextUnit]) -> Vec<&str> {
    units.iter().map(|unit| unit.text.as_str()).collect()
}

fn origin(resolved: &Resolved<Direction>) -> &Origin {
    resolved
        .origin()
        .expect("an inherited value names its source")
}

// ------------------------------------------------------------ what is a unit

#[test]
fn a_block_with_text_is_a_paragraph() {
    let units = scan("<p>first</p><p>second</p>");
    assert_eq!(texts(&units), ["first", "second"]);
    assert!(units.iter().all(|unit| unit.kind == UnitKind::Paragraph));
}

#[test]
fn a_blocks_own_text_stops_at_the_next_block() {
    // Two boxes on the screen: the div's own line, and the paragraph under it.
    let units = scan("<div>outer <b>bold</b><p>inner</p></div>");
    assert_eq!(texts(&units), ["outer bold", "inner"]);
}

#[test]
fn inline_markup_does_not_split_a_word() {
    let units = scan("<p>الرب<span>ع</span> الرابع</p>");
    assert_eq!(texts(&units), ["الربع الرابع"]);
}

#[test]
fn whitespace_collapses_the_way_css_collapses_it() {
    let units = scan("<p>one   \n  two\t three</p>");
    assert_eq!(texts(&units), ["one two three"]);
}

#[test]
fn a_pre_keeps_the_whitespace_it_was_given() {
    let units = scan("<pre>one   two</pre>");
    assert_eq!(texts(&units), ["one   two"]);
}

#[test]
fn a_line_break_separates_the_words_either_side_of_it() {
    let units = scan("<p>one<br>two</p>");
    assert_eq!(texts(&units), ["one two"]);
}

#[test]
fn script_and_style_are_not_prose() {
    let units = scan("<style>p{direction:rtl}</style><script>var s = 'نص';</script><p>real</p>");
    assert_eq!(texts(&units), ["real"]);
}

#[test]
fn the_title_is_text_a_reader_sees() {
    let units = scan("<html><head><title>تقرير</title></head><body><p>body</p></body></html>");
    assert_eq!(texts(&units), ["تقرير", "body"]);
}

#[test]
fn metadata_in_the_head_is_not_text() {
    let units =
        scan(r#"<head><meta name="description" content="وصف"></head><body><p>x</p></body>"#);
    assert_eq!(texts(&units), ["x"]);
}

#[test]
fn an_empty_block_produces_nothing() {
    let units = scan("<div></div><p>   </p><p>text</p>");
    assert_eq!(texts(&units), ["text"]);
}

// ------------------------------------------------------------------ direction

#[test]
fn dir_on_the_paragraph_is_explicit() {
    let units = scan(&format!(r#"<p dir="rtl">{ARABIC}</p>"#));
    assert_eq!(
        unit(&units, ARABIC).props.direction,
        Resolved::Explicit(Direction::Rtl)
    );
}

#[test]
fn dir_on_an_ancestor_is_inherited_and_names_it() {
    let units = scan(&format!(r#"<body dir="rtl"><p>{ARABIC}</p></body>"#));
    let direction = &unit(&units, ARABIC).props.direction;
    assert_eq!(direction.effective(), Some(&Direction::Rtl));
    assert!(direction.is_inherited());
    assert_eq!(origin(direction).property, "body@");
}

#[test]
fn the_nearest_ancestor_that_states_a_direction_wins() {
    let units = scan(&format!(
        r#"<body dir="ltr"><div id="main" dir="rtl"><p>{ARABIC}</p></div></body>"#
    ));
    let direction = &unit(&units, ARABIC).props.direction;
    assert_eq!(direction.effective(), Some(&Direction::Rtl));
    assert_eq!(origin(direction).property, "div#main@");
}

#[test]
fn nothing_anywhere_is_unset() {
    let units = scan(&format!("<p>{ARABIC}</p>"));
    assert!(unit(&units, ARABIC).props.direction.is_unset());
}

/// `dir="auto"` asks the browser to guess from the first strong character,
/// which is what `Unset` means. See the module documentation in `html.rs`.
#[test]
fn dir_auto_states_no_direction() {
    let units = scan(&format!(r#"<p dir="auto">{ARABIC}</p>"#));
    assert!(unit(&units, ARABIC).props.direction.is_unset());
}

/// `auto` replaces the inherited direction rather than deferring to it, so a
/// paragraph under an `ltr` ancestor is not reported as contradicted by a
/// chain the browser has already stopped consulting.
#[test]
fn dir_auto_stops_the_inheritance_it_overrides() {
    let units = scan(&format!(
        r#"<body dir="ltr"><p dir="auto">{ARABIC}</p></body>"#
    ));
    assert!(unit(&units, ARABIC).props.direction.is_unset());
}

#[test]
fn a_stylesheet_still_beats_dir_auto() {
    let units = scan(&format!(
        r#"<style>p {{ direction: rtl }}</style><p dir="auto">{ARABIC}</p>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

#[test]
fn a_stylesheet_states_direction_and_is_cited_as_the_source() {
    let units = scan(&format!(
        r#"<style>body {{ direction: rtl; }}</style><body><p>{ARABIC}</p></body>"#
    ));
    let direction = &unit(&units, ARABIC).props.direction;
    assert_eq!(direction.effective(), Some(&Direction::Rtl));
    assert!(direction.is_inherited());
    assert_eq!(origin(direction).property, "body{direction}");
}

#[test]
fn a_rule_matching_the_paragraph_itself_is_still_the_stylesheets_decision() {
    // A stylesheet rule is a decision taken elsewhere about this element,
    // which is the relation a Word named style has to a paragraph (ADR 0007).
    let units = scan(&format!(
        r#"<style>.rtl {{ direction: rtl }}</style><p class="rtl">{ARABIC}</p>"#
    ));
    let direction = &unit(&units, ARABIC).props.direction;
    assert!(direction.is_inherited());
    assert_eq!(origin(direction).property, ".rtl{direction}");
}

#[test]
fn an_author_stylesheet_beats_the_dir_attribute() {
    // What the reader sees is what the tool must report.
    let units = scan(&format!(
        r#"<style>p {{ direction: ltr }}</style><p dir="rtl">{ARABIC}</p>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Ltr)
    );
}

#[test]
fn a_style_attribute_beats_the_stylesheet() {
    let units = scan(&format!(
        r#"<style>p {{ direction: ltr }}</style><p style="direction: rtl">{ARABIC}</p>"#
    ));
    let direction = &unit(&units, ARABIC).props.direction;
    assert_eq!(direction, &Resolved::Explicit(Direction::Rtl));
}

#[test]
fn specificity_decides_between_two_rules() {
    let units = scan(&format!(
        r#"<style>p {{ direction: ltr }} body p.lead {{ direction: rtl }}</style>
           <body><p class="lead">{ARABIC}</p></body>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

#[test]
fn important_outranks_specificity() {
    let units = scan(&format!(
        r#"<style>body p#x {{ direction: ltr }} p {{ direction: rtl !important }}</style>
           <body><p id="x">{ARABIC}</p></body>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

#[test]
fn a_declaration_inside_a_media_query_is_not_applied() {
    // It applies under a viewport this tool does not have.
    let units = scan(&format!(
        r#"<style>@media print {{ p {{ direction: rtl }} }}</style><p>{ARABIC}</p>"#
    ));
    assert!(unit(&units, ARABIC).props.direction.is_unset());
}

#[test]
fn a_descendant_selector_matches_through_the_chain() {
    let units = scan(&format!(
        r#"<style>#main p {{ direction: rtl }}</style>
           <div id="main"><section><p>{ARABIC}</p></section></div>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

#[test]
fn a_child_selector_does_not_match_a_grandchild() {
    let units = scan(&format!(
        r#"<style>#main > p {{ direction: rtl }}</style>
           <div id="main"><section><p>{ARABIC}</p></section></div>"#
    ));
    assert!(unit(&units, ARABIC).props.direction.is_unset());
}

#[test]
fn an_attribute_selector_matches_the_dir_a_site_styles_on() {
    let units = scan(&format!(
        r#"<style>[dir="rtl"] p {{ text-align: right }}</style>
           <body dir="rtl"><p>{ARABIC}</p></body>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.alignment.effective(),
        Some(&Alignment::Right)
    );
}

#[test]
fn an_unsupported_selector_drops_only_itself() {
    let units = scan(&format!(
        r#"<style>p:hover, p {{ direction: rtl }}</style><p>{ARABIC}</p>"#
    ));
    assert_eq!(
        unit(&units, ARABIC).props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

// ----------------------------------------------------------------- alignment

/// CSS `left` is a *physical* edge, unlike Word's `w:jc`, so HTML can state
/// the hard left on Arabic that `alignment-incoherent` reports.
#[test]
fn text_align_left_is_the_physical_left() {
    let units = scan(&format!(r#"<p style="text-align: left">{ARABIC}</p>"#));
    assert_eq!(
        unit(&units, ARABIC).props.alignment,
        Resolved::Explicit(Alignment::Left)
    );
}

#[test]
fn text_align_start_stays_direction_relative() {
    let units = scan(&format!(r#"<p style="text-align: start">{ARABIC}</p>"#));
    assert_eq!(
        unit(&units, ARABIC).props.alignment,
        Resolved::Explicit(Alignment::Start)
    );
}

#[test]
fn the_align_attribute_is_a_presentational_hint() {
    let units = scan(&format!(r#"<p align="center">{ARABIC}</p>"#));
    assert_eq!(
        unit(&units, ARABIC).props.alignment.effective(),
        Some(&Alignment::Center)
    );
}

// ------------------------------------------------------------------ language

#[test]
fn lang_is_inherited_from_the_html_element() {
    let units = scan(&format!(
        r#"<html lang="ar-SA"><body><p>{ARABIC}</p></body></html>"#
    ));
    let language = &unit(&units, ARABIC).props.language;
    assert_eq!(language.effective().map(String::as_str), Some("ar-SA"));
    assert!(language.is_inherited());
}

#[test]
fn lang_on_the_paragraph_is_explicit() {
    let units = scan(&format!(r#"<p lang="ar">{ARABIC}</p>"#));
    assert!(unit(&units, ARABIC).props.language.is_explicit());
}

// --------------------------------------------------------------------- fonts

/// CSS has one font stack per element, not OOXML's Latin/complex pair, so it
/// answers for both slots and `complex-font-missing` cannot fire on HTML.
#[test]
fn one_font_stack_answers_for_both_slots() {
    let units = scan(&format!(
        r#"<p style="font-family: 'Dubai', Arial, sans-serif">{ARABIC}</p>"#
    ));
    let props = &unit(&units, ARABIC).props;
    assert_eq!(
        props.complex_font.effective().map(String::as_str),
        Some("Dubai")
    );
    assert_eq!(
        props.latin_font.effective().map(String::as_str),
        Some("Dubai")
    );
}

#[test]
fn a_font_set_on_the_body_reaches_the_paragraph() {
    let units = scan(&format!(
        r#"<style>body {{ font-family: Dubai }}</style><body><p>{ARABIC}</p></body>"#
    ));
    assert_eq!(
        unit(&units, ARABIC)
            .props
            .complex_font
            .effective()
            .map(String::as_str),
        Some("Dubai")
    );
}

// ------------------------------------------------------------------- bullets

#[test]
fn a_list_item_carries_a_native_marker() {
    let units = scan("<ul><li>item</li></ul>");
    assert_eq!(unit(&units, "item").props.bullet, Bullet::Native);
}

#[test]
fn list_style_none_suppresses_the_marker() {
    let units = scan("<style>ul { list-style-type: none }</style><ul><li>item</li></ul>");
    assert_eq!(unit(&units, "item").props.bullet, Bullet::Suppressed);
}

#[test]
fn a_paragraph_is_not_a_list_item() {
    let units = scan("<p>text</p>");
    assert_eq!(unit(&units, "text").props.bullet, Bullet::None);
}

// ---------------------------------------------------------------- containers

#[test]
fn a_table_is_a_container_beside_the_paragraphs_in_it() {
    let units = scan(r#"<table dir="rtl"><tr><td>واحد</td><td>اثنان</td></tr></table>"#);
    let table = units
        .iter()
        .find(|unit| unit.kind == UnitKind::Table)
        .expect("the table is a unit");
    assert_eq!(table.id.to_string(), "page.html#tbl1");
    assert_eq!(table.props.direction, Resolved::Explicit(Direction::Rtl));
    assert!(table.text.contains("واحد") && table.text.contains("اثنان"));
    assert_eq!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Paragraph)
            .count(),
        2
    );
}

#[test]
fn a_cell_paragraph_says_which_cell_it_is_in() {
    let units = scan("<table><tr><td>a</td></tr><tr><td>b</td><td>c</td></tr></table>");
    assert_eq!(
        unit(&units, "c").location.container.as_deref(),
        Some("table 1 row 2 cell 2")
    );
}

#[test]
fn a_paragraph_outside_a_table_is_located_by_its_element() {
    let units = scan(r#"<div id="lead"><p class="intro">text</p></div>"#);
    assert_eq!(
        unit(&units, "text").location.container.as_deref(),
        Some("p.intro")
    );
}

#[test]
fn columns_are_a_container() {
    let units = scan(&format!(r#"<div style="column-count: 2">{ARABIC}</div>"#));
    let columns = units
        .iter()
        .find(|unit| unit.kind == UnitKind::Columns)
        .expect("a two-column box is a container");
    assert_eq!(columns.id.to_string(), "page.html#cols1");
}

#[test]
fn one_column_is_not_a_container() {
    let units = scan(&format!(r#"<div style="column-count: 1">{ARABIC}</div>"#));
    assert!(units.iter().all(|unit| unit.kind != UnitKind::Columns));
}

// ------------------------------------------------------------- ids and parts

#[test]
fn units_are_numbered_within_the_part() {
    let units = scan("<p>one</p><p>two</p>");
    assert_eq!(unit(&units, "one").id.to_string(), "page.html#p1");
    assert_eq!(unit(&units, "two").id.to_string(), "page.html#p2");
    assert_eq!(unit(&units, "two").location.paragraph, Some(2));
    assert_eq!(unit(&units, "two").location.part, "page.html");
}

#[test]
fn the_format_names_itself() {
    assert_eq!(HtmlDocument::from_source("page.html", "").format(), "html");
}

// -------------------------------------------------------- broken markup

/// The tree a browser builds, not the one the tags suggest: `<p>` cannot
/// nest, so the second closes the first and the two are siblings.
#[test]
fn tree_construction_is_the_browsers_and_not_the_tags() {
    let units = scan(r#"<body dir="rtl"><p>one<p>two</body>"#);
    assert_eq!(texts(&units), ["one", "two"]);
    for text in ["one", "two"] {
        assert_eq!(
            unit(&units, text).props.direction.effective(),
            Some(&Direction::Rtl)
        );
    }
}

#[test]
fn an_unclosed_document_still_has_a_tree() {
    let units = scan("<div dir=\"rtl\"><p>text");
    assert_eq!(
        unit(&units, "text").props.direction.effective(),
        Some(&Direction::Rtl)
    );
}

// ------------------------------------------------------------- inline runs

/// The runs a paragraph reports, as `(text, bidi, origin property)`.
fn runs<'a>(units: &'a [TextUnit], text: &str) -> Vec<(&'a str, SpanBidi, &'a str)> {
    let unit = unit(units, text);
    unit.spans
        .iter()
        .map(|span| {
            (
                span.text(&unit.text).expect("a run inside its own text"),
                span.bidi,
                span.origin.property.as_str(),
            )
        })
        .collect()
}

#[test]
fn an_inline_element_is_a_run_and_says_where_it_starts() {
    let units = scan("<p>الربع <span>الرابع</span></p>");
    assert_eq!(
        runs(&units, "الربع الرابع"),
        [("الرابع", SpanBidi::Plain, "span@")]
    );
}

#[test]
fn a_run_is_located_in_the_text_after_whitespace_collapsed() {
    // The offsets are into the string a finding reports, not the one the file
    // holds: three spaces became one before anybody counted.
    let units = scan("<p>الربع   <b>الرابع</b></p>");
    assert_eq!(
        runs(&units, "الربع الرابع"),
        [("الرابع", SpanBidi::Plain, "b@")]
    );
}

#[test]
fn bdo_imposes_an_order_and_bdi_isolates_one() {
    let units = scan(r#"<p><bdo dir="rtl">أ</bdo><bdi>ب</bdi></p>"#);
    assert_eq!(
        runs(&units, "أب"),
        [
            ("أ", SpanBidi::Imposed(Direction::Rtl), "bdo@"),
            ("ب", SpanBidi::Isolated, "bdi@"),
        ]
    );
}

#[test]
fn any_element_naming_a_direction_is_isolated_the_way_a_browser_isolates_it() {
    let units = scan(r#"<p>الربع <span dir="ltr">Q4</span></p>"#);
    assert_eq!(
        runs(&units, "الربع Q4"),
        [("Q4", SpanBidi::Isolated, "span@")]
    );
}

#[test]
fn an_author_stylesheet_decides_isolation_the_way_it_decides_direction() {
    let units = scan(
        r#"<style>.raw { unicode-bidi: normal }</style><p>الربع <span class="raw" dir="ltr">Q4</span></p>"#,
    );
    // The author's `normal` beats the isolation `dir` would have given it,
    // which is what a browser does and therefore what a reader gets.
    assert_eq!(
        runs(&units, "الربع Q4"),
        [("Q4", SpanBidi::Plain, ".raw{unicode-bidi}")]
    );
}

#[test]
fn an_element_that_laid_out_nothing_delimits_no_run() {
    let units = scan("<p>الربع <span></span><img src=\"x.png\"> الرابع</p>");
    assert!(runs(&units, "الربع الرابع").is_empty());
}

#[test]
fn a_block_inside_an_inline_element_is_its_own_box_and_not_part_of_the_run() {
    let units = scan("<div><span>واحد<p>اثنان</p></span></div>");
    assert_eq!(texts(&units), ["واحد", "اثنان"]);
    assert_eq!(runs(&units, "واحد"), [("واحد", SpanBidi::Plain, "span@")]);
    assert!(runs(&units, "اثنان").is_empty());
}

#[test]
fn a_preformatted_box_of_nothing_but_newlines_is_an_empty_box() {
    // Trimmed away from both ends, so the two offsets cross. A document is not
    // a place to find that out.
    assert!(scan("<pre>\n\n</pre>").is_empty());
    assert!(scan("<pre>\n\n<b>\n</b></pre>").is_empty());
}

#[test]
fn a_preformatted_box_keeps_its_runs_where_the_trimming_left_them() {
    let units = scan("<pre>\nالربع <b>الرابع</b>\n</pre>");
    assert_eq!(
        runs(&units, "الربع الرابع"),
        [("الرابع", SpanBidi::Plain, "b@")]
    );
}

// ------------------------------------------------------------------- insets

fn inset(units: &[TextUnit], text: &str) -> Resolved<Inset> {
    unit(units, text).props.inset.clone()
}

#[test]
fn a_one_sided_physical_margin_is_a_physical_inset() {
    let units = scan(&format!(r#"<p style="margin-left:2rem">{ARABIC}</p>"#));
    assert_eq!(inset(&units, ARABIC), Resolved::Explicit(Inset::Left));
}

#[test]
fn the_logical_property_is_the_logical_edge() {
    let units = scan(&format!(
        r#"<p style="margin-inline-start:2rem">{ARABIC}</p>"#
    ));
    assert_eq!(inset(&units, ARABIC), Resolved::Explicit(Inset::Start));
}

#[test]
fn a_gutter_is_not_an_indent() {
    // Equal on both sides is a page margin, and it looks the same whichever
    // way the text runs. Reporting one would be a finding on a layout.
    let units = scan(&format!(
        r#"<p style="margin-left:2rem;margin-right:2rem">{ARABIC}</p>"#
    ));
    assert!(inset(&units, ARABIC).is_unset());
}

#[test]
fn a_zero_or_unreadable_length_insets_nothing() {
    for value in ["0", "0px", "auto", "calc(1rem + 2px)", "var(--gap)"] {
        let units = scan(&format!(r#"<p style="margin-left:{value}">{ARABIC}</p>"#));
        assert!(inset(&units, ARABIC).is_unset(), "{value:?}");
    }
}

#[test]
fn an_inset_belongs_to_the_box_that_states_it() {
    // Margins are not inherited, so a wrapper's indent is the wrapper's.
    let units = scan(&format!(
        r#"<div style="margin-left:2rem"><p>{ARABIC}</p></div>"#
    ));
    assert!(inset(&units, ARABIC).is_unset());
}

#[test]
fn a_stylesheet_states_an_inset_and_is_cited_for_it() {
    let units = scan(&format!(
        r#"<style>.note {{ padding-left: 2rem }}</style><p class="note">{ARABIC}</p>"#
    ));
    let inset = inset(&units, ARABIC);
    assert_eq!(inset.effective(), Some(&Inset::Left));
    assert_eq!(
        inset.origin().expect("the stylesheet is named").property,
        ".note{padding-left}"
    );
}

// -------------------------------------------------------- reversed layouts

#[test]
fn a_reversed_flex_row_is_a_container_that_says_what_reversed_it() {
    let units = scan(
        r#"<div style="display:flex;flex-direction:row-reverse"><div>المؤشر</div><div>الربع</div></div>"#,
    );
    let container = units
        .iter()
        .find(|unit| unit.kind == UnitKind::Columns)
        .expect("the reversed row is a container");
    assert_eq!(
        container
            .props
            .reversed
            .as_ref()
            .expect("something reversed it")
            .property,
        "div@"
    );
}

#[test]
fn flex_direction_on_a_box_that_is_not_a_flex_container_does_nothing() {
    // A declaration nobody applied is not a defect: the property has no
    // effect on a block box, and reporting it would report the stylesheet
    // rather than the page.
    let units = scan(r#"<div style="flex-direction:row-reverse"><div>المؤشر</div></div>"#);
    assert!(units.iter().all(|unit| unit.kind != UnitKind::Columns));
}

#[test]
fn a_flex_row_in_the_order_it_stores_is_not_reversed() {
    let units = scan(r#"<div style="display:flex"><div>المؤشر</div><div>الربع</div></div>"#);
    assert!(units.iter().all(|unit| unit.props.reversed.is_none()));
}

#[test]
fn a_box_that_is_both_reversed_and_in_columns_is_still_one_container() {
    let units = scan(
        r#"<div style="display:flex;flex-direction:row-reverse;column-count:2"><div>المؤشر</div></div>"#,
    );
    assert_eq!(
        units
            .iter()
            .filter(|unit| unit.kind == UnitKind::Columns)
            .count(),
        1
    );
}
