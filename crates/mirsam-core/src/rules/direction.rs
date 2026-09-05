//! Rules about base direction and alignment.

use super::Rule;
use crate::bidi;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::fix::Fix;
use crate::script;
use crate::text::{Alignment, Direction, Origin, Resolved, TextUnit, UnitKind};

/// The direction these rules judge: the one written on the paragraph, or —
/// where none is — UAX#9's first-strong auto-detection.
///
/// Deliberately *not* `Resolved::effective`, which would hand back an
/// inherited value too. Resolving the chain says what the reader will see; it
/// does not say that anyone wrote a direction here, and an English template's
/// untouched `rtl="0"` is not a claim about the Arabic under it. Feeding it to
/// `direction-mismatch` would turn a deck's warnings into errors on the
/// strength of a template default. A contradicting inherited value is
/// `direction-unset`'s finding, at the severity an absent one carries
/// (ADR 0007 §3).
fn judged_direction(unit: &TextUnit) -> Direction {
    match unit.props.direction {
        Resolved::Explicit(direction) => direction,
        _ => bidi::auto_direction(&unit.text),
    }
}

/// A value taken from a stand-in ancestor, for the tests below: what matters
/// to a rule is that it was not written on the unit, not which part said it.
#[cfg(test)]
fn inherited<T>(value: T) -> Resolved<T> {
    Resolved::Inherited(
        value,
        crate::text::Origin::new("ppt/slideMasters/slideMaster1.xml", "bodyStyle/lvl1pPr"),
    )
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
        let actual = judged_direction(unit);
        let evidence = || Evidence {
            logical: Some(unit.text.clone()),
            visual_declared: Some(bidi::resolve(&unit.text, actual).visual),
            visual_expected: Some(bidi::resolve(&unit.text, expected).visual),
            offenders: Vec::new(),
            // The direction this rule judges is one the author wrote or the
            // renderer's own auto-detection. Neither has a part to name, and
            // naming one the rule did not consult would be worse than naming
            // none.
            inherited_from: None,
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

/// Arabic text with no base direction of its own.
///
/// Two cases, one finding. The paragraph declares nothing and nothing above it
/// does either, so the renderer auto-detects — fragile rather than broken, and
/// a warning. Or the chain does supply a direction, but one that *contradicts*
/// the letters: an English template's untouched `rtl="0"` under Arabic, which
/// is the absence of a decision wearing the same clothes as one. Inheritance
/// resolves the value; it does not establish that anyone chose it
/// (ADR 0007 §1), so that case is the same warning, and its evidence names the
/// part that supplied the value so a reader can check the claim.
///
/// An inherited direction that *agrees* with the text is the layout doing its
/// job, and is silent. That is the milestone's acceptance (ADR 0007 §4).
pub struct DirectionUnset;

impl Rule for DirectionUnset {
    fn id(&self) -> RuleId {
        RuleId("direction-unset")
    }

    fn description(&self) -> &'static str {
        "Arabic text has no base direction of its own, and none is inherited that agrees with it"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        // A direction the author wrote is `direction-mismatch`'s business,
        // whether it agrees with the text or not.
        if !script::has_arabic(&unit.text) || unit.props.direction.is_explicit() {
            return Vec::new();
        }
        let expected = bidi::dominant_direction(&unit.text);
        // An inherited value consistent with the text is a choice; nothing to
        // report, and the whole point of resolving the chain.
        if unit.props.direction.effective() == Some(&expected) {
            return Vec::new();
        }
        // Already reported, with both renderings as evidence, by
        // DirectionMismatch: with nothing written here it judges the
        // auto-detected direction, which is what an unset paragraph gets and
        // what an inherited one gets too (ADR 0007 §3).
        if bidi::order_differs(&unit.text, judged_direction(unit), expected) {
            return Vec::new();
        }

        let (message, evidence) = match (
            unit.props.direction.effective(),
            unit.props.direction.origin(),
        ) {
            (Some(inherited), Some(origin)) => (
                format!(
                    "no base direction of its own; inherits {inherited}, but this reads as {expected}"
                ),
                Evidence {
                    inherited_from: Some(origin.to_string()),
                    ..Default::default()
                },
            ),
            _ => (
                "no base direction declared; correct today only by auto-detection".to_string(),
                Evidence::default(),
            ),
        };

        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                message,
            )
            .with_evidence(evidence)
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        // On the paragraph the finding names, never on the master that
        // supplied the contradicting value: setting `rtl="1"` there would
        // change every paragraph in the deck (ADR 0007 §6).
        Some(Fix::SetDirection(bidi::dominant_direction(&unit.text)))
    }
}

