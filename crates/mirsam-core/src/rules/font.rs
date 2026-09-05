//! The two rules that judge the font a paragraph's Arabic will be drawn with.
//!
//! [`crate::shape`] and [`crate::coverage`] report facts and refuse to draw a
//! conclusion from them; PLAN §4.3 is where the conclusions are drawn. There
//! are exactly two, and they are different defects with different advice:
//!
//! - **`font-coverage`** — the font has no glyph for some of the Arabic. It
//!   renders as empty boxes, and the only repair is a different font.
//! - **`shaping-broken`** — the font has every letter and no shaping tables.
//!   It renders as a row of disconnected letters, and the only repair is,
//!   again, a different font, but for a completely different reason.
//!
//! ## These are the only rules that ask about the machine
//!
//! Every other rule in this crate reads a document and reasons about it.
//! These two need a font file, so they need a [`FontSource`] — and a source
//! is a fact about the computer the tool is running on, not about the
//! document. Three consequences follow, and all three are deliberate.
//!
//! *No source, no findings.* A rule built without one checks nothing and says
//! nothing. That is not a silent failure as long as the caller says so, which
//! is standing rule 4: a check that did not run is `NOT RUN`, never an
//! implied pass. `mirsam` reports the font checks as unrun unless `--fonts`
//! asked for them, and the reason they are opt-in at all is this one — an
//! audit whose result depended on which fonts happened to be installed on the
//! machine that ran it would be an audit nobody could reproduce.
//!
//! *A font this machine does not have is silence, not a finding.* The source
//! answering `None` means the tool can no longer say what the reader will
//! see, which is a fact about the machine and not a defect in the deck.
//! Reporting it would fire on every CI runner with no fonts installed.
//!
//! *The claim runs one way only.* A font that is *here* and cannot draw the
//! text will not draw it anywhere, because a `cmap` and a `GSUB` travel with
//! the font — that is why a finding is worth making at all. A font that is
//! here and draws the text perfectly proves nothing about the reader's
//! machine, and neither rule says otherwise: silence here is not a claim that
//! the document will render.
//!
//! ## Neither one is fixable
//!
//! The repair for both is "use a different typeface", and which typeface is
//! an authoring decision the text cannot supply — the same reason
//! [`super::typography::ComplexFontMissing`] proposes nothing until `--font`
//! chooses one. It is worse here: that rule fills an *empty* slot, while
//! these two would be overwriting a font the author put there. So they
//! report, and the author chooses.

use std::sync::Arc;

use super::Rule;
use crate::charname;
use crate::coverage::{Coverage, coverage};
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::ports::{FontFile, FontSource};
use crate::script;
use crate::shape::{Outcome, Shaping, shape};
use crate::text::TextUnit;

/// The font a unit's Arabic will actually be drawn with, on this machine.
///
/// `None` — and so silence from both rules — at every step that cannot be
/// taken: no source was supplied, the text has no Arabic in it, no
/// complex-script font is named, this machine has no such family, or the
/// file that answered is not one the shaper can read. Only the second is
/// about the document, and `complex-font-missing` already reports it.
fn resolve(fonts: Option<&Arc<dyn FontSource>>, unit: &TextUnit) -> Option<FontFile> {
    let fonts = fonts?;
    if !script::has_arabic(&unit.text) {
        return None;
    }
    let family = unit.props.complex_font.effective()?;
    // An error from the source is the machine failing to answer, not the
    // document failing a check. Nothing can be concluded either way.
    fonts.load(family).ok().flatten()
}

/// Where the typeface came from, when the unit did not name it itself.
///
/// A master writes `+mn-cs` and the theme holds the typeface, so a reviewer
/// told only that "Calibri has no Arabic" has nowhere to go. Invariant 6.
fn inherited_from(unit: &TextUnit) -> Option<String> {
    unit.props
        .complex_font
        .origin()
        .map(std::string::ToString::to_string)
}

/// How the finding names the font that answered: the family the *file* states
/// and the path it was read from.
///
/// Both, because neither alone is checkable. A machine with no `Calibri` may
/// answer with something else entirely, so the requested name would describe
/// a font nobody has; and the family alone leaves a reviewer hunting for
/// which of eleven files it was.
fn answered_by(file: &FontFile) -> String {
    format!("{} ({})", file.family, file.path)
}

