//! Rules about the codepoints themselves.

use super::Rule;
use crate::diagnostic::{Diagnostic, Evidence, RuleId, Severity};
use crate::fix::Fix;
use crate::tatweel::{self, Run};
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
///
/// Two findings, because two different things live in those blocks. A
/// contextual letter form (U+FEF2 for a final yeh) is a shaping artefact
/// stored where a logical codepoint belongs: an error, and mechanically
/// repairable, since the letter it stands for is known. A word ligature
/// (U+FDFA ﷺ) is content the author chose: reported, because many fonts lack
/// the glyph and a search for the spelled-out phrase will not match it, but
/// never expanded, because that would rewrite what they wrote.
pub struct PresentationForms;

/// The distinct codepoints in `text` that satisfy `pred`, as `U+XXXX`, so a
/// reviewer can verify the finding without rendering the text.
fn offending_codepoints(text: &str, pred: fn(char) -> bool) -> Vec<String> {
    let mut codes: Vec<u32> = text.chars().filter(|c| pred(*c)).map(u32::from).collect();
    codes.sort_unstable();
    codes.dedup();
    codes.into_iter().map(|cp| format!("U+{cp:04X}")).collect()
}

impl Rule for PresentationForms {
    fn id(&self) -> RuleId {
        RuleId("presentation-forms")
    }

    fn description(&self) -> &'static str {
        "Text stores pre-shaped Arabic Presentation Forms instead of logical-order codepoints"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        let forms = unit
            .text
            .chars()
            .filter(|c| script::is_presentation_form(*c))
            .count();
        if forms > 0 {
            out.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Error,
                    &unit.id,
                    &unit.location,
                    format!(
                        "{forms} pre-shaped presentation form(s); text is not searchable or reflowable"
                    ),
                )
                .with_evidence(Evidence {
                    logical: Some(unit.text.clone()),
                    offenders: offending_codepoints(&unit.text, script::is_presentation_form),
                    ..Default::default()
                })
                .fixable(),
            );
        }

        let ligatures = unit
            .text
            .chars()
            .filter(|c| script::is_word_ligature(*c))
            .count();
        if ligatures > 0 {
            out.push(
                Diagnostic::new(
                    self.id(),
                    Severity::Warning,
                    &unit.id,
                    &unit.location,
                    format!(
                        "{ligatures} Arabic word ligature(s); left as authored, but many fonts lack the glyph and search will not match the spelled-out phrase"
                    ),
                )
                .with_evidence(Evidence {
                    logical: Some(unit.text.clone()),
                    offenders: offending_codepoints(&unit.text, script::is_word_ligature),
                    ..Default::default()
                }),
            );
        }

        out
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        unit.text
            .chars()
            .any(script::is_presentation_form)
            .then_some(Fix::NormalizePresentationForms)
    }
}

/// U+0640 ARABIC TATWEEL typed as width rather than written as a character.
///
/// The rule that had to state a threshold before it could exist. Tatweel is
/// legitimate — it is the kashida, and a font inserts it at layout time — so
/// "the text contains a tatweel" is not a finding, it is [ADR 0004]'s first
/// failure mode. [`crate::tatweel`] holds the argument for where the line
/// falls; this reports what falls on the wrong side of it.
///
/// [ADR 0004]: ../../../docs/adr/0004-prove-defects-dont-assert-them.md
#[derive(Default)]
pub struct TatweelPadding {
    /// Whether a repair deletes the padding. Off by default, for
    /// [`super::typography::LiteralBullet`]'s reason: this edits the
    /// characters the author typed, not the properties around them.
    pub strip: bool,
}

impl Rule for TatweelPadding {
    fn id(&self) -> RuleId {
        RuleId("tatweel-padding")
    }

    fn description(&self) -> &'static str {
        "Typed U+0640 ARABIC TATWEEL pads text to a width instead of justifying it"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        let padding: Vec<Run> = tatweel::scan(&unit.text)
            .into_iter()
            .filter(|run| run.verdict.is_padding())
            .collect();
        if padding.is_empty() {
            return Vec::new();
        }
        let total: usize = padding.iter().map(|run| run.length).sum();

