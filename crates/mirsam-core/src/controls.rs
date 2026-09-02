//! Unicode bidi control characters.
//!
//! These are the characters that "fix" Arabic in one preview and break it
//! everywhere else. The engine reports them and can strip them, but never
//! inserts them: direction belongs to the container, not to the string.

/// A bidi control character found in a text unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ControlHit {
    /// Byte offset within the unit's text.
    pub offset: usize,
    pub codepoint: u32,
    pub name: &'static str,
}

/// Returns the Unicode name if `c` is an explicit bidi formatting control.
///
/// ZWJ (U+200D) and ZWNJ (U+200C) are deliberately excluded: they are
/// linguistically meaningful in Arabic and Persian orthography and must never
/// be stripped as a direction fix.
pub fn control_name(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{061C}' => "ARABIC LETTER MARK",
        '\u{200E}' => "LEFT-TO-RIGHT MARK",
        '\u{200F}' => "RIGHT-TO-LEFT MARK",
        '\u{202A}' => "LEFT-TO-RIGHT EMBEDDING",
        '\u{202B}' => "RIGHT-TO-LEFT EMBEDDING",
        '\u{202C}' => "POP DIRECTIONAL FORMATTING",
        '\u{202D}' => "LEFT-TO-RIGHT OVERRIDE",
        '\u{202E}' => "RIGHT-TO-LEFT OVERRIDE",
        '\u{2066}' => "LEFT-TO-RIGHT ISOLATE",
        '\u{2067}' => "RIGHT-TO-LEFT ISOLATE",
        '\u{2068}' => "FIRST STRONG ISOLATE",
        '\u{2069}' => "POP DIRECTIONAL ISOLATE",
        _ => return None,
    })
}

/// Locate every explicit bidi control in `text`.
pub fn scan(text: &str) -> Vec<ControlHit> {
    text.char_indices()
        .filter_map(|(offset, c)| {
            control_name(c).map(|name| ControlHit {
                offset,
                codepoint: c as u32,
                name,
            })
        })
        .collect()
}

/// Remove every explicit bidi control, preserving ZWJ/ZWNJ.
pub fn strip(text: &str) -> String {
    text.chars()
        .filter(|c| control_name(*c).is_none())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rlm() {
        let hits = scan("بند أول\u{200F}");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "RIGHT-TO-LEFT MARK");
    }

    #[test]
    fn preserves_zwj_and_zwnj() {
        // ZWNJ is orthographically meaningful in Persian; stripping it corrupts text.
        let text = "می\u{200C}رود";
        assert!(scan(text).is_empty());
        assert_eq!(strip(text), text);
    }

    #[test]
    fn strips_only_controls() {
        assert_eq!(strip("\u{202B}مرحبا\u{202C}"), "مرحبا");
    }
}
