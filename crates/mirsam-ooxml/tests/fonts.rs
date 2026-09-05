//! PLAN §4.3's acceptance, over a real deck.
//!
//! `mirsam-core`'s own suite proves what the two font rules conclude from a
//! `TextUnit` a test wrote by hand. This proves the other half — that a
//! typeface a *deck* names, resolved through the adapter's inheritance chain,
//! reaches those rules and comes back as a finding naming the exact characters
//! that will not render.
//!
//! The machine is a stub. `broken-arabic.pptx` names `Calibri` and `Dubai`,
//! and whether either of them draws Arabic depends entirely on who is running
//! the test — a suite that asked the developer's font directories would assert
//! whatever happened to be installed. So the source below answers every family
//! with one committed fixture font, which is the same statement as "on this
//! reader's machine, the deck's font has no Arabic".

use std::sync::Arc;

use mirsam_core::diagnostic::Severity;
use mirsam_core::ports::{FontFile, FontSource};
use mirsam_core::{Diagnostic, DocumentReader, Engine, RepairOptions, Result};
use mirsam_ooxml::PptxDocument;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .expect("tests/fixtures exists")
}

/// The shaping fixtures, which live with the crate that shapes.
fn font(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../mirsam-core/tests/fonts")
        .join(name);
    std::fs::read(&path).expect("the fixture fonts are committed")
}

/// A machine on which every typeface a document could name is `file`.
struct Everything(&'static str);

impl FontSource for Everything {
    fn load(&self, family: &str) -> Result<Option<FontFile>> {
        Ok(Some(FontFile {
            data: font(self.0),
            index: 0,
            path: format!("<fixture>/{}", self.0),
            family: family.to_string(),
        }))
    }
}

fn audit(deck: &str, machine: Everything) -> Vec<Diagnostic> {
    let mut document = PptxDocument::open(fixtures().join(deck)).expect("a corpus deck");
    let units = document.scan().expect("the deck lowers");
    Engine::with_fonts(&RepairOptions::default(), Arc::new(machine))
        .audit(&units)
        .diagnostics
}

fn of(diagnostics: &[Diagnostic], rule: &str) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .filter(|d| d.rule.0 == rule)
        .cloned()
        .collect()
}

#[test]
fn a_deck_whose_font_has_no_arabic_is_reported_with_the_exact_characters() {
    // The acceptance, in one sentence: this deck's complex-script slot
    // resolves to a font that maps printable ASCII and nothing else, and the
    // report says which letters go blank rather than that "the font is wrong".
    let findings = audit("broken-arabic.pptx", Everything("latin.ttf"));
    let coverage = of(&findings, "font-coverage");
    assert!(
        !coverage.is_empty(),
        "every Arabic paragraph in this deck should be reported: {findings:#?}"
    );

    for finding in &coverage {
        assert_eq!(finding.severity, Severity::Error, "{finding:#?}");
        assert!(
            !finding.evidence.offenders.is_empty(),
            "a coverage finding with no characters names nothing a reviewer can check"
        );
        for offender in &finding.evidence.offenders {
            assert!(
                offender.starts_with("U+") && offender.contains("ARABIC"),
                "{offender:?} is not a named Arabic character"
            );
        }
        // The unit the finding is on is a real place in the package, so a
        // reviewer can open the part and find the paragraph.
        assert!(finding.location.part.starts_with("ppt/"), "{finding:#?}");
    }

    // A font with no Arabic has no shaping to be broken. One paragraph, one
    // finding, one repair.
    assert!(of(&findings, "shaping-broken").is_empty(), "{findings:#?}");
}

#[test]
fn a_deck_whose_font_cannot_join_is_reported_as_a_shaping_defect() {
    // The same deck, the same correct Unicode, a font that has every letter —
    // and no `GSUB`. Nothing in the XML changed and nothing in the XML could
    // have said so.
    let findings = audit("broken-arabic.pptx", Everything("nonjoining.ttf"));
    let shaping = of(&findings, "shaping-broken");
    assert!(!shaping.is_empty(), "{findings:#?}");

    for finding in &shaping {
        assert_eq!(finding.severity, Severity::Error, "{finding:#?}");
        assert!(!finding.evidence.offenders.is_empty(), "{finding:#?}");
    }
    assert!(of(&findings, "font-coverage").is_empty(), "{findings:#?}");
}

#[test]
fn a_deck_whose_font_does_its_job_is_reported_for_neither() {
    // `joining.ttf` covers U+0621..U+064A and shapes all of it. Anything this
    // deck's Arabic still trips is a defect in the deck, not in the font — and
    // a rule that fired here would be a false positive on every correct
    // pairing there is.
    for deck in ["broken-arabic.pptx", "clean.pptx"] {
        let findings = audit(deck, Everything("joining.ttf"));
        assert!(
            of(&findings, "shaping-broken").is_empty(),
            "{deck}: {findings:#?}"
        );
    }
}
