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

/// The declared direction disagrees with the text's own.
///
/// Two tiers. When the two directions resolve to different visual orders the
/// finding is an error, proven by the two renderings in its evidence; this
/// is the flagship rule, and it has no false positives by construction. When
/// the orders happen to coincide — pure Arabic reorders identically under
/// either base — an *explicit* contrary direction is still reported, as a
/// warning: the paragraph direction decides which edge is the start and
/// where edge punctuation lands, and no alignment repair can be lowered
/// correctly while it is wrong. That tier was added on visual evidence
/// (ADR 0006); the evidence it carries shows the two orders equal, which is
/// exactly the claim.
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
        let evidence = || Evidence {
            logical: Some(unit.text.clone()),
            visual_declared: Some(bidi::resolve(&unit.text, actual).visual),
            visual_expected: Some(bidi::resolve(&unit.text, expected).visual),
            offenders: Vec::new(),
        };

        if bidi::order_differs(&unit.text, actual, expected) {
            return vec![
                Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    &unit.id,
                    &unit.location,
                    format!(
                        "renders as {actual} but reads as {expected}; visual order differs from the logical text"
                    ),
                )
                .with_evidence(evidence())
                .fixable(),
            ];
        }

        // Same order either way. Only a direction the author *wrote* is
        // reported: an absent one is `direction-unset`'s business, and an
        // inherited one is the container's design.
        let Resolved::Explicit(declared) = unit.props.direction else {
            return Vec::new();
        };
        if declared == expected {
            return Vec::new();
        }
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                format!(
                    "declared {declared} but reads as {expected}; letter order is unaffected, but alignment and edge punctuation follow the paragraph direction"
                ),
            )
            .with_evidence(evidence())
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

/// Right-to-left text with no alignment of its own.
///
/// The paragraph takes its alignment from a layout the adapter cannot yet
/// read (M2). On a left-to-right template that is the left edge — the very
/// thing `alignment-incoherent` reports when it is written on the paragraph
/// — and on a centred title it is the design. The tool cannot tell which
/// from inside the paragraph, so this is a note: it never blocks, and it is
/// repaired only when the caller asks with `RepairOptions::align`. Judged
/// from the text alone, by decision (ADR 0006). It fires on `Unset`, never on
/// `Inherited`, so invariant 2 holds: once M2 resolves the chain, a layout
/// that centres or right-aligns the paragraph silences it.
pub struct AlignmentUnset {
    /// Whether the caller asked for the repair.
    pub align: bool,
}

impl Rule for AlignmentUnset {
    fn id(&self) -> RuleId {
        RuleId("alignment-unset")
    }

    fn description(&self) -> &'static str {
        "Right-to-left text has no alignment of its own and takes one from a layout the tool cannot yet read"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text)
            || bidi::dominant_direction(&unit.text) != Direction::Rtl
            || !unit.props.alignment.is_unset()
        {
            return Vec::new();
        }
        let diagnostic = Diagnostic::new(
            self.id(),
            Severity::Note,
            &unit.id,
            &unit.location,
            "no alignment declared; a left-to-right layout places this on the left edge",
        )
        .with_evidence(Evidence {
            logical: Some(unit.text.clone()),
            ..Default::default()
        });
        vec![if self.align {
            diagnostic.fixable()
        } else {
            diagnostic
        }]
    }

    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        // `Start`, for the same reason as `alignment-incoherent`: the right
        // edge in RTL, and still correct if the paragraph is ever re-used
        // left-to-right.
        self.align.then_some(Fix::SetAlignment(Alignment::Start))
    }
}

#[cfg(test)]
mod direction_tier_tests {
    use super::*;
    use crate::text::Properties;

    fn unit(text: &str, direction: Resolved<Direction>) -> TextUnit {
        TextUnit::new("u1", text).with_props(Properties {
            direction,
            ..Default::default()
        })
    }

