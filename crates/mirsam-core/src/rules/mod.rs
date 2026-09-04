//! The rule set and the engine that runs it.
//!
//! Adding a check means adding a [`Rule`] implementation and registering it.
//! Neither the engine nor any adapter changes — the Open/Closed seam of this
//! design, and the reason format support and rule coverage can grow
//! independently of one another.

mod direction;
mod typography;
mod unicode;

pub use typography::{DEFAULT_LOCALE, is_arabic_tag};

use crate::diagnostic::{Diagnostic, Report, RuleId};
use crate::fix::{Fix, Repair};
use crate::script;
use crate::text::{TextUnit, UnitKind};

/// One correctness check over a single text unit.
///
/// Rules are pure: same unit in, same diagnostics out. That is what makes the
/// engine trivially parallelisable later, and testable without any document.
pub trait Rule: Send + Sync {
    fn id(&self) -> RuleId;

    /// One line, present tense, describing what the rule enforces.
    fn description(&self) -> &'static str;

    /// Which kind of unit this rule judges. Paragraphs unless a rule says
    /// otherwise; the engine never hands a rule a unit it does not apply to.
    fn applies_to(&self, kind: UnitKind) -> bool {
        kind == UnitKind::Paragraph
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic>;

    /// The mechanical repair for this rule on this unit, when one exists.
    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        None
    }
}

/// The authoring decisions a repair needs from the caller.
///
/// The engine can prove that a language tag is missing; it cannot know which
/// one the author meant. Each field here is a choice of that kind, with a
/// default only where a safe one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RepairOptions {
    /// BCP-47 tag written where Arabic text carries no Arabic tag.
    pub language: String,
    /// Complex-script typeface written where a Latin font is set and the
    /// Arabic slot is empty. `None` leaves those findings unrepaired.
    pub complex_font: Option<String>,
    /// Replace typed bullet glyphs with the format's native list. Off by
    /// default because it edits the text itself, not only its properties.
    pub convert_bullets: bool,
    /// Write an explicit start-edge alignment on right-to-left paragraphs
    /// that have none of their own. Off by default: the alignment such a
    /// paragraph inherits may be a layout's design — a centred title — and
    /// until M2 the tool cannot read the layout to tell.
    pub align: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            language: DEFAULT_LOCALE.to_string(),
            complex_font: None,
            convert_bullets: false,
            align: false,
        }
    }
}

/// Runs a set of rules over a set of units.
pub struct Engine {
    rules: Vec<Box<dyn Rule>>,
}

impl Engine {
    /// The rules enabled by default, proposing their default repairs.
    pub fn with_default_rules() -> Self {
        Self::with_options(&RepairOptions::default())
    }

    /// The rules enabled by default, with the repairs they propose shaped by
    /// `options`.
    ///
    /// What is *reported* does not depend on the options: the same defects
    /// come back whatever the caller intends to do about them. Only the
    /// proposed fixes vary, and with them `Diagnostic::fixable`.
    pub fn with_options(options: &RepairOptions) -> Self {
        Self {
            rules: vec![
                Box::new(unicode::BidiControls),
                Box::new(unicode::PresentationForms),
                Box::new(direction::DirectionMismatch),
                Box::new(direction::DirectionUnset),
                Box::new(direction::AlignmentIncoherent),
                Box::new(direction::AlignmentUnset {
                    align: options.align,
                }),
                Box::new(direction::ContainerDirection),
                Box::new(typography::LanguageMissing {
                    locale: options.language.clone(),
                }),
                Box::new(typography::ComplexFontMissing {
                    typeface: options.complex_font.clone(),
                }),
                Box::new(typography::LiteralBullet {
                    convert: options.convert_bullets,
                }),
            ],
        }
    }

    pub fn from_rules(rules: Vec<Box<dyn Rule>>) -> Self {
        Self { rules }
    }

    /// Every registered rule, for `mirsam rules`.
    pub fn rules(&self) -> impl Iterator<Item = (RuleId, &'static str)> + '_ {
        self.rules.iter().map(|r| (r.id(), r.description()))
    }

    pub fn audit(&self, units: &[TextUnit]) -> Report {
        let mut report = Report {
            units_scanned: units.len(),
            ..Default::default()
        };

        for unit in units {
            if script::has_arabic(&unit.text) {
                report.arabic_units += 1;
                if script::has_ltr_or_digits(&unit.text) {
                    report.mixed_units += 1;
                }
            }
            for rule in &self.rules {
                if rule.applies_to(unit.kind) {
                    report.diagnostics.extend(rule.check(unit));
                }
            }
        }
        report.sorted()
    }

    /// Repairs for every unit that a rule can mechanically fix.
    ///
    /// Document order, then rule order within a unit, so an adapter receives
    /// a unit's repairs together and in a stable sequence.
    pub fn plan(&self, units: &[TextUnit]) -> Vec<Repair> {
        let mut repairs = Vec::new();
        for unit in units {
            for rule in &self.rules {
                if !rule.applies_to(unit.kind) || rule.check(unit).is_empty() {
                    continue;
                }
                if let Some(fix) = rule.fix(unit) {
                    repairs.push(Repair::new(&unit.id, fix));
                }
            }
        }
        repairs
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::with_default_rules()
    }
}
