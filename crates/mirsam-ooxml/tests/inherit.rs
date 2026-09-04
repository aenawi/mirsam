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
use mirsam_ooxml::inherit::{PartStyles, StyleIndex};
use mirsam_ooxml::pptx::scan_xml_with;
use mirsam_ooxml::{Package, PptxDocument};
use std::path::{Path, PathBuf};

const NS: &str = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#;

const SLIDE: &str = "ppt/slides/slide1.xml";
const LAYOUT: &str = "ppt/slideLayouts/slideLayout1.xml";
const MASTER: &str = "ppt/slideMasters/slideMaster1.xml";

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

/// A slide over a layout over a master, resolved as the package would resolve
/// it: nearest source first, the master's named styles last.
fn deck(slide_xml: &str, layout_xml: &str, master_xml: &str) -> Vec<TextUnit> {
    let styles = StyleIndex::from_parts(
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
    .resolve(SLIDE, None, &mut props);
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
            from.starts_with("ppt/slideMasters/") && from.ends_with("/lvl1pPr@rtl"),
            "{rule} on {unit} does not name where the direction came from: {from:?}"
        );
    }
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
            styles.resolve(slide, None, &mut props);
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