/// The Arabic a font resolved for a paragraph has no glyph for.
///
/// The empty-boxes defect. A deck that points its complex-script slot at a
/// Latin font — Helvetica, Comic Sans, Calibri before its Arabic was added —
/// stores perfectly correct Unicode and renders nothing at all.
pub struct FontCoverage {
    /// Where a typeface name is resolved to bytes. `None` — the default —
    /// checks nothing; see the module documentation for why that is a
    /// position rather than an omission.
    pub fonts: Option<Arc<dyn FontSource>>,
}

impl Rule for FontCoverage {
    fn id(&self) -> RuleId {
        RuleId("font-coverage")
    }

    fn description(&self) -> &'static str {
        "The font resolved for a paragraph's Arabic has no glyph for some of it"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let Some(file) = resolve(self.fonts.as_ref(), unit) else {
            return Vec::new();
        };
        let Some(font) = file.font() else {
            return Vec::new();
        };
        let report = coverage(&font, &unit.text);
        if report.is_complete() {
            return Vec::new();
        }

        vec![
            Diagnostic::new(
                self.id(),
                severity(&report),
                &unit.id,
                &unit.location,
                message(&file, &report),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                // The exact characters that will not render, by name, which is
                // the whole of what this rule promises.
                offenders: report.missing.iter().map(ToString::to_string).collect(),
                inherited_from: inherited_from(unit),
                ..Default::default()
            }),
        ]
    }
}

/// How much of the text goes blank decides how loud the finding is.
///
/// A font that answers for *none* of the Arabic was not the font this text
/// needed, and that is an error: the paragraph renders as empty boxes and
/// there is nothing to argue about. A font missing some of it may be an
/// otherwise correct pairing meeting one unusual letter — Mishafi has no
/// Persian peh — which is still wrong for the characters it hits and is still
/// worth an author's attention, but is not the same claim.
fn severity(report: &Coverage) -> Severity {
    if report.covers_nothing() {
        Severity::Error
    } else {
        Severity::Warning
    }
}

fn message(file: &FontFile, report: &Coverage) -> String {
    let font = answered_by(file);
    if report.covers_nothing() {
        format!(
            "{font} has no glyph for any of the {} Arabic characters here; \
             the text will render as empty boxes",
            report.checked
        )
    } else {
        format!(
            "{font} has no glyph for {} of the {} Arabic characters here; \
             those will render as empty boxes",
            report.missing_occurrences(),
            report.checked
        )
    }
}

/// Arabic drawn by a font with no shaping tables.
///
/// The defect M4 exists for, and the first in this tool that no amount of
/// reading XML could find: the text is correct Unicode, correctly directed,
/// correctly aligned, every letter present in the font, and it renders as a
/// row of disconnected letters.
pub struct ShapingBroken {
    /// Where a typeface name is resolved to bytes. `None` checks nothing.
    pub fonts: Option<Arc<dyn FontSource>>,
}

/// How many letters must have been *required* to join, and been present in
/// the font, before "none of them did" means anything.
///
/// ADR 0008 leaves the number to this rule and states what it has to survive:
/// one letter drawn standalone is not a defect, because a design may share
/// one glyph between a letter's standalone and final forms and several do —
/// macOS's Arial among them. So the evidence is the aggregate, and the
/// threshold is what stops a scrap of text from being called one.
///
/// Four, and the arithmetic is why. A letter is required to take a final form
/// only because the letter before it joined forwards, so every final has a
/// companion initial or medial, and initials and medials are dual-joining
/// letters — the ones a font like Arial does shape. Two observable joins are
/// therefore *one* such letter, which is exactly the design choice the ADR
/// forbids concluding from. Four are two of them, independently, both silent.
/// No font that shapes Arabic at all looks like that.
///
/// A two-letter word offers two, and so is passed over rather than reported
/// on, which is the case ADR 0008 names.
const MIN_OBSERVABLE_JOINS: usize = 4;

/// Letters that were required to join *and* that the font actually has.
///
/// An unmapped letter is not evidence about shaping in either direction: a
/// font cannot join a glyph it does not have, and concluding "no shaping
/// tables" from a font that simply has no Arabic would put a second finding,
/// with the same repair and a different explanation, on text `font-coverage`
/// has already reported. Excluding them is what keeps the two rules from
/// arguing with each other about one paragraph.
fn observable_joins(shaping: &Shaping) -> usize {
    shaping
        .letters
        .iter()
        .filter(|l| l.required.is_joined() && l.outcome != Outcome::Unmapped)
        .count()
}

