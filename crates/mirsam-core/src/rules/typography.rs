//! Rules about language metadata, fonts and list formatting.

use super::Rule;
use crate::diagnostic::{Diagnostic, RuleId, Severity};
use crate::fix::Fix;
use crate::script;
use crate::text::{Bullet, TextUnit};

/// Default locale used when repairing missing language metadata.
pub const DEFAULT_LOCALE: &str = "ar-SA";

/// Whether a BCP-47 tag names Arabic: `ar`, `ar-SA`, `ar-Arab-AE`, …
///
/// The primary subtag alone decides. `arn` (Mapudungun) is not Arabic, and a
/// tag such as `arabic` is not a tag at all.
pub fn is_arabic_tag(tag: &str) -> bool {
    matches!(
        tag.as_bytes(),
        [b'a' | b'A', b'r' | b'R'] | [b'a' | b'A', b'r' | b'R', b'-', ..]
    )
}

/// Arabic text without an Arabic BCP-47 language tag.
///
/// Missing language metadata degrades spell-check, hyphenation, screen-reader
/// pronunciation and font fallback — invisible in a screenshot, real for users.
pub struct LanguageMissing {
    /// The tag a repair writes. Which Arabic locale the author meant is not
    /// something the text can prove, so it is a preference, not a finding.
    pub locale: String,
}

impl Default for LanguageMissing {
    fn default() -> Self {
        Self {
            locale: DEFAULT_LOCALE.to_string(),
        }
    }
}

impl Rule for LanguageMissing {
    fn id(&self) -> RuleId {
        RuleId("language-missing")
    }

    fn description(&self) -> &'static str {
        "Arabic text carries no Arabic language tag"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        let tagged = unit
            .props
            .language
            .effective()
            .is_some_and(|tag| is_arabic_tag(tag));
        if tagged {
            return Vec::new();
        }

        let found = unit
            .props
            .language
            .effective()
            .map_or("none", String::as_str);
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                format!("Arabic text tagged as {found}; expected an ar-* language tag"),
            )
            .fixable(),
        ]
    }

    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        Some(Fix::SetLanguage(self.locale.clone()))
    }
}

/// A Latin font is chosen but the complex-script slot is empty.
///
/// The Arabic then renders in whatever the application substitutes, which is
/// the most common cause of "it looked fine on my machine".
#[derive(Default)]
pub struct ComplexFontMissing {
    /// The typeface a repair writes into the empty slot. `None` — the default
    /// — reports the finding without proposing a fix: choosing a typeface is
    /// an authoring decision, and `repair --font` is where it is made.
    pub typeface: Option<String>,
}

impl Rule for ComplexFontMissing {
    fn id(&self) -> RuleId {
        RuleId("complex-font-missing")
    }

    fn description(&self) -> &'static str {
        "A Latin font is specified but the Arabic complex-script font slot is empty"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if !script::has_arabic(&unit.text) {
            return Vec::new();
        }
        if unit.props.latin_font.effective().is_none() {
            return Vec::new();
        }
        if unit.props.complex_font.effective().is_some() {
            return Vec::new();
        }

        let diagnostic = Diagnostic::new(
            self.id(),
            Severity::Warning,
            &unit.id,
            &unit.location,
            "Latin font specified without a complex-script font; Arabic will be substituted",
        );
        // Fixable only once a typeface has been chosen. Without one, no
        // mechanical repair exists, and saying otherwise would be a lie.
        vec![if self.typeface.is_some() {
            diagnostic.fixable()
        } else {
            diagnostic
        }]
    }

    fn fix(&self, _unit: &TextUnit) -> Option<Fix> {
        self.typeface.clone().map(Fix::SetComplexFont)
    }
}

/// A typed bullet glyph standing in for a real list.
#[derive(Default)]
pub struct LiteralBullet {
    /// Whether a repair replaces the glyph with a native list. Off by default:
    /// unlike every other fix this one edits the text itself, not only the
    /// properties around it, so the caller opts in.
    pub convert: bool,
}

/// Marker glyphs commonly typed in place of a native list.
const MARKERS: [char; 5] = ['\u{2022}', '\u{25E6}', '\u{25AA}', '\u{2023}', '\u{2043}'];

