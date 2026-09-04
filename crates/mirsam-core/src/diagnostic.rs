//! What the engine reports.

use crate::text::{Location, UnitId};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum Severity {
    /// Worth knowing; never blocks delivery.
    Note,
    /// Probably wrong, or wrong in some renderers.
    Warning,
    /// Demonstrably renders incorrectly.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Note => "note",
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// Stable identifier for a rule. Used in reports, suppressions and docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RuleId(pub &'static str);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// Why the engine believes something is wrong.
///
/// Evidence is what makes a finding arguable rather than dogmatic: a reviewer
/// can check the claim without opening PowerPoint.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Evidence {
    /// The text as stored, in logical order.
    pub logical: Option<String>,
    /// How it resolves as declared.
    pub visual_declared: Option<String>,
    /// How it resolves once corrected.
    pub visual_expected: Option<String>,
    /// Offending characters, by name.
    pub offenders: Vec<String>,
    /// The part and property that supplied the value this finding is about,
    /// when the unit did not state it itself:
    /// `ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr@rtl`.
    ///
    /// A finding on an inherited value is unverifiable without it — "the
    /// master says left-to-right" is not checkable unless the tool names the
    /// master (ADR 0007 §5). Carried here rather than written into `message`,
    /// because consumers are told not to parse the human output.
    pub inherited_from: Option<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Diagnostic {
    pub rule: RuleId,
    pub severity: Severity,
    pub unit: UnitId,
    pub location: Location,
    pub message: String,
    pub evidence: Evidence,
    /// Whether a mechanical repair exists. The fix itself is attached
    /// separately so that `audit` need never construct one.
    pub fixable: bool,
}

impl Diagnostic {
    pub fn new(
        rule: RuleId,
        severity: Severity,
        unit: &UnitId,
        location: &Location,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule,
            severity,
            unit: unit.clone(),
            location: location.clone(),
            message: message.into(),
            evidence: Evidence::default(),
            fixable: false,
        }
    }

    pub fn with_evidence(mut self, evidence: Evidence) -> Self {
        self.evidence = evidence;
        self
    }

    pub fn fixable(mut self) -> Self {
        self.fixable = true;
        self
    }
}

/// The outcome of auditing one document.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Report {
    pub diagnostics: Vec<Diagnostic>,
    pub units_scanned: usize,
    pub arabic_units: usize,
    pub mixed_units: usize,
}

impl Report {
    pub fn count(&self, severity: Severity) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .count()
    }

    /// Does this report block delivery?
    ///
    /// Errors always block. Warnings block only under `strict`, which is the
    /// posture a release pipeline should take.
    pub fn is_blocking(&self, strict: bool) -> bool {
        self.count(Severity::Error) > 0 || (strict && self.count(Severity::Warning) > 0)
    }

    /// Highest severity first, then by rule, so output is stable across runs.
    pub fn sorted(mut self) -> Self {
        self.diagnostics.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then_with(|| a.rule.cmp(&b.rule))
        });
        self
    }
}