/// A hard left alignment written on right-to-left text.
///
/// Centre, justify and the direction-relative alignments are all legitimate and
/// are deliberately left alone. So is an *inherited* left alignment, which is
/// not this rule's finding but `alignment-unset`'s: the severity differs
/// because writing `algn="l"` on Arabic is a mistake someone made, while
/// inheriting it is a template default nobody aimed at the text
/// (ADR 0007 §3).
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
        // Only a left alignment the author *wrote* is this finding. An
        // inherited one is `alignment-unset`'s, at note severity.
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

/// Right-to-left text with no alignment of its own, and none inherited that
/// reads correctly.
///
/// Where the chain supplies nothing, the renderer's own default decides, and
/// on a left-to-right template that is the left edge — the very thing
/// `alignment-incoherent` reports when it is written on the paragraph. Where
/// the chain supplies `algn="l"`, the reader really does start on the edge
/// they do not read from, and that is reported for the same reason: an
/// inherited default is not the author's choice (ADR 0007 §1).
///
/// A centred or right-aligned inherited value is silent. A layout that centres
/// a title has made a design decision that reads correctly in either
/// direction, and this is the case that earns the distinction — it is what
/// retires ADR 0006's cost note that `--align` pushes such a title to the
/// right edge.
///
/// A note either way: it never blocks, and it is repaired only when the caller
/// asks with `RepairOptions::align`.
pub struct AlignmentUnset {
    /// Whether the caller asked for the repair.
    pub align: bool,
}

impl Rule for AlignmentUnset {
    fn id(&self) -> RuleId {
        RuleId("alignment-unset")
    }

