//! Does resolving the property chain change what the tool reports, and only
//! in the ways [ADR 0007] decided it should?
//!
//! Two halves. The first builds small packages by hand — a slide, a layout, a
//! master — so that each rule of the walk is exercised in isolation and a
//! failure names the rule that broke. The second runs the same claims against
//! the five corpus decks, two of which PowerPoint itself wrote, because a
//! synthetic package can only prove the walk is the walk that was designed.
//!
//! The load-bearing assertions are PLAN §2.2's acceptance, stated before the
//! code was written:
//!
//! * a deck with direction set only on the master reports no `direction-unset`;
//! * a right-to-left paragraph a layout centres reports no `alignment-unset`,
//!   while one an English layout leaves on the left edge still does — and says
//!   which part left it there.
//!
//! [ADR 0007]: ../../../docs/adr/0007-an-inherited-default-is-not-a-choice.md

use mirsam_core::{Alignment, Direction, Engine, Severity, TextUnit};
use mirsam_ooxml::inherit::{FontScheme, PartStyles, StyleIndex, ThemeFont, ThemeScript};
use mirsam_ooxml::pptx::scan_xml_with;
use mirsam_ooxml::{Package, PptxDocument};
use std::path::{Path, PathBuf};

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

const SLIDE: &str = "ppt/slides/slide1.xml";
const LAYOUT: &str = "ppt/slideLayouts/slideLayout1.xml";
const MASTER: &str = "ppt/slideMasters/slideMaster1.xml";
const THEME: &str = "ppt/theme/theme1.xml";

/// Arabic that reads right-to-left by any measure.
const ARABIC: &str = "التقرير الفصلي";

// ------------------------------------------------------------ hand-built decks

/// One shape, with the placeholder and paragraph markup given.
fn shape(placeholder: &str, body: &str) -> String {
    format!(
        r#"<p:sp><p:nvSpPr><p:cNvPr id="2" name="Title 1"/><p:cNvSpPr/><p:nvPr>{placeholder}</p:nvPr></p:nvSpPr><p:txBody><a:bodyPr/>{body}</p:txBody></p:sp>"#
    )
}

fn slide(placeholder: &str, body: &str) -> String {
    format!(
        r#"<p:sld {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sld>"#,
        shape(placeholder, body)
    )
}

fn layout(placeholder: &str, list_style: &str) -> String {
    format!(
        r#"<p:sldLayout {NS}><p:cSld><p:spTree>{}</p:spTree></p:cSld></p:sldLayout>"#,
        shape(
            placeholder,
            &format!("<a:lstStyle>{list_style}</a:lstStyle>")
        )
    )
}

fn master(text_styles: &str) -> String {
    format!(
        r#"<p:sldMaster {NS}><p:cSld><p:spTree/></p:cSld><p:txStyles>{text_styles}</p:txStyles></p:sldMaster>"#
    )
}

/// A theme stating one typeface in each of its two complex-script slots.
/// `""` is what the stock Office theme states, and means "names none".
fn theme(major_cs: &str, minor_cs: &str) -> String {
    format!(
        r#"<a:theme {NS}><a:themeElements><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface="{major_cs}"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface="{minor_cs}"/></a:minorFont></a:fontScheme></a:themeElements></a:theme>"#
    )
}

/// A slide over a layout over a master, resolved as the package would resolve
/// it: nearest source first, the master's named styles last.
fn deck(slide_xml: &str, layout_xml: &str, master_xml: &str) -> Vec<TextUnit> {
    themed_deck(slide_xml, layout_xml, master_xml, None)
}

/// The same, with the theme its `+mj-`/`+mn-` font references resolve against.
fn themed_deck(
    slide_xml: &str,
    layout_xml: &str,
    master_xml: &str,
    theme_xml: Option<&str>,
) -> Vec<TextUnit> {
    let mut styles = StyleIndex::from_parts(
        [
            (
                SLIDE.to_string(),
                PartStyles::parse(SLIDE, slide_xml).unwrap(),
            ),
            (
                LAYOUT.to_string(),
                PartStyles::parse(LAYOUT, layout_xml).unwrap(),
            ),
            (
                MASTER.to_string(),
                PartStyles::parse(MASTER, master_xml).unwrap(),
            ),
        ],
        [(
            SLIDE.to_string(),
            vec![SLIDE.to_string(), LAYOUT.to_string(), MASTER.to_string()],
        )],
    );
    if let Some(theme_xml) = theme_xml {
        styles = styles.with_theme(
            THEME,
            FontScheme::parse(THEME, theme_xml).unwrap(),
            [SLIDE.to_string(), LAYOUT.to_string(), MASTER.to_string()],
        );
    }
    scan_xml_with(SLIDE, slide_xml, Some(&styles)).unwrap()
}

