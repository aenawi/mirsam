//! The two font rules (PLAN §4.3), through the engine that arms them.
//!
//! These run against the hand-built fonts in `tests/fonts/`, which differ in
//! one thing each, so a test that separates them is proving the *rule* and not
//! the shaper. The stub source below is the whole of the machine: a family
//! name in, the bytes of a committed fixture out, and no directory on the
//! developer's computer consulted for anything.
//!
//! What each font is here to prove:
//!
//! | font | coverage | shaping | what it stands for |
//! |---|---|---|---|
//! | `latin.ttf` | none | — | Helvetica under Arabic: empty boxes |
//! | `nonjoining.ttf` | complete | none | a font with no `GSUB`: disconnected letters |
//! | `partial.ttf` | complete | most | macOS's Arial: correct, and must stay silent |
//! | `joining.ttf` | complete | all | a font doing its job |

use std::sync::Arc;

use mirsam_core::diagnostic::Severity;
use mirsam_core::ports::{FontFile, FontSource};
use mirsam_core::{Diagnostic, Engine, Origin, Properties, RepairOptions, Resolved, TextUnit};

/// A machine with exactly the four fixture fonts installed, and nothing else.
///
/// `mirsam-fonts` is the real adapter and has its own suite; what is under
/// test here is what the domain concludes once bytes are in hand, so the
/// source is as small as the port allows.
struct Fixtures;

impl Fixtures {
    fn file(family: &str) -> Option<&'static str> {
        Some(match family {
            "Mirsam Joining" => "joining.ttf",
            "Mirsam Nonjoining" => "nonjoining.ttf",
            "Mirsam Partial" => "partial.ttf",
            "Mirsam Latin" => "latin.ttf",
            _ => return None,
        })
    }
}

impl FontSource for Fixtures {
    fn load(&self, family: &str) -> mirsam_core::Result<Option<FontFile>> {
        let Some(name) = Self::file(family) else {
            return Ok(None);
        };
        let path = format!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/{}"), name);
        Ok(Some(FontFile {
            data: std::fs::read(&path).expect("the fixture fonts are committed"),
            index: 0,
            path,
            family: family.to_string(),
        }))
    }
}

/// One paragraph of Arabic whose complex-script slot names `font`.
fn paragraph(text: &str, font: &str) -> TextUnit {
    TextUnit::new("slide1.xml#p1", text).with_props(Properties {
        complex_font: Resolved::Explicit(font.to_string()),
        ..Default::default()
    })
}

/// What the engine reports for one unit, with the font checks armed.
fn audit(unit: TextUnit) -> Vec<Diagnostic> {
    Engine::with_fonts(&RepairOptions::default(), Arc::new(Fixtures))
        .audit(&[unit])
        .diagnostics
}

/// Only the findings of one rule; the unit also trips `language-missing` and
/// the direction rules, which are not what any of this is about.
fn of(diagnostics: &[Diagnostic], rule: &str) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.rule.0 == rule)
        .cloned()
        .collect()
}

// ------------------------------------------------------------- font-coverage

