//! Rules about text whose order the document decided instead of the algorithm.
//!
//! Every other direction rule in this engine asks about a whole unit: what base
//! direction was declared for it, which edge it starts on. These two ask about a
//! *part* of one, because that is where the two remaining ways to get
//! bidirectional text wrong actually live.
//!
//! The first is an order the document **imposed**. `<bdo dir="rtl">` and an
//! embedded U+202E are the same instruction — lay these characters out right to
//! left whatever they are — and both replace the algorithm rather than
//! informing it. `bidi-control` already reports the character; this reports the
//! markup, and the two findings say the same sentence about the same defect
//! (invariant 4: direction belongs to the container).
//!
//! The second is a run the document left **un-isolated**. A run whose direction
//! differs from the text around it does not merely render itself: the neutrals
//! on either side of it — a colon, a dash, a trailing number — resolve against
//! the strong characters it happens to begin and end with, so the layout
//! *outside* the run is decided by what is *inside* it. That is fine while the
//! text is the text the author saw, and it is the single most common way a page
//! that looked right in review breaks on real data. `<bdi>` exists for exactly
//! this, and asking whether isolating the run changes anything is how the tool
//! proves the finding instead of asserting it (ADR 0004).
//!
//! Both rules read [`TextUnit::spans`], which an adapter fills in only if its
//! format can state something about a range. An adapter that says nothing
//! leaves both rules silent rather than having them guess — a run boundary
//! cannot be recovered from the characters, and inventing one would report a
//! defect nobody wrote.

use super::Rule;
use super::direction::judged_direction;
use crate::bidi;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::script;
use crate::text::{Direction, SpanBidi, TextUnit};

/// The order of a run is imposed by the document rather than resolved.
///
/// Two tiers, on the same test the flagship direction rule uses. When the
/// imposed order differs from the order the algorithm gives that run under the
/// very direction the override names, the override is doing damage and the two
/// renderings in the evidence prove it: an error. When they agree — an override
/// wrapped around text with no digits, no Latin and no punctuation, where
/// reversing the characters and resolving them come to the same thing — nothing
/// is wrong on the screen today, and the finding is a warning at the fragile
/// tier `dir="auto"` occupies. The markup still says *ignore the algorithm*, and
/// the day someone types a year into it, it will.
pub struct BidiOverride;

impl Rule for BidiOverride {
    fn id(&self) -> RuleId {
        RuleId("bidi-override")
    }

    fn description(&self) -> &'static str {
        "A run's order is imposed by the document instead of resolved by the bidirectional algorithm"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        let mut out = Vec::new();

        for span in &unit.spans {
            let SpanBidi::Imposed(direction) = span.bidi else {
                continue;
            };
            let Some(run) = span.text(&unit.text).filter(|run| !run.is_empty()) else {
                continue;
            };

            // What the document asked for, against what the same run would look
            // like had the direction merely been *declared* on it. That is the
            // repair a reviewer would make, so it is the comparison worth
            // showing.
            let declared = bidi::imposed(run, direction);
            let expected = bidi::resolve(run, direction).visual;
            let evidence = Evidence {
                logical: Some(run.to_string()),
                visual_declared: Some(declared.clone()),
                visual_expected: Some(expected.clone()),
                offenders: vec![format!(
                    "{} \u{d7}{} @{}",
                    span.origin.property, span.len, span.offset
                )],
                // What imposed the order, which is never the unit itself: a
                // reviewer cannot check this finding without being told which
                // element or declaration to look at (invariant 6).
                inherited_from: Some(span.origin.to_string()),
            };

            out.push(if declared == expected {
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    &unit.id,
                    &unit.location,
                    format!(
                        "a run of this text has its order imposed {direction} rather than \
                         declared; this run reorders the same either way, so nothing moves \
                         today — but the algorithm is switched off, and the first digit, Latin \
                         word or edge punctuation typed into it will come out backwards"
                    ),
                )
                .with_evidence(evidence)
            } else {
                Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    &unit.id,
                    &unit.location,
                    format!(
                        "a run of this text has its order imposed {direction} rather than \
                         declared, and the imposed order is not the one it reads in; declare \
                         the direction on the run instead of overriding the algorithm"
                    ),
                )
                .with_evidence(evidence)
            });
        }

        out
    }
}