        // A warning, not an error: the text renders exactly as the author
        // arranged it. What is wrong is the string behind it, and every cost
        // below is one a screenshot cannot show.
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                format!(
                    "{total} typed tatweel(s) stretch the text to a width; the stored text is no \
                     longer the word — search, spell-check and screen readers all read the \
                     padding, and it does not follow a change of width, font or size"
                ),
            )
            .with_evidence(Evidence {
                logical: Some(unit.text.clone()),
                // Where each run is and how long it is, so a reviewer can find
                // it in the string without counting characters.
                offenders: padding
                    .iter()
                    .map(|run| {
                        format!("U+0640 ARABIC TATWEEL \u{d7}{} @{}", run.length, run.offset)
                    })
                    .collect(),
                ..Default::default()
            })
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        if !self.strip {
            return None;
        }
        let offsets = tatweel::padding_offsets(&unit.text);
        (!offsets.is_empty()).then_some(Fix::RemoveTatweel(offsets))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(text: &str) -> TextUnit {
        TextUnit::new("s#p1", text)
    }

    #[test]
    fn a_contextual_form_is_an_error_with_a_repair() {
        let rule = PresentationForms;
        let u = unit("الملخص: ﺍﻟﺘﻘﺮﻳﺮ");
        let found = rule.check(&u);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Error);
        assert!(found[0].fixable);
        assert_eq!(
            found[0].evidence.offenders,
            ["U+FE8D", "U+FE98", "U+FEAE", "U+FED8", "U+FEDF", "U+FEF3"]
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(rule.fix(&u), Some(Fix::NormalizePresentationForms));
    }

    #[test]
    fn a_word_ligature_is_a_warning_with_no_repair() {
        let rule = PresentationForms;
        let u = unit("قال النبي \u{FDFA} ذلك");
        let found = rule.check(&u);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(!found[0].fixable);
        assert_eq!(found[0].evidence.offenders, vec!["U+FDFA".to_string()]);
        assert_eq!(rule.fix(&u), None);
    }

    #[test]
    fn a_unit_with_both_gets_both_and_the_repair_addresses_the_forms() {
        let rule = PresentationForms;
        let u = unit("\u{FDFA} ﺍﻟﺘﻘﺮﻳﺮ");
        let found = rule.check(&u);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[1].severity, Severity::Warning);
        assert_eq!(rule.fix(&u), Some(Fix::NormalizePresentationForms));
    }

    #[test]
    fn what_no_repair_can_change_is_not_reported() {
        // A byte-order mark, ornate parentheses and a pedagogical symbol sit
        // in the presentation-form blocks and have no logical equivalent.
        // Reporting them proposed a repair that changed nothing and left
        // the finding standing after it.
        let rule = PresentationForms;
        let u = unit("\u{FEFF}\u{FD3E}مرحبا\u{FD3F}\u{FBB2}");
        assert!(rule.check(&u).is_empty());
        assert_eq!(rule.fix(&u), None);
    }

    #[test]
    fn logical_order_text_is_clean() {
        let rule = PresentationForms;
        assert!(rule.check(&unit("التقرير الفصلي")).is_empty());
    }

    /// `n` tatweel between `head` and `tail`. Spelled out rather than typed
    /// into a literal, because a run of tatweel in source cannot be counted by
    /// eye.
    fn padded(head: &str, n: usize, tail: &str) -> String {
        format!("{head}{}{tail}", "\u{0640}".repeat(n))
    }

    #[test]
    fn a_heading_padded_to_a_width_is_reported_with_its_offsets() {
        let rule = TatweelPadding::default();
        let text = padded("العنوان", 5, "");
        let found = rule.check(&unit(&text));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].severity, Severity::Warning);
        assert!(found[0].fixable);
        assert!(found[0].message.contains('5'), "{}", found[0].message);
        assert_eq!(
            found[0].evidence.offenders,
            vec![format!(
                "U+0640 ARABIC TATWEEL \u{d7}5 @{}",
                "العنوان".len()
            )]
        );
    }

    #[test]
    fn a_rule_that_regressed_to_any_tatweel_is_a_defect_would_fail_here() {
        // The three legitimate uses, which is the whole reason this rule
        // needed a threshold rather than a character test.
        let rule = TatweelPadding::default();
        for text in [
            padded("", 1, "\u{064E}"),          // a fatha on its own
            padded("", 1, &padded("ه", 1, "")), // medial heh
            padded("ن", 1, ""),                 // initial noon
            padded("مرحبا ", 4, " عالم"),       // a rule drawn with it
            padded("ب", 1, "\u{064E}ت"),        // a mark inside a word
        ] {
            let unit = unit(&text);
            assert!(rule.check(&unit).is_empty(), "{text:?} was reported");
            assert_eq!(TatweelPadding { strip: true }.fix(&unit), None);
        }
    }

    #[test]
    fn padding_is_stripped_only_on_request() {
        let text = padded("الس", 3, "لام");
        let u = unit(&text);
        assert_eq!(TatweelPadding::default().fix(&u), None);
        assert_eq!(
            TatweelPadding { strip: true }.fix(&u),
            Some(Fix::RemoveTatweel(vec![6, 8, 10]))
        );
        // Reported, and reported as fixable, either way: the repair exists
        // whether or not this run opted into it.
        assert!(TatweelPadding::default().check(&u)[0].fixable);
    }

    #[test]
    fn stripping_the_padding_clears_the_finding() {
        // The fixed point the repair has to reach: what the fix deletes is
        // exactly what the check found, so a second pass reports nothing.
        let rule = TatweelPadding { strip: true };
        let text = format!("{} و{}", padded("العنوان", 5, ""), padded("الس", 1, "لام"));
        let Some(Fix::RemoveTatweel(offsets)) = rule.fix(&unit(&text)) else {
            panic!("padding went unrepaired");
        };
        let mut repaired = text.clone();
        for &offset in offsets.iter().rev() {
            repaired.replace_range(offset..offset + '\u{0640}'.len_utf8(), "");
        }
        assert!(!repaired.contains('\u{0640}'));
        assert!(rule.check(&unit(&repaired)).is_empty());
    }
}