#[test]
fn a_latin_font_under_arabic_is_reported_with_the_exact_characters() {
    // PLAN §4.3's acceptance. `latin.ttf` maps printable ASCII and no Arabic
    // whatsoever, which is what Helvetica is when a deck points the
    // complex-script slot at it: every letter renders as an empty box.
    let findings = audit(paragraph("مرحبا", "Mirsam Latin"));
    let coverage = of(&findings, "font-coverage");
    assert_eq!(coverage.len(), 1, "{findings:#?}");

    let finding = &coverage[0];
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(
        finding.evidence.offenders,
        [
            "U+0645 ARABIC LETTER MEEM",
            "U+0631 ARABIC LETTER REH",
            "U+062D ARABIC LETTER HAH",
            "U+0628 ARABIC LETTER BEH",
            "U+0627 ARABIC LETTER ALEF",
        ],
        "every character that will not render, named, in first-appearance order"
    );
    // The font that answered is named, and so is the file it was read from:
    // a reviewer can open it.
    assert!(
        finding.message.contains("Mirsam Latin") && finding.message.contains("latin.ttf"),
        "{}",
        finding.message
    );

    // And the *other* rule stays out of it. A font with no Arabic has no
    // shaping to be broken, and a second finding with the same repair would
    // be the tool arguing with itself about one paragraph.
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

#[test]
fn one_letter_a_font_is_missing_is_a_warning_not_an_error() {
    // The fixtures map U+0621..U+064A, so Persian peh is genuinely absent —
    // Mishafi's situation exactly. The pairing is not wrong; one character of
    // it will not render, and the severity says which claim is being made.
    let findings = audit(paragraph("مرحبا پ", "Mirsam Joining"));
    let coverage = of(&findings, "font-coverage");
    assert_eq!(coverage.len(), 1, "{findings:#?}");
    assert_eq!(coverage[0].severity, Severity::Warning);
    assert_eq!(coverage[0].evidence.offenders, ["U+067E ARABIC LETTER PEH"]);
}

#[test]
fn a_font_that_draws_the_text_is_not_reported() {
    for family in ["Mirsam Joining", "Mirsam Nonjoining", "Mirsam Partial"] {
        let findings = audit(paragraph("مرحبا", family));
        assert!(
            of(&findings, "font-coverage").is_empty(),
            "{family} covers this text: {findings:#?}"
        );
    }
}

// ------------------------------------------------------------ shaping-broken

#[test]
fn a_font_with_no_shaping_tables_is_reported() {
    // `nonjoining.ttf` has every letter and no `GSUB` at all: the text is
    // correct Unicode, correctly directed, every glyph present, and it
    // renders as a row of disconnected letters. No attribute in any document
    // format can express this, and no amount of reading XML would find it.
    let findings = audit(paragraph("مرحبا", "Mirsam Nonjoining"));
    let shaping = of(&findings, "shaping-broken");
    assert_eq!(shaping.len(), 1, "{findings:#?}");

    let finding = &shaping[0];
    assert_eq!(finding.severity, Severity::Error);
    assert_eq!(
        finding.evidence.offenders,
        [
            "U+0645 ARABIC LETTER MEEM",
            "U+0631 ARABIC LETTER REH",
            "U+062D ARABIC LETTER HAH",
            "U+0628 ARABIC LETTER BEH",
            "U+0627 ARABIC LETTER ALEF",
        ],
        "every letter the reader will see standing alone"
    );
    assert!(
        finding.message.contains("nonjoining.ttf"),
        "{}",
        finding.message
    );
    assert!(of(&findings, "font-coverage").is_empty(), "{findings:#?}");
}

#[test]
fn a_font_that_shapes_some_letters_and_not_others_is_never_reported() {
    // ADR 0008, and the reason `partial.ttf` is committed. It shapes
    // everything except the final forms of the right-joining letters, which
    // is macOS's Arial reduced to its principle: `مرحبا` comes back with the
    // reh standing alone and renders perfectly. A rule that regressed to a
    // per-letter verdict fires here rather than on a user's deck.
    for text in ["مرحبا", "مرحبا يا عالم", "بم", "الأداء بنسبة"] {
        let findings = audit(paragraph(text, "Mirsam Partial"));
        assert!(
            of(&findings, "shaping-broken").is_empty(),
            "{text:?} through partial.ttf: {findings:#?}"
        );
    }
}

#[test]
fn a_font_that_shapes_everything_is_never_reported() {
    let findings = audit(paragraph("مرحبا يا عالم", "Mirsam Joining"));
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

#[test]
fn a_scrap_of_text_is_passed_over_rather_than_reported_on() {
    // ADR 0008's stated cost: the aggregate needs enough text to mean
    // anything. `بم` requires two joins — one dual-joining letter — and a
    // design sharing a glyph between two forms is exactly what the ADR
    // forbids concluding a defect from. Through a font with no shaping
    // tables at all, and still silent.
    for text in ["بم", "سا"] {
        let findings = audit(paragraph(text, "Mirsam Nonjoining"));
        assert!(
            of(&findings, "shaping-broken").is_empty(),
            "{text:?} is not enough to conclude from: {findings:#?}"
        );
    }
    // Two such letters, independently silent, is. The same font, four
    // observable joins, and now it is reported.
    for text in ["بمبم", "بم مب"] {
        let findings = audit(paragraph(text, "Mirsam Nonjoining"));
        assert_eq!(
            of(&findings, "shaping-broken").len(),
            1,
            "{text:?}: {findings:#?}"
        );
    }
}

#[test]
fn text_with_no_join_to_make_proves_nothing_about_a_font() {
    // Every letter here is right-joining and followed by another, so no
    // letter is required to take a contextual form. The best font on the
    // machine and the worst would come back identical.
    let findings = audit(paragraph("ا د ر و", "Mirsam Nonjoining"));
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

// -------------------------------------------------------------- the boundary

#[test]
fn a_font_this_machine_does_not_have_is_silence_not_a_finding() {
    // The tool can no longer say what the reader will see, which is a fact
    // about this computer and not a defect in the deck. Reporting it would
    // fire on every runner with no fonts installed.
    let findings = audit(paragraph("مرحبا", "Traditional Arabic"));
    assert!(of(&findings, "font-coverage").is_empty(), "{findings:#?}");
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

#[test]
fn a_paragraph_naming_no_complex_font_is_left_to_the_rule_that_owns_it() {
    // `complex-font-missing` reports an empty slot. These two would be
    // guessing which font the application is going to substitute.
    let findings = audit(TextUnit::new("slide1.xml#p1", "مرحبا"));
    assert!(of(&findings, "font-coverage").is_empty(), "{findings:#?}");
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

#[test]
fn text_with_no_arabic_is_not_a_question_for_the_complex_script_slot() {
    let unit = TextUnit::new("slide1.xml#p1", "Revenue rose 25% in Q4").with_props(Properties {
        complex_font: Resolved::Explicit("Mirsam Latin".into()),
        ..Default::default()
    });
    let findings = audit(unit);
    assert!(of(&findings, "font-coverage").is_empty(), "{findings:#?}");
}

#[test]
fn without_a_font_source_the_checks_do_not_run() {
    // And a caller in this position has to say so: standing rule 4. Silence
    // from a check that never ran is not a pass, which is why `mirsam` reports
    // the font checks as unrun unless `--fonts` asked for them.
    let report = Engine::with_default_rules().audit(&[paragraph("مرحبا", "Mirsam Latin")]);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| matches!(d.rule.0, "font-coverage" | "shaping-broken")),
        "{:#?}",
        report.diagnostics
    );
    // Registered all the same, so `mirsam rules` describes the whole set.
    let ids: Vec<&str> = Engine::with_default_rules()
        .rules()
        .map(|r| r.0.0)
        .collect();
    assert!(ids.contains(&"font-coverage") && ids.contains(&"shaping-broken"));
}

#[test]
fn an_inherited_typeface_is_cited_where_it_was_stated() {
    // A master writes `+mn-cs` and the theme holds the typeface, so a
    // reviewer told only that the font has no Arabic has nowhere to go.
    // Invariant 6, and the reason `evidence.inherited_from` exists.
    let unit = TextUnit::new("slide1.xml#p1", "مرحبا").with_props(Properties {
        complex_font: Resolved::Inherited(
            "Mirsam Latin".into(),
            Origin::new("ppt/theme/theme1.xml", "fontScheme/minorFont/cs@typeface"),
        ),
        ..Default::default()
    });
    let findings = audit(unit);
    assert_eq!(
        of(&findings, "font-coverage")[0].evidence.inherited_from,
        Some("ppt/theme/theme1.xml fontScheme/minorFont/cs@typeface".to_string())
    );
}