    fn description(&self) -> &'static str {
        "Right-to-left text has no alignment of its own, and none is inherited that reads correctly"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text)
            || bidi::dominant_direction(&unit.text) != Direction::Rtl
            // An alignment the author wrote is `alignment-incoherent`'s business.
            || unit.props.alignment.is_explicit()
        {
            return Vec::new();
        }
        // Only a hard `Left` contradicts right-to-left text; centre, justify
        // and the direction-relative alignments all read correctly.
        if unit
            .props
            .alignment
            .effective()
            .is_some_and(|a| a.is_rtl_coherent())
        {
            return Vec::new();
        }

        let (message, inherited_from) = match unit.props.alignment.origin() {
            Some(origin) => (
                "no alignment of its own; the alignment it inherits is left, \
                 the edge a right-to-left reader does not start from",
                Some(origin.to_string()),
            ),
            None => (
                "no alignment declared; a left-to-right layout places this on the left edge",
                None,
            ),
        };
        let diagnostic =
            Diagnostic::new(self.id(), Severity::Note, &unit.id, &unit.location, message)
                .with_evidence(Evidence {
                    logical: Some(unit.text.clone()),
                    inherited_from,
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
///
/// A container's direction can be inherited — Word answers a `w:tbl` that
/// states no `w:bidiVisual` from the table style above it — and ADR 0007 §1
/// decides what to conclude from that: an inherited value that agrees with
/// the text is the design doing its job and is silent, and one that
/// contradicts it is a default nobody aimed at this container and is reported
/// exactly as an absent one. The repair still writes to the container the
/// finding names, never to the style (ADR 0007 §6).
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
            // Whoever said it, a direction that agrees with the text is the
            // container laid out the way it reads (ADR 0007 §1, §4).
            Resolved::Explicit(declared) | Resolved::Inherited(declared, _)
                if declared == expected =>
            {
                return Vec::new();
            }
            Resolved::Explicit(declared) => {
                format!("{subject} declared {declared} but {reads} {expected}; {consequence}")
            }
            // Left-to-right is what an undeclared container gets, and is right.
            Resolved::Unset | Resolved::Inherited(..) if expected == Direction::Ltr => {
                return Vec::new();
            }
            // Undeclared, or declared by a style whose value contradicts the
            // text — a default nobody aimed at this container, reported
            // exactly as an absent one and at the severity an absent one
            // carries (ADR 0007 §1, §3).
            Resolved::Unset | Resolved::Inherited(..) => {
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
                // Present exactly when a source above the container supplied
                // the direction, so a reviewer can check the claim without
                // opening the application (invariant 6, ADR 0007 §5).
                inherited_from: unit.props.direction.origin().map(Origin::to_string),
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
            // The style laid the table out the way its text reads: the design
            // doing its job, and never a finding (ADR 0007 §1).
            table(ARABIC, inherited(Direction::Rtl)),
            table("Metric\nQ3\nQ4", Resolved::Unset),
            table("Metric\nQ3\nQ4", inherited(Direction::Ltr)),
            table(
                "Metric\nThird quarter (قطاع الطاقة)\nFourth quarter",
                Resolved::Unset,
            ),
            columns(ARABIC, Resolved::Explicit(Direction::Rtl)),
            columns(ARABIC, inherited(Direction::Rtl)),
            columns("Two columns of English prose", Resolved::Unset),
        ] {
            assert!(ContainerDirection.check(&u).is_empty(), "{u:#?}");
        }
    }

    #[test]
    fn an_inherited_direction_contradicting_the_text_is_reported_as_an_absent_one() {
        // A table style that lays its tables out left to right is a default
        // nobody aimed at Arabic, and the reader still meets the columns the
        // wrong way round. ADR 0007 §1 and §3: reported, at the severity an
        // absent direction carries, and naming the source so the claim can be
        // checked without opening Word (§5).
        for u in [
            table(ARABIC, inherited(Direction::Ltr)),
            columns(ARABIC, inherited(Direction::Ltr)),
        ] {
            let found = ContainerDirection.check(&u);
            assert_eq!(found.len(), 1, "{found:#?}");
            assert_eq!(found[0].severity, Severity::Warning);
            assert!(found[0].message.contains("declares no direction"));
            assert_eq!(
                found[0].evidence.inherited_from.as_deref(),
                Some("ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr")
            );
            // The repair writes to the container, never to the source
            // above it (ADR 0007 §6).
            assert_eq!(
                ContainerDirection.fix(&u),
                Some(Fix::SetDirection(Direction::Rtl))
            );
        }
    }

    #[test]
    fn a_direction_written_on_the_container_names_no_source() {
        let found = &ContainerDirection.check(&table(ARABIC, Resolved::Unset))[0];
        assert!(found.evidence.inherited_from.is_none(), "{found:#?}");
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
        // Absent is direction-unset's finding, and so is inherited: an
        // inherited value is not one anyone wrote here.
        for direction in [Resolved::Unset, inherited(Direction::Ltr)] {
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
    fn an_inherited_contradiction_does_not_escalate_a_warning_into_an_error() {
        // ADR 0007 §3. This text reorders differently under the two
        // directions, so judging it by the master's `rtl="0"` would make it
        // an error. The finding stays `direction-unset`'s warning, at the
        // severity an absent direction carries, and it names the part.
        let text = "ارتفع الأداء بنسبة 25% في Q4 2026.";
        let u = unit(text, inherited(Direction::Ltr));
        assert!(DirectionMismatch.check(&u).is_empty(), "{u:#?}");

        let found = DirectionUnset.check(&u);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].severity, Severity::Warning);
        assert_eq!(
            found[0].evidence.inherited_from.as_deref(),
            Some("ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr")
        );
        // Written on the paragraph the finding names, never on the master
        // that supplied the value (ADR 0007 §6).
        assert_eq!(
            DirectionUnset.fix(&u),
            Some(Fix::SetDirection(Direction::Rtl))
        );

        // The same text with the direction actually written is still an error.
        let written = unit(text, Resolved::Explicit(Direction::Ltr));
        assert_eq!(
            DirectionMismatch.check(&written)[0].severity,
            Severity::Error
        );
    }

    #[test]
    fn an_inherited_direction_that_agrees_with_the_text_is_silent() {
        // The case invariant 2 was written to protect: Arabic under an Arabic
        // master, which no rule may report.
        let u = unit("التقرير الفصلي", inherited(Direction::Rtl));
        assert!(DirectionUnset.check(&u).is_empty(), "{u:#?}");
        assert!(DirectionMismatch.check(&u).is_empty(), "{u:#?}");

        // And the same for English under an English master.
        let english = unit("Quarterly report", inherited(Direction::Ltr));
        assert!(DirectionUnset.check(&english).is_empty());
    }

    #[test]
    fn nothing_declared_anywhere_still_reads_as_auto_detection() {
        // The message an unset paragraph carries does not change with M2, and
        // it names no part, because none supplied a value.
        let found = DirectionUnset.check(&unit("التقرير الفصلي", Resolved::Unset));
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(
            found[0]
                .message
                .contains("correct today only by auto-detection"),
            "{:?}",
            found[0].message
        );
        assert_eq!(found[0].evidence.inherited_from, None);
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
    fn an_alignment_the_author_wrote_is_not_this_finding() {
        let rule = AlignmentUnset { align: true };
        for alignment in [
            Resolved::Explicit(Alignment::Center),
            Resolved::Explicit(Alignment::Left), // alignment-incoherent's
        ] {
            assert!(
                rule.check(&unit("التقرير الفصلي", alignment.clone()))
                    .is_empty(),
                "{alignment:?}"
            );
        }
    }

    #[test]
    fn an_inherited_alignment_that_reads_correctly_is_silent() {
        // ADR 0007 §4. A layout that centres or right-aligns has made a
        // decision that reads correctly right-to-left, and silencing it is
        // what retires ADR 0006's cost note about `--align` pushing a centred
        // title to the right edge.
        let rule = AlignmentUnset { align: true };
        for alignment in [
            Alignment::Center,
            Alignment::Right,
            Alignment::Start,
            Alignment::Justify,
        ] {
            assert!(
                rule.check(&unit("التقرير الفصلي", inherited(alignment)))
                    .is_empty(),
                "{alignment:?}"
            );
        }
    }

    #[test]
    fn an_inherited_left_alignment_is_reported_and_names_its_source() {
        // An English template's untouched `algn="l"` under Arabic puts the
        // text on the edge a reader does not start from. Same note severity
        // as an absent alignment (ADR 0007 §3), and the part is named so the
        // claim can be checked without opening the application.
        let rule = AlignmentUnset { align: true };
        let u = unit("التقرير الفصلي", inherited(Alignment::Left));
        let found = rule.check(&u);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].severity, Severity::Note);
        assert_eq!(
            found[0].evidence.inherited_from.as_deref(),
            Some("ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr")
        );
        assert_eq!(rule.fix(&u), Some(Fix::SetAlignment(Alignment::Start)));

        // Still a note, and still not repaired unless the caller asks.
        let found = AlignmentUnset { align: false }.check(&u);
        assert!(!found[0].fixable);
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