/// The rules that would report on these units, by id.
fn rules(units: &[TextUnit]) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Engine::with_default_rules()
        .audit(units)
        .diagnostics
        .iter()
        .map(|d| d.rule.0)
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

const ARABIC_MASTER: &str = r#"<p:titleStyle><a:lvl1pPr rtl="1" algn="r"/></p:titleStyle><p:bodyStyle><a:lvl1pPr rtl="1" algn="r"/></p:bodyStyle><p:otherStyle><a:lvl1pPr rtl="1" algn="r"/></p:otherStyle>"#;

const ENGLISH_MASTER: &str = r#"<p:titleStyle><a:lvl1pPr rtl="0" algn="ctr"/></p:titleStyle><p:bodyStyle><a:lvl1pPr rtl="0" algn="l"/></p:bodyStyle><p:otherStyle><a:lvl1pPr rtl="0" algn="l"/></p:otherStyle>"#;

#[test]
fn direction_set_only_on_the_master_silences_direction_unset() {
    // PLAN §2.2's first acceptance. The paragraph states nothing; the master
    // says `rtl="1"`, which agrees with the letters, so the layout is doing
    // its job and there is nothing to report.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!("<a:p><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"),
        ),
        &layout(r#"<p:ph type="body" idx="1"/>"#, "<a:lstStyle/>"),
        &master(ARABIC_MASTER),
    );

    assert_eq!(units.len(), 1);
    assert_eq!(units[0].props.direction.effective(), Some(&Direction::Rtl));
    assert_eq!(
        units[0].props.direction.origin().map(ToString::to_string),
        Some(format!("{MASTER} bodyStyle/lvl1pPr@rtl"))
    );
    assert!(rules(&units).is_empty(), "{:#?}", rules(&units));
}

#[test]
fn a_master_that_contradicts_the_text_keeps_the_finding_and_names_itself() {
    // ADR 0007 §1 and §3: an English template's untouched `rtl="0"` under
    // Arabic is a default nobody aimed at the text, reported exactly as an
    // absent one is — a warning, not an error — and the master is named.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!("<a:p><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"),
        ),
        &layout(r#"<p:ph type="body" idx="1"/>"#, "<a:lstStyle/>"),
        &master(ENGLISH_MASTER),
    );

    let report = Engine::with_default_rules().audit(&units);
    let unset: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule.0 == "direction-unset")
        .collect();
    assert_eq!(unset.len(), 1, "{report:#?}");
    assert_eq!(unset[0].severity, Severity::Warning);
    assert_eq!(
        unset[0].evidence.inherited_from.as_deref(),
        Some(format!("{MASTER} bodyStyle/lvl1pPr@rtl").as_str())
    );
    // Not escalated to an error by resolving the chain (ADR 0007 §3).
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.rule.0 == "direction-mismatch"),
        "{report:#?}"
    );
}

#[test]
fn a_layout_that_centres_silences_alignment_unset() {
    // The second half of #8, and the case ADR 0007 §4 says earns the
    // distinction: a centred title reads correctly in either direction, so
    // `--align` must stop proposing to push it to the right edge.
    let units = deck(
        &slide(
            r#"<p:ph type="title"/>"#,
            &format!(
                "<a:p><a:pPr rtl=\"1\"/><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"
            ),
        ),
        &layout(r#"<p:ph type="title"/>"#, r#"<a:lvl1pPr algn="ctr"/>"#),
        &master(ENGLISH_MASTER),
    );

    assert_eq!(
        units[0].props.alignment.effective(),
        Some(&Alignment::Center)
    );
    assert_eq!(
        units[0].props.alignment.origin().map(ToString::to_string),
        Some(format!("{LAYOUT} ph[type=title]/lstStyle/lvl1pPr@algn"))
    );
    assert!(rules(&units).is_empty(), "{:#?}", rules(&units));
}

#[test]
fn a_layout_that_leaves_it_on_the_left_edge_still_reports() {
    // The other half of #8. Same paragraph, same rule, opposite conclusion,
    // and the finding names the part that put it there.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!(
                "<a:p><a:pPr rtl=\"1\"/><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"
            ),
        ),
        &layout(r#"<p:ph type="body" idx="1"/>"#, r#"<a:lvl1pPr algn="l"/>"#),
        &master(ARABIC_MASTER),
    );

    let report = Engine::with_default_rules().audit(&units);
    let notes: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.rule.0 == "alignment-unset")
        .collect();
    assert_eq!(notes.len(), 1, "{report:#?}");
    assert_eq!(notes[0].severity, Severity::Note);
    assert_eq!(
        notes[0].evidence.inherited_from.as_deref(),
        Some(format!("{LAYOUT} ph[type=body,idx=1]/lstStyle/lvl1pPr@algn").as_str())
    );
}