fn leading_marker(text: &str) -> Option<char> {
    let trimmed = text.trim_start();
    let marker = trimmed.chars().next()?;
    if !MARKERS.contains(&marker) {
        return None;
    }
    // Require whitespace after the glyph, so a bullet used as real content
    // (a legend key, say) is not mistaken for list formatting.
    trimmed[marker.len_utf8()..]
        .starts_with(char::is_whitespace)
        .then_some(marker)
}

impl Rule for LiteralBullet {
    fn id(&self) -> RuleId {
        RuleId("literal-bullet")
    }

    fn description(&self) -> &'static str {
        "A typed bullet glyph is used instead of the format's native list feature"
    }

    fn check(&self, unit: &TextUnit) -> Vec<Diagnostic> {
        if unit.props.bullet == Bullet::Native {
            return Vec::new();
        }
        let Some(marker) = leading_marker(&unit.text) else {
            return Vec::new();
        };

        // A mechanical repair exists whether or not this run opts into it,
        // which is what `fixable` reports.
        vec![
            Diagnostic::new(
                self.id(),
                Severity::Warning,
                &unit.id,
                &unit.location,
                format!("paragraph begins with a typed {marker:?}; use a native list instead"),
            )
            .fixable(),
        ]
    }

    fn fix(&self, unit: &TextUnit) -> Option<Fix> {
        if !self.convert {
            return None;
        }
        leading_marker(&unit.text).map(|marker| Fix::ConvertLiteralBullet { marker })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Properties, Resolved};

    #[test]
    fn marker_requires_trailing_space() {
        assert_eq!(leading_marker("• بند أول"), Some('\u{2022}'));
        assert_eq!(leading_marker("  • بند"), Some('\u{2022}'));
        // A lone glyph used as content, not as list formatting.
        assert_eq!(leading_marker("•"), None);
        assert_eq!(leading_marker("بند أول"), None);
    }

    #[test]
    fn arabic_tags_are_recognised_by_their_primary_subtag() {
        for tag in ["ar", "AR", "ar-SA", "ar-AE", "ar-Arab-EG", "Ar-sa"] {
            assert!(is_arabic_tag(tag), "{tag} should count as Arabic");
        }
        for tag in ["", "a", "en-US", "arn", "arabic", "fa-IR", "-ar", "ar_SA"] {
            assert!(!is_arabic_tag(tag), "{tag} should not count as Arabic");
        }
    }

    #[test]
    fn a_bullet_is_converted_only_on_request() {
        let unit = TextUnit::new("u1", "• بند أول");
        assert_eq!(LiteralBullet::default().fix(&unit), None);
        assert_eq!(
            LiteralBullet { convert: true }.fix(&unit),
            Some(Fix::ConvertLiteralBullet { marker: '•' })
        );
        // Reported, and reported as fixable, either way.
        assert!(LiteralBullet::default().check(&unit)[0].fixable);
    }

    #[test]
    fn a_complex_font_is_proposed_only_once_chosen() {
        let unit = TextUnit::new("u1", "مرحبا").with_props(Properties {
            latin_font: Resolved::Explicit("Calibri".into()),
            ..Default::default()
        });
        let unchosen = ComplexFontMissing::default();
        assert_eq!(unchosen.fix(&unit), None);
        assert!(!unchosen.check(&unit)[0].fixable);

        let chosen = ComplexFontMissing {
            typeface: Some("Dubai".into()),
        };
        assert_eq!(chosen.fix(&unit), Some(Fix::SetComplexFont("Dubai".into())));
        assert!(chosen.check(&unit)[0].fixable);
    }

    #[test]
    fn the_language_written_is_the_one_configured() {
        let unit = TextUnit::new("u1", "مرحبا");
        assert_eq!(
            LanguageMissing::default().fix(&unit),
            Some(Fix::SetLanguage("ar-SA".into()))
        );
        let rule = LanguageMissing {
            locale: "ar-AE".into(),
        };
        assert_eq!(rule.fix(&unit), Some(Fix::SetLanguage("ar-AE".into())));
        // And the configured tag is one the check accepts, so the repair sticks.
        let repaired = unit.with_props(Properties {
            language: Resolved::Explicit("ar-AE".into()),
            ..Default::default()
        });
        assert!(rule.check(&repaired).is_empty());
    }
}
