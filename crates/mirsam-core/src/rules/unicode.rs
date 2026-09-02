//! Rules about the codepoints themselves.

use super::Rule;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::fix::Fix;
use crate::text::TextUnit;
use crate::{controls, script};

/// Explicit bidi controls embedded in the text.
pub struct BidiControls;

impl Rule for BidiControls {
    fn id(&self) -> RuleId {
        RuleId("bidi-control")
    }

    fn description(&self) -> &'static str {
        "Text carries explicit Unicode bidi controls instead of container direction"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let hits = controls::scan(&unit.text);
        if hits.is_empty() {
            return Vec::new();
        }
        let mut names: Vec<String> = hits.iter().map(|h| h.name.to_string()).collect();
        names.sort();
        names.dedup();

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Error,
                &unit.id,
                &unit.location,
                format!(
                    "{} explicit bidi control(s) embedded in text; set container direction instead",
                    hits.len()
                ),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                offenders: names,
                ..Default::default()
            })
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        let offsets: Vec<usize> = controls::scan(&unit.text)
            .iter()
            .map(|h| h.offset)
            .collect();
        (!offsets.is_empty()).then_some(Fix::RemoveControls(offsets))
    }
}

/// Pre-shaped Arabic Presentation Forms in stored text.
pub struct PresentationForms;

impl Rule for PresentationForms {
    fn id(&self) -> RuleId {
        RuleId("presentation-forms")
    }

    fn description(&self) -> &'static str {
        "Text stores pre-shaped Arabic Presentation Forms instead of logical-order codepoints"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let count = unit
            .text
            .chars()
            .filter(|c| script::is_presentation_form(*c))
            .count();
        if count == 0 {
            return Vec::new();
        }
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Error,
                &unit.id,
                &unit.location,
                format!(
                    "{count} pre-shaped presentation form(s); text is not searchable or reflowable"
                ),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                ..Default::default()
            })
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        unit.text
            .chars()
            .any(script::is_presentation_form)
            .then_some(Fix::NormalizePresentationForms)
    }
}
