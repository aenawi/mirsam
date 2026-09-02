//! Rules about base direction and alignment.

use super::Rule;
use crate::bidi;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::fix::Fix;
use crate::script;
use crate::text::{Alignment, Direction, Resolved, TextUnit};

/// What the renderer will actually use: the declared direction if any,
/// otherwise UAX#9's first-strong auto-detection.
fn effective_direction(unit: &TextUnit) -> Direction {
    unit.props
        .direction
        .effective()
        .copied()
        .unwrap_or_else(|| bidi::auto_direction(&unit.text))
}

/// The declared direction produces a demonstrably different rendering than the
/// semantically correct one.
///
/// This is the flagship rule: it fires on proven misrendering, not on a
/// missing attribute, so it has no false positives by construction.
pub struct DirectionMismatch;

impl Rule for DirectionMismatch {
    fn id(&self) -> RuleId {
        RuleId("direction-mismatch")
    }

    fn description(&self) -> &'static str {
        "Declared base direction resolves to a different visual order than the text's own direction"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        let expected = bidi::dominant_direction(&unit.text);
        let actual = effective_direction(unit);
        if !bidi::order_differs(&unit.text, actual, expected) {
            return Vec::new();
        }

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Error,
                &unit.id,
                &unit.location,
                format!(
                    "renders as {actual} but reads as {expected}; visual order differs from the logical text"
                ),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                visual_declared: Some(bidi::resolve(&unit.text, actual).visual),
                visual_expected: Some(bidi::resolve(&unit.text, expected).visual),
                offenders: Vec::new(),
            })
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        Some(Fix::SetDirection(bidi::dominant_direction(&unit.text)))
    }
}

/// Arabic text with no direction declared anywhere in the inheritance chain.
///
/// A warning, not an error: the text may well render correctly today via
/// auto-detection. It is fragile rather than broken, and the distinction is
/// worth preserving in the report.
pub struct DirectionUnset;

impl Rule for DirectionUnset {
    fn id(&self) -> RuleId {
        RuleId("direction-unset")
    }

    fn description(&self) -> &'static str {
        "Arabic text relies on renderer auto-detection because no base direction is declared"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) || !unit.props.direction.is_unset() {
            return Vec::new();
        }
        // Already reported, with evidence, by DirectionMismatch.
        let expected = bidi::dominant_direction(&unit.text);
        if bidi::order_differs(&unit.text, bidi::auto_direction(&unit.text), expected) {
            return Vec::new();
        }

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                "no base direction declared; correct today only by auto-detection",
            )
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        Some(Fix::SetDirection(bidi::dominant_direction(&unit.text)))
    }
}

/// A hard left alignment on right-to-left text.
///
/// Centre, justify and the direction-relative alignments are all legitimate and
/// are deliberately left alone; so is any alignment merely *inherited*, which
/// is the author's layout choice rather than a defect.
pub struct AlignmentIncoherent;

impl Rule for AlignmentIncoherent {
    fn id(&self) -> RuleId {
        RuleId("alignment-incoherent")
    }

    fn description(&self) -> &'static str {
        "Right-to-left text is explicitly aligned left"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        if bidi::dominant_direction(&unit.text) != Direction::Rtl {
            return Vec::new();
        }
        // Only an *explicit* left alignment is a finding. An inherited one
        // belongs to the layout and is none of this tool's business.
        let Resolved::Explicit(alignment) = unit.props.alignment else {
            return Vec::new();
        };
        if alignment.is_rtl_coherent() {
            return Vec::new();
        }

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                "right-to-left text is explicitly aligned left",
            )
            .fixable(),
        ]
    }

    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        // Direction-relative rather than a hard `Right`, so the paragraph stays
        // correct if it is ever re-used in a left-to-right context.
        //
        // `Start`, not `End`: the start edge is the side reading begins on —
        // the right in RTL, the left in LTR. `End` in an RTL paragraph is the
        // left edge, which is the defect this rule reports, so proposing it
        // would hand back the very alignment being flagged.
        Some(Fix::SetAlignment(Alignment::Start))
    }
}

#[cfg(test)]
mod alignment_fix_tests {
    use super::*;
    use crate::text::Properties;

    fn rtl_unit_aligned_left() -> TextUnit {
        TextUnit::new("u1", "ارتفع الأداء بنسبة 25%").with_props(Properties {
            alignment: Resolved::Explicit(Alignment::Left),
            ..Default::default()
        })
    }

    #[test]
    fn left_aligned_rtl_text_is_reported() {
        let found = AlignmentIncoherent.check(&rtl_unit_aligned_left());
        assert_eq!(found.len(), 1, "{found:#?}");
    }

    #[test]
    fn the_proposed_alignment_would_not_be_reported_again() {
        // The property that matters: applying the fix must clear the finding.
        // `End` satisfied `is_rtl_coherent` while still resolving to the left
        // edge in an RTL paragraph, so this asserts the reading direction, not
        // merely that some non-Left value was chosen.
        let Some(Fix::SetAlignment(proposed)) = AlignmentIncoherent.fix(&rtl_unit_aligned_left())
        else {
            panic!("expected an alignment fix");
        };
        assert_eq!(
            proposed,
            Alignment::Start,
            "the fix must align to the side RTL reading begins on — the right"
        );

        let repaired = TextUnit::new("u1", "ارتفع الأداء بنسبة 25%").with_props(Properties {
            alignment: Resolved::Explicit(proposed),
            ..Default::default()
        });
        assert!(
            AlignmentIncoherent.check(&repaired).is_empty(),
            "the repaired paragraph is still reported"
        );
    }
}
