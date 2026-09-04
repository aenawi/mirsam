//! Rules about base direction and alignment.

use super::Rule;
use crate::bidi;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::fix::Fix;
use crate::script;
use crate::text::{Alignment, Direction, Resolved, TextUnit, UnitKind};

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

/// How a finding on one kind of container reads: what to call it, how to
/// refer to the text it lays out, and what goes wrong when its direction is
/// not the one that text reads.
fn wording(kind: UnitKind) -> (&'static str, &'static str, &'static str) {
    match kind {
        UnitKind::Table => ("table", "its cells read", "the columns run the wrong way"),
        UnitKind::Columns => (
            "text body",
            "its text reads",
            "the columns flow the wrong way",
        ),
        // Which chart container it is — axis, legend, labels — is in the
        // finding's location, where a reader looks to find it in the file.
        UnitKind::ChartText => (
            "chart text",
            "the strings it draws read",
            "they are laid out the wrong way round",
        ),
        // `applies_to` never hands this rule a paragraph.
        UnitKind::Paragraph => ("container", "its text reads", "it starts on the wrong side"),
    }
}

/// A container whose contents run against the direction they read.
///
/// A container is a unit of its own kind: its text is the text it lays out,
/// and its direction is its own — `a:tblPr/@rtl` for a table,
/// `a:bodyPr/@rtlCol` for a text body in two or more columns — which decides
/// which side the reader starts on. Judged from the letters, like a paragraph
/// (ADR 0006). The paragraphs inside keep their own direction and alignment
/// and stay the paragraph rules' business: DrawingML does not make a cell's
/// or a column's text inherit the container's, so both have to be right, and
/// both are reported separately.
///
/// One rule rather than one per container, because the judgement does not
/// vary with the container: only the attribute an adapter lowers the repair
/// onto does, and that is the adapter's business.
pub struct ContainerDirection;

impl Rule for ContainerDirection {
    fn id(&self) -> RuleId {
        RuleId("container-direction")
    }

    fn description(&self) -> &'static str {
        "A container's contents run against the direction its text reads"
    }

    fn applies_to(&self, kind: UnitKind) -> bool {
        kind != UnitKind::Paragraph
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        let (subject, reads, consequence) = wording(unit.kind);
        let expected = bidi::dominant_direction(&unit.text);
        let message = match unit.props.direction {
            // The container's design, never a finding.
            Resolved::Inherited(_) => return Vec::new(),
            Resolved::Explicit(declared) if declared == expected => return Vec::new(),
            // Left-to-right is what an undeclared container gets, and is right.
            Resolved::Unset if expected == Direction::Ltr => return Vec::new(),
            Resolved::Explicit(declared) => {
                format!("{subject} declared {declared} but {reads} {expected}; {consequence}")
            }
            Resolved::Unset => {
                format!("{subject} declares no direction; {reads} {expected}, so {consequence}")
            }
        };
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                message,
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                ..Default::default()
            })
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        Some(Fix::SetDirection(bidi::dominant_direction(&unit.text)))
    }
}

#[cfg(test)]
mod container_direction_tests {
    use super::*;
    use crate::rules::Engine;
    use crate::text::Properties;

    fn container(kind: UnitKind, text: &str, direction: Resolved<Direction>) -> TextUnit {
        TextUnit::new("s#c1", text)
            .with_kind(kind)
            .with_props(Properties {
                direction,
                ..Default::default()
            })
    }

    fn table(text: &str, direction: Resolved<Direction>) -> TextUnit {
        container(UnitKind::Table, text, direction)
    }

    fn columns(text: &str, direction: Resolved<Direction>) -> TextUnit {
        container(UnitKind::Columns, text, direction)
    }

    const ARABIC: &str = "المؤشر\nالربع الثالث\nالربع الرابع\n2,100\n2,300";

    #[test]
    fn an_arabic_container_with_no_direction_is_a_warning_with_a_fix() {
        for u in [
            table(ARABIC, Resolved::Unset),
            columns(ARABIC, Resolved::Unset),
        ] {
            let found = ContainerDirection.check(&u);
            assert_eq!(found.len(), 1, "{found:#?}");
            assert_eq!(found[0].severity, Severity::Warning);
            assert!(found[0].fixable);
            assert_eq!(
                ContainerDirection.fix(&u),
                Some(Fix::SetDirection(Direction::Rtl))
            );
        }
    }

    #[test]
    fn each_kind_is_named_in_its_own_words() {
        // The finding has to say what is wrong with *this* container, or a
        // reader cannot act on it without opening the file.
        let table = &ContainerDirection.check(&table(ARABIC, Resolved::Unset))[0];
        assert!(
            table.message.contains("table declares no direction"),
            "{table:#?}"
        );
        assert!(table.message.contains("the columns run the wrong way"));

        let columns = &ContainerDirection.check(&columns(ARABIC, Resolved::Unset))[0];
        assert!(
            columns.message.contains("text body declares no direction"),
            "{columns:#?}"
        );
        assert!(columns.message.contains("the columns flow the wrong way"));
    }

    #[test]
    fn a_declared_direction_contrary_to_the_contents_is_a_warning_either_way() {
        assert_eq!(
            ContainerDirection
                .check(&table(ARABIC, Resolved::Explicit(Direction::Ltr)))
                .len(),
            1
        );
        assert_eq!(
            ContainerDirection
                .check(&columns(ARABIC, Resolved::Explicit(Direction::Ltr)))
                .len(),
            1
        );
        // An English table forced right-to-left has its columns reversed too.
        let english = table(
            "Metric\nThird quarter\nFourth quarter\nRevenue (قطاع الطاقة)",
            Resolved::Explicit(Direction::Rtl),
        );
        assert_eq!(ContainerDirection.check(&english).len(), 1);
        assert_eq!(
            ContainerDirection.fix(&english),
            Some(Fix::SetDirection(Direction::Ltr))
        );
    }

    #[test]
    fn a_correct_inherited_or_english_container_is_silent() {
        for u in [
            table(ARABIC, Resolved::Explicit(Direction::Rtl)),
            table(ARABIC, Resolved::Inherited(Direction::Ltr)),
            table("Metric\nQ3\nQ4", Resolved::Unset),
            table(
                "Metric\nThird quarter (قطاع الطاقة)\nFourth quarter",
                Resolved::Unset,
            ),
            columns(ARABIC, Resolved::Explicit(Direction::Rtl)),
            columns(ARABIC, Resolved::Inherited(Direction::Ltr)),
            columns("Two columns of English prose", Resolved::Unset),
        ] {
            assert!(ContainerDirection.check(&u).is_empty(), "{u:#?}");
        }
    }

    #[test]
    fn the_engine_hands_each_kind_only_the_rules_that_judge_it() {
        // A container unit carries no language, font or alignment of its own;
        // the paragraph rules must not report those as missing on it. And a
        // paragraph is never a container.
        let engine = Engine::with_default_rules();
        for u in [
            table(ARABIC, Resolved::Unset),
            columns(ARABIC, Resolved::Unset),
        ] {
            let report = engine.audit(&[u]);
            let rules: Vec<_> = report.diagnostics.iter().map(|d| d.rule.0).collect();
            assert_eq!(rules, ["container-direction"], "{report:#?}");
        }

        let paragraph = TextUnit::new("s#p1", "المؤشر");
        assert!(
            engine
                .audit(&[paragraph])
                .diagnostics
                .iter()
                .all(|d| d.rule.0 != "container-direction")
        );
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