#[test]
fn the_nearest_source_wins_and_the_layout_is_nearer_than_the_master() {
    // The whole point of walking the chain in order rather than merging it.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!("<a:p><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"),
        ),
        &layout(r#"<p:ph type="body" idx="1"/>"#, r#"<a:lvl1pPr rtl="1"/>"#),
        &master(ENGLISH_MASTER),
    );

    // Direction from the layout, which says `rtl="1"`…
    assert_eq!(
        units[0].props.direction.origin().map(ToString::to_string),
        Some(format!("{LAYOUT} ph[type=body,idx=1]/lstStyle/lvl1pPr@rtl"))
    );
    // …and alignment from the master, which is the only source that states one.
    assert_eq!(
        units[0].props.alignment.origin().map(ToString::to_string),
        Some(format!("{MASTER} bodyStyle/lvl1pPr@algn"))
    );
}

#[test]
fn a_paragraph_that_states_its_own_value_takes_nothing_from_the_chain() {
    // Invariant: resolving the chain never overwrites what the author wrote.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!(
                "<a:p><a:pPr rtl=\"1\" algn=\"r\"/><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"
            ),
        ),
        &layout(
            r#"<p:ph type="body" idx="1"/>"#,
            r#"<a:lvl1pPr rtl="0" algn="l"/>"#,
        ),
        &master(ENGLISH_MASTER),
    );

    assert!(units[0].props.direction.is_explicit());
    assert!(units[0].props.alignment.is_explicit());
    assert!(rules(&units).is_empty(), "{:#?}", rules(&units));
}