    #[test]
    fn a_different_order_is_still_an_error() {
        let found = DirectionMismatch.check(&unit(
            "ارتفع الأداء بنسبة 25% في Q4 2026.",
            Resolved::Explicit(Direction::Ltr),
        ));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Error);
    }

    #[test]
    fn an_explicit_contrary_direction_with_the_same_order_is_a_warning() {
        // Pure Arabic: the letters come out the same either way, so nothing
        // is proven about order. The evidence says so — both renderings are
        // equal — and the direction is still wrong for alignment.
        let u = unit("التقرير الفصلي", Resolved::Explicit(Direction::Ltr));
        let found = DirectionMismatch.check(&u);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(found[0].fixable);
        assert_eq!(
            found[0].evidence.visual_declared,
            found[0].evidence.visual_expected
        );
        assert_eq!(
            DirectionMismatch.fix(&u),
            Some(Fix::SetDirection(Direction::Rtl))
        );
    }

    #[test]
    fn the_warning_tier_needs_a_direction_the_author_wrote() {
        // Absent is direction-unset's finding; inherited is the container's
        // design and never a finding of this rule.
        for direction in [Resolved::Unset, Resolved::Inherited(Direction::Ltr)] {
            assert!(
                DirectionMismatch
                    .check(&unit("التقرير الفصلي", direction))
                    .is_empty()
            );
        }
        assert!(
            DirectionMismatch
                .check(&unit("التقرير الفصلي", Resolved::Explicit(Direction::Rtl)))
                .is_empty()
        );
    }

    #[test]
    fn mostly_arabic_decides_not_the_first_letter() {
        // Opens with a Latin acronym, but is an Arabic sentence. Judged by
        // counting letters it reads right-to-left, so an explicit LTR on it
        // is a finding; judged by the first strong letter it would pass.
        let found = DirectionMismatch.check(&unit(
            "GPS يعتمد عليه النظام في تتبّع الشحنات",
            Resolved::Explicit(Direction::Ltr),
        ));
        assert_eq!(found.len(), 1, "{found:#?}");
    }
}

#[cfg(test)]
mod alignment_unset_tests {
    use super::*;
    use crate::text::Properties;

    fn unit(text: &str, alignment: Resolved<Alignment>) -> TextUnit {
        TextUnit::new("u1", text).with_props(Properties {
            alignment,
            ..Default::default()
        })
    }

    #[test]
    fn rtl_text_with_no_alignment_is_a_note_and_not_repaired_unless_asked() {
        let u = unit("التقرير الفصلي", Resolved::Unset);
        let rule = AlignmentUnset { align: false };
        let found = rule.check(&u);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].severity, Severity::Note);
        assert!(!found[0].fixable);
        assert_eq!(rule.fix(&u), None);

        let rule = AlignmentUnset { align: true };
        let found = rule.check(&u);
        assert!(found[0].fixable);
        assert_eq!(rule.fix(&u), Some(Fix::SetAlignment(Alignment::Start)));
    }

    #[test]
    fn an_alignment_the_author_wrote_or_inherited_is_not_this_finding() {
        let rule = AlignmentUnset { align: true };
        for alignment in [
            Resolved::Explicit(Alignment::Center),
            Resolved::Explicit(Alignment::Left), // alignment-incoherent's
            Resolved::Inherited(Alignment::Left), // invariant 2
        ] {
            assert!(
                rule.check(&unit("التقرير الفصلي", alignment.clone()))
                    .is_empty(),
                "{alignment:?}"
            );
        }
    }

    #[test]
    fn text_that_reads_left_to_right_is_left_alone() {
        let rule = AlignmentUnset { align: true };
        assert!(
            rule.check(&unit(
                "Q4 results for قطاع الطاقة were strong",
                Resolved::Unset
            ))
            .is_empty()
        );
        assert!(
            rule.check(&unit("Quarterly report", Resolved::Unset))
                .is_empty()
        );
        // Mostly Arabic, though it opens with an acronym: reported.
        assert_eq!(
            rule.check(&unit(
                "GPS يعتمد عليه النظام في تتبّع الشحنات",
                Resolved::Unset
            ))
            .len(),
            1
        );
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
