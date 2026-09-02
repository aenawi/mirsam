//! Rules about language metadata, fonts and list formatting.

use super::Rule;
use crate::diagnostic::{Diagnostic, RuleId, Severity};
use crate::fix::Fix;
use crate::script;
use crate::text::{Bullet, TextUnit};

/// Default locale used when repairing missing language metadata.
pub const DEFAULT_LOCALE: &str = "ar-SA";

/// Arabic text without an Arabic BCP-47 language tag.
///
/// Missing language metadata degrades spell-check, hyphenation, screen-reader
/// pronunciation and font fallback — invisible in a screenshot, real for users.
pub struct LanguageMissing;

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
            .is_some_and(|tag| tag.to_ascii_lowercase().starts_with("ar"));
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
        Some(Fix::SetLanguage(DEFAULT_LOCALE.to_string()))
    }
}

/// A Latin font is chosen but the complex-script slot is empty.
///
/// The Arabic then renders in whatever the application substitutes, which is
/// the most common cause of "it looked fine on my machine".
pub struct ComplexFontMissing;

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

        vec![Diagnostic::new(
            self.id(),
            Severity::Warning,
            &unit.id,
            &unit.location,
            "Latin font specified without a complex-script font; Arabic will be substituted",
        )]
        // Intentionally not fixable: choosing a typeface is an authoring
        // decision. `repair --font` supplies one explicitly.
    }
}

/// A typed bullet glyph standing in for a real list.
pub struct LiteralBullet;

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
        leading_marker(&unit.text).map(|marker| Fix::ConvertLiteralBullet { marker })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_requires_trailing_space() {
        assert_eq!(leading_marker("• بند أول"), Some('\u{2022}'));
        assert_eq!(leading_marker("  • بند"), Some('\u{2022}'));
        // A lone glyph used as content, not as list formatting.
        assert_eq!(leading_marker("•"), None);
        assert_eq!(leading_marker("بند أول"), None);
    }
}