impl Rule for ShapingBroken {
    fn id(&self) -> RuleId {
        RuleId("shaping-broken")
    }

    fn description(&self) -> &'static str {
        "The font resolved for a paragraph's Arabic produces no joining forms"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let Some(file) = resolve(self.fonts.as_ref(), unit) else {
            return Vec::new();
        };
        let Some(font) = file.font() else {
            return Vec::new();
        };
        let shaping = shape(&font, &unit.text);

        let observable = observable_joins(&shaping);
        if observable < MIN_OBSERVABLE_JOINS || shaping.joins_produced() > 0 {
            return Vec::new();
        }

        // The letters the reader will see standing alone, named and
        // deduplicated in first-appearance order. Every one of them is in
        // the font, so this is a list of shapes that exist and were not used.
        let mut offenders: Vec<String> = Vec::new();
        for letter in shaping.drawn_standalone() {
            let Some(name) = charname::name(letter.ch) else {
                continue;
            };
            let entry = format!("U+{:04X} {name}", letter.ch as u32);
            if !offenders.contains(&entry) {
                offenders.push(entry);
            }
        }

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Error,
                &unit.id,
                &unit.location,
                format!(
                    "{} produced no joining form for any of the {observable} letters that \
                     require one; the Arabic will render as disconnected letters",
                    answered_by(&file)
                ),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                offenders,
                inherited_from: inherited_from(unit),
                ..Default::default()
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joining::JoiningForm;
    use crate::shape::ShapedLetter;

    /// A shaping result with `joined` letters required to join, `mapped` of
    /// them present in the font. The letter and its outcome do not matter
    /// here; the counting does.
    fn shaping(joined: usize, mapped: usize) -> Shaping {
        let letters = (0..joined)
            .map(|i| ShapedLetter {
                offset: i,
                ch: 'ب',
                required: JoiningForm::Medial,
                outcome: if i < mapped {
                    Outcome::Standalone
                } else {
                    Outcome::Unmapped
                },
                glyphs: Vec::new(),
            })
            .collect();
        Shaping { letters }
    }

    #[test]
    fn a_letter_the_font_has_no_glyph_for_is_not_shaping_evidence() {
        // Six letters required a join and the font has none of them: this is
        // `font-coverage`'s paragraph, and `shaping-broken` must not also
        // claim it.
        assert_eq!(observable_joins(&shaping(6, 0)), 0);
        assert_eq!(observable_joins(&shaping(6, 6)), 6);
        assert_eq!(observable_joins(&shaping(6, 2)), 2);
    }

    #[test]
    fn the_threshold_is_above_a_two_letter_word() {
        // ADR 0008: a container holding one two-letter word gives the
        // aggregate nothing to work with. Two joins is one dual-joining
        // letter, which is the design choice the ADR forbids concluding from.
        const { assert!(MIN_OBSERVABLE_JOINS > 2) };
    }

    #[test]
    fn a_coverage_severity_turns_on_how_much_of_the_text_goes_blank() {
        let nothing = Coverage {
            checked: 5,
            missing: vec![crate::coverage::MissingChar {
                ch: 'ب',
                codepoint: 0x0628,
                name: charname::name('ب').unwrap(),
                first_offset: 0,
                occurrences: 5,
            }],
        };
        assert_eq!(severity(&nothing), Severity::Error);

        let some = Coverage {
            checked: 5,
            missing: vec![crate::coverage::MissingChar {
                ch: 'ب',
                codepoint: 0x0628,
                name: charname::name('ب').unwrap(),
                first_offset: 0,
                occurrences: 1,
            }],
        };
        assert_eq!(severity(&some), Severity::Warning);
    }

    #[test]
    fn without_a_source_neither_rule_resolves_anything() {
        let unit = TextUnit::new("u1", "مرحبا").with_props(crate::text::Properties {
            complex_font: crate::text::Resolved::Explicit("Mirsam Joining".into()),
            ..Default::default()
        });
        assert!(resolve(None, &unit).is_none());
        assert!(FontCoverage { fonts: None }.check(&unit).is_empty());
        assert!(ShapingBroken { fonts: None }.check(&unit).is_empty());
    }
}