#[test]
fn the_placeholder_decides_which_of_the_masters_three_styles_applies() {
    let body = format!("<a:p><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>");
    // `title` and `ctrTitle` are one placeholder under two names; a bare
    // `p:ph` is a body placeholder, because that is the schema's default; and
    // a shape with no `p:ph` at all — a text box — takes `otherStyle`.
    for (placeholder, expected) in [
        (r#"<p:ph type="title"/>"#, "titleStyle"),
        (r#"<p:ph type="ctrTitle"/>"#, "titleStyle"),
        (r#"<p:ph type="body" idx="1"/>"#, "bodyStyle"),
        (r#"<p:ph type="subTitle" idx="1"/>"#, "bodyStyle"),
        (r#"<p:ph idx="1"/>"#, "bodyStyle"),
        (r#"<p:ph type="ftr" idx="11"/>"#, "otherStyle"),
        ("", "otherStyle"),
    ] {
        let units = deck(
            &slide(placeholder, &body),
            &layout(r#"<p:ph type="title"/>"#, "<a:lstStyle/>"),
            &master(ENGLISH_MASTER),
        );
        assert_eq!(
            units[0].props.direction.origin().map(ToString::to_string),
            Some(format!("{MASTER} {expected}/lvl1pPr@rtl")),
            "{placeholder}"
        );
    }
}

#[test]
fn a_notes_master_answers_every_placeholder_with_its_one_style() {
    // A notes master has no title/body/other split: `p:notesStyle` governs
    // everything it lays out, and a notes slide's body placeholder resolves
    // against it rather than falling through to nothing.
    const NOTES_SLIDE: &str = "ppt/notesSlides/notesSlide1.xml";
    const NOTES_MASTER: &str = "ppt/notesMasters/notesMaster1.xml";

    let notes_master = format!(
        r#"<p:notesMaster {NS}><p:cSld><p:spTree/></p:cSld><p:notesStyle><a:lvl1pPr rtl="0" algn="l"/></p:notesStyle></p:notesMaster>"#
    );
    let notes_slide = slide(
        r#"<p:ph type="body" sz="quarter" idx="3"/>"#,
        &format!(
            "<a:p><a:pPr rtl=\"1\"/><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"
        ),
    );

    let styles = StyleIndex::from_parts(
        [(
            NOTES_MASTER.to_string(),
            PartStyles::parse(NOTES_MASTER, &notes_master).unwrap(),
        )],
        [(
            NOTES_SLIDE.to_string(),
            vec![NOTES_SLIDE.to_string(), NOTES_MASTER.to_string()],
        )],
    );
    let units = scan_xml_with(NOTES_SLIDE, &notes_slide, Some(&styles)).unwrap();

    assert_eq!(
        units[0].props.alignment.origin().map(ToString::to_string),
        Some(format!("{NOTES_MASTER} notesStyle/lvl1pPr@algn"))
    );
    assert_eq!(rules(&units), ["alignment-unset"]);
}

#[test]
fn a_placeholder_the_layout_does_not_answer_falls_through_to_the_master() {
    // A layout that carries a different placeholder must not lend its list
    // style to this one.
    let units = deck(
        &slide(
            r#"<p:ph type="body" idx="1"/>"#,
            &format!("<a:p><a:r><a:rPr lang=\"ar-SA\"/><a:t>{ARABIC}</a:t></a:r></a:p>"),
        ),
        &layout(r#"<p:ph type="body" idx="2"/>"#, r#"<a:lvl1pPr rtl="0"/>"#),
        &master(ARABIC_MASTER),
    );

    assert_eq!(
        units[0].props.direction.origin().map(ToString::to_string),
        Some(format!("{MASTER} bodyStyle/lvl1pPr@rtl"))
    );
    assert!(rules(&units).is_empty(), "{:#?}", rules(&units));
}

#[test]
fn an_empty_list_style_supplies_nothing() {
    // `<a:lstStyle/>` is what PowerPoint writes on most layout placeholders.
    // Treating it as a source would stop the walk one hop short of the master.
    let styles = PartStyles::parse(LAYOUT, &layout(r#"<p:ph type="title"/>"#, "")).unwrap();
    let mut props = mirsam_core::Properties::default();
    StyleIndex::from_parts(
        [(LAYOUT.to_string(), styles)],
        [(
            SLIDE.to_string(),
            vec![SLIDE.to_string(), LAYOUT.to_string()],
        )],
    )
    .resolve(SLIDE, None, 0, &mut props);
    assert!(props.direction.is_unset());
    assert!(props.alignment.is_unset());
}

#[test]
fn a_part_with_no_chain_resolves_nothing() {
    // A chart part has no layout and no master. Its paragraphs stay `Unset`,
    // which is the honest answer rather than a guess.
    let units = scan_xml_with(
        "ppt/charts/chart1.xml",
        &slide("", &format!("<a:p><a:r><a:t>{ARABIC}</a:t></a:r></a:p>")),
        Some(&StyleIndex::default()),
    )
    .unwrap();
    assert!(units[0].props.direction.is_unset());
    assert!(units[0].props.alignment.is_unset());
}

// ------------------------------------------------------- 2.3: nine list levels

/// A master whose second list level contradicts its first. Nothing real writes
/// this, and that is the point: a resolver reading the wrong level cannot hide
/// behind two levels that happen to agree, which is what every stock template
/// ships and why the corpus alone cannot settle this.
const SPLIT_LEVEL_MASTER: &str =
    r#"<p:bodyStyle><a:lvl1pPr rtl="1" algn="r"/><a:lvl2pPr rtl="0" algn="l"/></p:bodyStyle>"#;

/// A body paragraph at the given `a:pPr/@lvl`. Zero-based: `lvl="1"` is the
/// second level and reads `a:lvl2pPr`.
fn at_level(lvl: Option<u32>) -> String {
    let pr = match lvl {
        Some(lvl) => format!(r#"<a:pPr lvl="{lvl}"/>"#),
        None => "<a:pPr/>".to_string(),
    };
    format!(r#"<a:p>{pr}<a:r><a:rPr lang="ar-SA"/><a:t>{ARABIC}</a:t></a:r></a:p>"#)
}

#[test]
fn a_paragraph_at_the_second_level_reads_the_sources_second_level() {
    // PLAN §2.3's first half. Both paragraphs are the same Arabic under the
    // same master; only `@lvl` differs, and it has to be what decides.
    let first = deck(
        &slide(r#"<p:ph type="body" idx="1"/>"#, &at_level(None)),
        &layout(r#"<p:ph type="body" idx="1"/>"#, ""),
        &master(SPLIT_LEVEL_MASTER),
    );
    // `lvl1pPr` says right-to-left and right-aligned, which agrees with the
    // letters, so ADR 0007 silences both findings.
    assert_eq!(rules(&first), Vec::<&str>::new());
    assert_eq!(
        first[0].props.direction.origin().unwrap().property,
        "bodyStyle/lvl1pPr@rtl"
    );

    let second = deck(
        &slide(r#"<p:ph type="body" idx="1"/>"#, &at_level(Some(1))),
        &layout(r#"<p:ph type="body" idx="1"/>"#, ""),
        &master(SPLIT_LEVEL_MASTER),
    );
    // `lvl2pPr` contradicts the letters, so both findings stand — and name the
    // level that supplied the contradiction, not the one above it.
    assert_eq!(rules(&second), vec!["alignment-unset", "direction-unset"]);
    assert_eq!(
        second[0].props.direction.origin().unwrap().property,
        "bodyStyle/lvl2pPr@rtl"
    );
    assert_eq!(
        second[0].props.alignment.origin().unwrap().property,
        "bodyStyle/lvl2pPr@algn"
    );
}

#[test]
fn a_level_the_source_does_not_state_is_not_answered_by_its_first_level() {
    // PowerPoint's fallback for a level a master leaves out is its own
    // application default, not that master's `lvl1pPr`. Reaching for level one
    // would report a value no reader will ever see — and would report it as
    // inherited, which is a claim about the document rather than a guess.
    let units = deck(
        &slide(r#"<p:ph type="body" idx="1"/>"#, &at_level(Some(4))),
        &layout(r#"<p:ph type="body" idx="1"/>"#, ""),
        &master(ARABIC_MASTER),
    );
    assert!(units[0].props.direction.is_unset());
    assert!(units[0].props.alignment.is_unset());
    assert_eq!(rules(&units), vec!["alignment-unset", "direction-unset"]);
}

#[test]
fn a_layouts_own_level_is_still_nearer_than_the_masters() {
    // The level selection is applied at every hop, not only the last one.
    let units = deck(
        &slide(r#"<p:ph type="body" idx="1"/>"#, &at_level(Some(1))),
        &layout(
            r#"<p:ph type="body" idx="1"/>"#,
            r#"<a:lvl1pPr algn="l"/><a:lvl2pPr algn="r"/>"#,
        ),
        &master(SPLIT_LEVEL_MASTER),
    );
    let origin = units[0].props.alignment.origin().unwrap();
    assert_eq!(origin.part, LAYOUT);
    assert_eq!(origin.property, "ph[type=body,idx=1]/lstStyle/lvl2pPr@algn");
    assert_eq!(
        units[0].props.alignment.effective(),
        Some(&Alignment::Right)
    );
}

// ------------------------------------------------------ 2.3: theme font slots

/// Arabic in a shape that is not a placeholder, with a Latin font on the run.
/// That is `complex-font-missing`'s precondition: the finding stands unless
/// something supplies a complex-script font.
fn latin_paragraph() -> String {
    format!(
        r#"<a:p><a:r><a:rPr lang="ar-SA"><a:latin typeface="Calibri"/></a:rPr><a:t>{ARABIC}</a:t></a:r></a:p>"#
    )
}

/// A master naming the complex-script slot the way the argument says.
fn font_master(cs: &str) -> String {
    master(&format!(
        r#"<p:otherStyle><a:lvl1pPr rtl="1" algn="r"><a:defRPr lang="ar-SA"><a:cs typeface="{cs}"/></a:defRPr></a:lvl1pPr></p:otherStyle>"#
    ))
}

#[test]
fn without_a_complex_font_anywhere_the_finding_stands() {
    // Kept rather than described: the three tests below claim to silence this
    // finding, and none of them proves anything unless it is here to silence.
    let units = deck(
        &slide("", &latin_paragraph()),
        &layout("", ""),
        &font_master(""),
    );
    assert_eq!(rules(&units), vec!["complex-font-missing"]);
}

#[test]
fn a_master_naming_a_typeface_outright_supplies_the_complex_font() {
    let units = deck(
        &slide("", &latin_paragraph()),
        &layout("", ""),
        &font_master("Dubai"),
    );
    assert_eq!(rules(&units), Vec::<&str>::new());
    let origin = units[0].props.complex_font.origin().unwrap();
    assert_eq!(origin.part, MASTER);
    assert_eq!(origin.property, "otherStyle/lvl1pPr/defRPr/cs@typeface");
    assert_eq!(
        units[0].props.complex_font.effective().map(String::as_str),
        Some("Dubai")
    );
}

#[test]
fn a_master_referring_to_the_theme_resolves_through_the_font_scheme() {
    // PLAN §2.3's second half, and the reason the slot could not be resolved
    // in 2.2: `+mn-cs` is a pointer, and the typeface is in another part.
    let units = themed_deck(
        &slide("", &latin_paragraph()),
        &layout("", ""),
        &font_master("+mn-cs"),
        Some(&theme("Dubai Heading", "Dubai")),
    );
    assert_eq!(rules(&units), Vec::<&str>::new());
    assert_eq!(
        units[0].props.complex_font.effective().map(String::as_str),
        Some("Dubai")
    );
    // The theme is named, not the master: the theme is where a reviewer can
    // read the typeface the reader will see (invariant 6).
    let origin = units[0].props.complex_font.origin().unwrap();
    assert_eq!(origin.part, THEME);
    assert_eq!(origin.property, "fontScheme/minorFont/cs@typeface");
}

#[test]
fn a_theme_naming_no_complex_font_leaves_the_slot_unset() {
    // What the stock Office theme states, and what `quarterly-report.pptx`
    // sits on: `<a:cs typeface=""/>`. Resolving the reference to the empty
    // string would silence a real defect — the deck names no Arabic font at
    // all — so it resolves to nothing instead.
    let units = themed_deck(
        &slide("", &latin_paragraph()),
        &layout("", ""),
        &font_master("+mn-cs"),
        Some(&theme("", "")),
    );
    assert!(units[0].props.complex_font.is_unset());
    assert_eq!(rules(&units), vec!["complex-font-missing"]);
}

#[test]
fn a_theme_reference_on_the_run_itself_is_never_reported_as_a_typeface() {
    // `+mn-cs` written on the paragraph's own run. Before 2.3 this was
    // recorded as the typeface, so a report claimed a font named "+mn-cs".
    let paragraph = format!(
        r#"<a:p><a:r><a:rPr lang="ar-SA"><a:latin typeface="+mn-lt"/><a:cs typeface="+mn-cs"/></a:rPr><a:t>{ARABIC}</a:t></a:r></a:p>"#
    );
    let units = themed_deck(
        &slide("", &paragraph),
        &layout("", ""),
        &font_master(""),
        Some(&theme("Dubai Heading", "Dubai")),
    );
    assert_eq!(
        units[0].props.complex_font.effective().map(String::as_str),
        Some("Dubai")
    );
    assert_eq!(
        units[0].props.latin_font.effective().map(String::as_str),
        Some("Calibri")
    );

    // With no theme to read, the pointer resolves to nothing rather than to
    // itself: a font named "+mn-cs" is a font nobody has.
    let unthemed = deck(&slide("", &paragraph), &layout("", ""), &font_master(""));
    assert!(unthemed[0].props.complex_font.is_unset());
    assert!(unthemed[0].props.latin_font.is_unset());
}

#[test]
fn the_latin_slot_is_not_inherited_from_the_chain() {
    // `complex-font-missing` fires only where a Latin font is *chosen*, and a
    // template's `+mn-lt` is not a choice anyone made about this paragraph.
    // Inheriting it would manufacture the rule's precondition on every Arabic
    // paragraph in every deck sitting on a stock theme.
    let master_xml = master(
        r#"<p:otherStyle><a:lvl1pPr rtl="1" algn="r"><a:defRPr lang="ar-SA"><a:latin typeface="Calibri"/></a:defRPr></a:lvl1pPr></p:otherStyle>"#,
    );
    let units = themed_deck(
        &slide(
            "",
            &format!(r#"<a:p><a:r><a:rPr lang="ar-SA"/><a:t>{ARABIC}</a:t></a:r></a:p>"#),
        ),
        &layout("", ""),
        &master_xml,
        Some(&theme("Dubai Heading", "Dubai")),
    );
    assert!(units[0].props.latin_font.is_unset());
    assert_eq!(rules(&units), Vec::<&str>::new());
}

#[test]
fn a_font_scheme_is_read_from_the_scheme_and_not_from_the_rest_of_the_theme() {
    // `a:latin` and `a:cs` occur in a theme's `a:objectDefaults` too, where
    // they are shape defaults rather than the slots a reference points at.
    let xml = format!(
        r#"<a:theme {NS}><a:themeElements><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:cs typeface="Dubai"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:cs typeface=""/></a:minorFont></a:fontScheme></a:themeElements><a:objectDefaults><a:spDef><a:lstStyle><a:lvl1pPr><a:defRPr><a:cs typeface="Impostor"/></a:defRPr></a:lvl1pPr></a:lstStyle></a:spDef></a:objectDefaults></a:theme>"#
    );
    let scheme = FontScheme::parse(THEME, &xml).unwrap();
    assert_eq!(
        scheme.typeface(ThemeFont::Major, ThemeScript::Complex),
        Some("Dubai")
    );
    // Empty is "names none", not "names the empty string".
    assert_eq!(
        scheme.typeface(ThemeFont::Minor, ThemeScript::Complex),
        None
    );
    assert_eq!(
        scheme.typeface(ThemeFont::Minor, ThemeScript::Latin),
        Some("Calibri")
    );
}

// ------------------------------------------------------------- the real decks

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn decks() -> Vec<PathBuf> {
    let mut decks: Vec<PathBuf> = std::fs::read_dir(fixtures())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "pptx"))
        .filter(|path| !name_of(path).contains(".out."))
        .collect();
    decks.sort();
    assert!(!decks.is_empty(), "no corpus decks under {:?}", fixtures());
    decks
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Every finding on one corpus deck, as the binary would produce it.
fn audit(deck: &Path) -> Vec<(String, String, Option<String>)> {
    use mirsam_core::DocumentReader;
    let mut document = PptxDocument::open(deck).unwrap();
    let units = document.scan().unwrap();
    Engine::with_default_rules()
        .audit(&units)
        .diagnostics
        .into_iter()
        .map(|d| (d.rule.0.to_string(), d.unit.0, d.evidence.inherited_from))
        .collect()
}

#[test]
fn the_rtl_mastered_decks_lose_their_paragraph_level_unset_findings() {
    // ADR 0007's consequences, stated as a prediction before the code existed:
    // the three hand-built decks sit on `rtl="1" algn="r"` masters, so every
    // paragraph in a slide that states nothing of its own is now correct by
    // inheritance rather than merely correct by luck.
    //
    // Asserted against the same decks read *without* the chain as well, so
    // this cannot pass by the decks having had nothing to report in the first
    // place: the findings are there when the chain is not read, and gone when
    // it is. That is the "starts red" of PLAN §2.2, kept rather than described.
    let unset = |findings: Vec<(String, String, Option<String>)>| -> Vec<_> {
        findings
            .into_iter()
            .filter(|(rule, unit, _)| {
                matches!(rule.as_str(), "direction-unset" | "alignment-unset")
                    && unit.starts_with("ppt/slides/")
            })
            .collect::<Vec<_>>()
    };

    for name in ["broken-arabic.pptx", "clean.pptx", "torture.pptx"] {
        let deck = fixtures().join(name);
        assert!(
            unset(audit(&deck)).is_empty(),
            "{name}: {:#?}",
            unset(audit(&deck))
        );
    }

    // Only two of the three had any to lose; `clean.pptx` states everything on
    // its paragraphs and was silent before M2 as well.
    for name in ["broken-arabic.pptx", "torture.pptx"] {
        let deck = fixtures().join(name);
        assert!(
            !unset(audit_without_the_chain(&deck)).is_empty(),
            "{name}: nothing to lose, so the test proves nothing"
        );
    }
}

/// The same audit with the layout/master chain deliberately not read: what
/// mirsam reported on these decks before 2.2.
fn audit_without_the_chain(deck: &Path) -> Vec<(String, String, Option<String>)> {
    use mirsam_ooxml::pptx::scan_xml;
    let package = Package::open(deck).unwrap();
    let mut units = Vec::new();
    for part in package
        .parts_where(|n| n.starts_with("ppt/") && n.ends_with(".xml"))
        .unwrap()
    {
        units.extend(scan_xml(&part, &package.read_text(&part).unwrap()).unwrap());
    }
    Engine::with_default_rules()
        .audit(&units)
        .diagnostics
        .into_iter()
        .map(|d| (d.rule.0.to_string(), d.unit.0, d.evidence.inherited_from))
        .collect()
}

#[test]
fn an_english_mastered_deck_keeps_its_direction_findings_and_names_the_master() {
    // The other side of the same prediction: `quarterly-report.pptx` sits on
    // an `rtl="0"` master, and M2 must change the *reason* for its findings
    // without removing them. Every one now names the part that supplied the
    // contradicting value, which is what makes it checkable.
    let findings: Vec<_> = audit(&fixtures().join("quarterly-report.pptx"))
        .into_iter()
        .filter(|(rule, _, _)| rule == "direction-unset")
        .collect();
    assert_eq!(findings.len(), 7, "{findings:#?}");
    for (rule, unit, from) in &findings {
        let from = from.as_deref().unwrap_or_default();
        assert!(
            from.starts_with("ppt/slideMasters/") && from.ends_with("pPr@rtl"),
            "{rule} on {unit} does not name where the direction came from: {from:?}"
        );
    }

    // 2.3 on a deck PowerPoint wrote. One of the seven is the sub-bullet on
    // slide 2, the corpus's only `a:pPr/@lvl`, and it must cite the level it
    // actually reads. Every stock master states all nine levels alike, so the
    // cited level is the only thing that can tell a resolver reading `@lvl`
    // from one that always reads the first.
    let levels: Vec<&str> = findings
        .iter()
        .filter_map(|(_, _, from)| from.as_deref())
        .filter_map(|from| from.rsplit_once('/').map(|(_, tail)| tail))
        .collect();
    assert_eq!(levels.iter().filter(|l| **l == "lvl2pPr@rtl").count(), 1);
    assert_eq!(levels.iter().filter(|l| **l == "lvl1pPr@rtl").count(), 6);
}

#[test]
fn the_torture_deck_resolves_its_complex_font_through_the_theme() {
    // PLAN §2.3's second half against a deck an application opens, not only
    // hand-built XML. Its master writes the complex-script slot both ways a
    // real one does: `titleStyle` and `bodyStyle` point into the theme with
    // `+mj-cs` / `+mn-cs`, `otherStyle` names Dubai outright. Both must arrive
    // at Dubai, and each must name the part a reviewer can read it in.
    //
    // `torture.pptx` alone, because it is the only hand-built deck whose
    // paragraphs leave the slot to the chain: `clean.pptx` and
    // `broken-arabic.pptx` write `<a:cs typeface="Dubai"/>` on every run, and
    // a paragraph that states its own font inherits none.
    use mirsam_core::DocumentReader;

    let mut document = PptxDocument::open(fixtures().join("torture.pptx")).unwrap();
    let units = document.scan().unwrap();
    let resolved: Vec<(String, String)> = units
        .iter()
        .filter(|u| u.id.0.starts_with("ppt/slides/"))
        .filter_map(|u| {
            let origin = u.props.complex_font.origin()?;
            Some((
                u.props.complex_font.effective()?.clone(),
                origin.part.clone(),
            ))
        })
        .collect();

    assert!(
        !resolved.is_empty(),
        "no slide paragraph inherited a complex-script font"
    );
    for (typeface, part) in &resolved {
        assert_eq!(typeface, "Dubai", "resolved {typeface} from {part}");
    }
    // The reference path specifically. Without this the test passes on a
    // resolver that reads only the typeface `otherStyle` names outright and
    // never opens the theme at all.
    assert!(
        resolved
            .iter()
            .any(|(_, part)| part.starts_with("ppt/theme/")),
        "nothing resolved through the theme, so the reference path is unproven: {resolved:#?}"
    );
}

#[test]
fn a_stock_office_theme_names_no_complex_font_and_the_finding_stands() {
    // The other outcome, on the deck that has it. `quarterly-report.pptx`'s
    // master says `+mn-cs` in all three styles and its theme answers with
    // `<a:cs typeface=""/>`, so the reference resolves to nothing — which is
    // why its four `complex-font-missing` warnings survive 2.3 rather than
    // being silenced by a resolver that took the empty string for a font.
    let deck = fixtures().join("quarterly-report.pptx");
    let package = Package::open(&deck).unwrap();
    let scheme = FontScheme::parse(
        "ppt/theme/theme1.xml",
        &package.read_text("ppt/theme/theme1.xml").unwrap(),
    )
    .unwrap();
    assert_eq!(
        scheme.typeface(ThemeFont::Minor, ThemeScript::Complex),
        None
    );
    assert_eq!(
        scheme.typeface(ThemeFont::Minor, ThemeScript::Latin),
        Some("Calibri")
    );

    let findings: Vec<_> = audit(&deck)
        .into_iter()
        .filter(|(rule, _, _)| rule == "complex-font-missing")
        .collect();
    assert_eq!(findings.len(), 2, "{findings:#?}");
}

#[test]
fn the_correct_deck_stays_silent() {
    // The deck the tool must leave completely alone states everything on its
    // paragraphs, so resolving the chain must change nothing about it.
    let findings = audit(&fixtures().join("quarterly-report-correct.pptx"));
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn every_slide_in_the_corpus_resolves_against_a_part_the_package_holds() {
    // The chain the index built is the chain the graph found, and every part
    // it names is readable. A silent mis-resolution — a layout matched to the
    // wrong master, a name that is not an item — would show up here rather
    // than as a quietly missing finding.
    for deck in decks() {
        let name = name_of(&deck);
        let package = Package::open(&deck).unwrap();
        let document = PptxDocument::open(&deck).unwrap();
        let styles = document.styles().unwrap();
        let graph = document.relationships().unwrap();

        let slides = graph.parts_with_role(mirsam_ooxml::rels::Role::Slide);
        assert!(!slides.is_empty(), "{name}: no slides");
        for slide in slides {
            let mut props = mirsam_core::Properties::default();
            styles.resolve(slide, None, 0, &mut props);
            // Every corpus master states a direction in `otherStyle`, so a
            // shape that is not a placeholder resolves one on every deck.
            let origin = props
                .direction
                .origin()
                .unwrap_or_else(|| panic!("{name}: {slide} resolved no direction"));
            package.read_text(&origin.part).unwrap_or_else(|e| {
                panic!("{name}: {slide} resolved against {}: {e}", origin.part)
            });
        }
    }
}