/// A run whose content decides the order of the text around it.
///
/// The precondition is deliberately narrow, because the wide version of this
/// question reports ordinary markup. A run is only asked about when it carries a
/// strong character of the direction *opposite* to the unit's own — an English
/// name in an Arabic line, an Arabic name in an English one — which is the case
/// `<bdi>` was added to the language for. A run of digits, a run of the same
/// direction, and a run of punctuation are all left alone, and so is any run
/// that turns out to change nothing when isolated, whatever it contains.
///
/// A warning, at the fragile tier rather than the broken one. Nothing is
/// mis-rendered today: the algorithm did exactly what it is specified to do with
/// the characters it was given. What is wrong is that the layout of text outside
/// the run is a function of text inside it, so the page is correct for this
/// content and undefined for the next.
pub struct IsolationMissing;

impl Rule for IsolationMissing {
    fn id(&self) -> RuleId {
        RuleId("isolation-missing")
    }

    fn description(&self) -> &'static str {
        "An un-isolated run of the opposite direction decides the order of the text around it"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        let base = judged_direction(unit);
        let mut out = Vec::new();

        for span in &unit.spans {
            if span.bidi != SpanBidi::Plain {
                continue;
            }
            let Some(run) = span.text(&unit.text) else {
                continue;
            };
            if !carries_strong(run, opposite(base)) {
                continue;
            }
            let Some(isolated) = bidi::resolve_isolating(&unit.text, base, span.offset, span.len)
            else {
                continue;
            };
            let declared = bidi::resolve(&unit.text, base);
            if isolated.visual == declared.visual {
                continue;
            }

            out.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    &unit.id,
                    &unit.location,
                    format!(
                        "a run of this text reads {} inside {base} text and is not isolated \
                         from it; the order of the text around the run is decided by what is \
                         inside it, so the line is laid out correctly for this content and not \
                         for the next",
                        opposite(base)
                    ),
                )
                .with_evidence(Evidence {
                    logical: Some(unit.text.clone()),
                    // The claim, in the two forms a reviewer can compare: what
                    // the line resolves to now, and what it resolves to once the
                    // run stops deciding for its neighbours.
                    visual_declared: Some(declared.visual),
                    visual_expected: Some(isolated.visual),
                    offenders: vec![format!(
                        "{} \u{d7}{} @{}",
                        span.origin.property, span.len, span.offset
                    )],
                    inherited_from: Some(span.origin.to_string()),
                }),
            );
        }

        out
    }
}

fn opposite(direction: Direction) -> Direction {
    match direction {
        Direction::Rtl => Direction::Ltr,
        Direction::Ltr => Direction::Rtl,
    }
}

