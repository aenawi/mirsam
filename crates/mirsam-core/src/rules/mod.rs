//! The rule set and the engine that runs it.
//!
//! Adding a check means adding a [`Rule`] implementation and registering it.
//! Neither the engine nor any adapter changes — the Open/Closed seam of this
//! design, and the reason format support and rule coverage can grow
//! independently of one another.

mod direction;
mod typography;
mod unicode;

use crate::diagnostic::{Diagnostic, Report, RuleId};
use crate::fix::{Fix, Repair};
use crate::script;
use crate::text::TextUnit;

/// One correctness check over a single text unit.
///
/// Rules are pure: same unit in, same diagnostics out. That is what makes the
/// engine trivially parallelisable later, and testable without any document.
pub trait Rule: Send + Sync {
    fn id(&self) -> RuleId;

    /// One line, present tense, describing what the rule enforces.
    fn description(&self) -> &'static str;

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic>;

    /// The mechanical repair for this rule on this unit, when one exists.
    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        None
    }
}

/// Runs a set of rules over a set of units.
pub struct Engine {
    rules: Vec<Box<dyn Rule>>,
}

impl Engine {
    /// The rules enabled by default.
    pub fn with_default_rules() -> Self {
        Self {
            rules: vec![
                Box::new(unicode::BidiControls),
                Box::new(unicode::PresentationForms),
                Box::new(direction::DirectionMismatch),
                Box::new(direction::DirectionUnset),
                Box::new(direction::AlignmentIncoherent),
                Box::new(typography::LanguageMissing),
                Box::new(typography::ComplexFontMissing),
                Box::new(typography::LiteralBullet),
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
                report.diagnostics.extend(rule.check(unit));
            }
        }
        report.sorted()
    }

    /// Repairs for every unit that a rule can mechanically fix.
    pub fn plan(&self, units: &[TextUnit]) -> Vec<Repair> {
        let mut repairs = Vec::new();
        for unit in units {
            for rule in &self.rules {
                if rule.check(unit).is_empty() {
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