/// Whether `run` contains a strong character of `direction`.
///
/// Strong is the word UAX#9 uses and the word that matters here: only a strong
/// character can set the direction a neighbouring neutral resolves to, so only a
/// run holding one can decide anything outside itself. Digits are weak and a
/// run of them is left alone.
fn carries_strong(run: &str, direction: Direction) -> bool {
    match direction {
        Direction::Rtl => run.chars().any(script::is_arabic_letter),
        Direction::Ltr => run.chars().any(|c| c.is_ascii() && c.is_alphabetic()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Direction, Origin, Properties, Resolved, Span, TextUnit};

    const ARABIC: &str = "ارتفع الأداء في الربع الرابع";

    fn origin(what: &str) -> Origin {
        Origin::new("page.html", what)
    }

    /// A unit carrying one delimited run over the byte range of `run` within
    /// `text`, which is how an adapter reports an inline element.
    fn with_run(text: &str, run: &str, bidi: SpanBidi) -> TextUnit {
        let offset = text.find(run).expect("the run is in the text");
        TextUnit::new("page.html#p1", text).with_spans(vec![Span::new(
            offset,
            run.len(),
            bidi,
            origin("bdo@dir"),
        )])
    }

    fn rtl(unit: TextUnit) -> TextUnit {
        let props = Properties {
            direction: Resolved::Explicit(Direction::Rtl),
            ..Default::default()
        };
        unit.with_props(props)
    }

    // ------------------------------------------------------- bidi-override

    #[test]
    fn an_override_that_moves_the_text_is_an_error_with_both_orders() {
        let text = "ارتفع الأداء بنسبة 25% في Q4 2026.";
        let unit = rtl(with_run(text, "Q4 2026", SpanBidi::Imposed(Direction::Rtl)));
        let found = BidiOverride.check(&unit);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(
            found[0].evidence.visual_declared.as_deref(),
            Some("6202 4Q"),
            "the override lays the digits out backwards"
        );
        assert_eq!(
            found[0].evidence.visual_expected.as_deref(),
            Some("Q4 2026"),
            "declaring the same direction leaves them alone"
        );
        assert_eq!(
            found[0].evidence.inherited_from.as_deref(),
            Some("page.html bdo@dir"),
            "a finding a reviewer cannot locate is not finished"
        );
    }

    #[test]
    fn an_override_that_moves_nothing_is_the_fragile_tier() {
        // Pure Arabic reverses to the same thing either way, so the screen is
        // right today. The algorithm is still switched off.
        let unit = rtl(with_run(ARABIC, "الأداء", SpanBidi::Imposed(Direction::Rtl)));
        let found = BidiOverride.check(&unit);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(!found[0].fixable);
    }

    #[test]
    fn a_run_that_states_nothing_is_not_an_override() {
        for bidi in [SpanBidi::Plain, SpanBidi::Isolated] {
            let unit = rtl(with_run(ARABIC, "الأداء", bidi));
            assert!(BidiOverride.check(&unit).is_empty(), "{bidi:?}");
        }
    }

    #[test]
    fn a_unit_with_no_spans_is_a_unit_nothing_was_said_about() {
        // The adapters that cannot state a range say nothing, and get silence
        // rather than a guess.
        assert!(
            BidiOverride
                .check(&rtl(TextUnit::new("s#p1", ARABIC)))
                .is_empty()
        );
        assert!(
            IsolationMissing
                .check(&rtl(TextUnit::new("s#p1", ARABIC)))
                .is_empty()
        );
    }

    #[test]
    fn a_range_outside_the_text_is_answered_with_silence() {
        let unit = rtl(
            TextUnit::new("page.html#p1", ARABIC).with_spans(vec![Span::new(
                1,
                2,
                SpanBidi::Imposed(Direction::Rtl),
                origin("bdo@dir"),
            )]),
        );
        assert!(BidiOverride.check(&unit).is_empty());
    }

    // --------------------------------------------------- isolation-missing

    #[test]
    fn an_unisolated_name_that_decides_its_neighbours_is_reported() {
        // The `<bdi>` case: a Latin name dropped into an Arabic line, with a
        // neutral after it that follows whatever the name ends with.
        let text = "المالك: John Smith - 5";
        let unit = rtl(with_run(text, "John Smith", SpanBidi::Plain));
        let found = IsolationMissing.check(&unit);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert_ne!(
            found[0].evidence.visual_declared, found[0].evidence.visual_expected,
            "the finding is the difference between these two"
        );
    }

    #[test]
    fn isolating_it_makes_the_finding_go_away() {
        let text = "المالك: John Smith - 5";
        let unit = rtl(with_run(text, "John Smith", SpanBidi::Isolated));
        assert!(IsolationMissing.check(&unit).is_empty());
    }

    #[test]
    fn ordinary_markup_is_left_alone() {
        // Every one of these is a run a real page has in it, and a rule that
        // reported them would be noise rather than a check.
        let text = "ارتفع الأداء بنسبة 25% في الربع الرابع";
        for run in ["الأداء", "25%", "الربع الرابع"] {
            let unit = rtl(with_run(text, run, SpanBidi::Plain));
            assert!(
                IsolationMissing.check(&unit).is_empty(),
                "{run:?} was reported"
            );
        }
    }

    #[test]
    fn a_latin_run_that_changes_nothing_around_it_is_left_alone() {
        // Opposite direction, and still silent: it sits between two Arabic
        // words with nothing neutral to drag with it. The rule reports the
        // proven case, not the suspicious one (ADR 0004).
        let text = "ارتفع الأداء في Q4 الربع الرابع";
        let unit = rtl(with_run(text, "Q4", SpanBidi::Plain));
        assert!(IsolationMissing.check(&unit).is_empty());
    }

    #[test]
    fn a_page_with_no_arabic_is_not_this_tools_business() {
        let text = "Owner: John Smith - 5";
        let unit = with_run(text, "John Smith", SpanBidi::Plain);
        assert!(IsolationMissing.check(&unit).is_empty());
        let unit = with_run(text, "John Smith", SpanBidi::Imposed(Direction::Rtl));
        assert!(BidiOverride.check(&unit).is_empty());
    }
}
