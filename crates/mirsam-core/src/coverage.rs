//! Coverage: which characters the font answering for a run cannot draw at all.
//!
//! [`crate::shape`] asks what a font *did* with a letter it has. This module
//! asks the question underneath it — does the font have the letter? — and the
//! two are different defects with different repairs. A font with no shaping
//! tables renders Arabic as disconnected letters; a font with no Arabic
//! renders it as a row of empty boxes, and no shaping table would have saved
//! it. Telling an author to install a font is the wrong advice for the first
//! and the only advice for the second.
//!
//! ## What a font is judged over
//!
//! Only the characters it actually answers for. A complex-script slot draws
//! the Arabic in a mixed paragraph and nothing else: the Latin, the European
//! digits and the ASCII punctuation are drawn by the Latin slot, which is a
//! different font and a different question. Reporting Times New Roman for
//! having no Arabic makes sense; reporting an Arabic font for having no `Q`
//! would be reporting a font for the text it was never asked to draw, and is
//! ADR 0004's first failure mode arriving through a new door.
//!
//! So [`judges`] states the set: the Arabic script as a document is supposed
//! to store it, minus the format characters no font is meant to draw. Two
//! exclusions inside that are worth naming.
//!
//! *Format characters.* U+0600 ARABIC NUMBER SIGN prefixes a numeral, U+06DD
//! ARABIC END OF AYAH encloses a verse number, U+061C ARABIC LETTER MARK is a
//! bidi control. None of them is a shape. A font that has no glyph for one is
//! every font there is.
//!
//! *Presentation forms.* Pre-shaped text is already a blocking defect that
//! [`crate::script::is_presentation_form`] reports and a repair maps back to
//! logical order. Asking whether a font covers U+FB50 is asking about text
//! that should not be in the document, and answering it would put a second
//! finding — with a completely different repair — on a character that already
//! has one.
//!
//! ## No I/O, here as everywhere
//!
//! [`coverage`] takes a parsed [`Font`], which took bytes. Which typeface a
//! paragraph resolves to, and where that file lives on this machine, are
//! questions about the world: they belong to a [`crate::ports::FontSource`],
//! and `mirsam-fonts` is the adapter that answers them.

use std::fmt;

use crate::charname;
use crate::shape::Font;

/// Whether a complex-script font is answerable for `c`.
///
/// The Arabic script in logical order, minus the format characters that are
/// meaning rather than shape. See the module documentation for why the set is
/// drawn exactly here — every exclusion is a false positive that would
/// otherwise be reported against a font doing its job.
pub fn judges(c: char) -> bool {
    charname::name(c).is_some() && !charname::is_format(c)
}

/// One character the font has no glyph for, and how much of the text it costs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MissingChar {
    pub ch: char,
    pub codepoint: u32,
    /// The Unicode name, so a reviewer reads `ARABIC LETTER TTEH` rather than
    /// looking `U+0679` up. Always present: [`judges`] admits only characters
    /// [`charname::name`] can name.
    pub name: &'static str,
    /// Byte offset of the first occurrence in the text that was checked.
    pub first_offset: usize,
    /// How many times the character appears. One missing letter repeated
    /// forty times is a heavier defect than forty distinct ones appearing
    /// once, and a caller that only saw the distinct list could not tell.
    pub occurrences: usize,
}

impl fmt::Display for MissingChar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "U+{:04X} {}", self.codepoint, self.name)
    }
}

/// What a font can and cannot draw of one piece of text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Coverage {
    /// How many characters the font was answerable for — [`judges`] over the
    /// text, counting repeats.
    ///
    /// Zero means the text proves nothing about the font: it had no Arabic to
    /// answer for, and the best and worst fonts on the machine would come
    /// back identical.
    pub checked: usize,
    /// The distinct characters it has no glyph for, in the order they first
    /// appear.
    pub missing: Vec<MissingChar>,
}

impl Coverage {
    /// Whether the font can draw every character it was answerable for.
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    /// How many characters of the text will not render, counting repeats.
    ///
    /// The measure a severity turns on. `missing.len()` counts how many
    /// *letters of the alphabet* are absent; this counts how much of the
    /// document goes blank.
    pub fn missing_occurrences(&self) -> usize {
        self.missing.iter().map(|m| m.occurrences).sum()
    }

    /// Whether the font answers for none of the text it was handed.
    ///
    /// The Latin-only-font-under-Arabic case, and the one conclusion a
    /// coverage report supports on its own: a font that drew nothing was not
    /// the font this text needed. Anything short of it may be a single
    /// unusual letter in an otherwise correct pairing, which is a different
    /// finding at a different severity.
    ///
    /// False when there was nothing to answer for, because a font cannot fail
    /// a text that asked it for nothing.
    pub fn covers_nothing(&self) -> bool {
        self.checked > 0 && self.missing_occurrences() == self.checked
    }
}

/// Which of the characters `font` answers for in `text` it has no glyph for.
///
/// Distinct characters, in first-appearance order, each carrying its Unicode
/// name and how often it occurs. Characters the font was never answerable for
/// — Latin, digits, punctuation, format characters, pre-shaped forms — are not
/// counted and not reported; see the module documentation.
pub fn coverage(font: &Font, text: &str) -> Coverage {
    let mut checked = 0;
    let mut missing: Vec<MissingChar> = Vec::new();

    for (offset, ch) in text.char_indices() {
        if !judges(ch) {
            continue;
        }
        checked += 1;
        if font.covers(ch) {
            continue;
        }
        match missing.iter_mut().find(|m| m.ch == ch) {
            Some(seen) => seen.occurrences += 1,
            None => missing.push(MissingChar {
                ch,
                codepoint: ch as u32,
                name: charname::name(ch).expect("judges admits only named characters"),
                first_offset: offset,
                occurrences: 1,
            }),
        }
    }

    Coverage { checked, missing }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judges_the_arabic_script_and_nothing_else() {
        for c in [
            'ب', 'ء', '\u{0640}', '\u{064E}', '\u{0660}', '\u{06A9}', 'ݐ',
        ] {
            assert!(judges(c), "U+{:04X}", c as u32);
        }
        // Latin, European digits and ASCII punctuation belong to the other
        // font slot; the complex-script font is not answerable for them.
        for c in ['Q', '4', '%', ' ', '\n', '\u{4E00}'] {
            assert!(!judges(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn a_format_character_is_never_a_missing_glyph() {
        // Meaning, not shape. Every font on earth lacks these.
        for c in ['\u{0600}', '\u{0605}', '\u{061C}', '\u{06DD}', '\u{08E2}'] {
            assert!(charname::name(c).is_some(), "U+{:04X}", c as u32);
            assert!(!judges(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn a_presentation_form_is_a_different_defect() {
        // Reported by `presentation-forms` and mapped back to logical order by
        // its repair. A font's opinion of U+FB50 is not a question about a
        // document that stores its text correctly.
        for c in ['\u{FB50}', '\u{FE83}', '\u{FEF2}', '\u{FDFA}'] {
            assert!(!judges(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn nothing_to_answer_for_is_not_a_failure() {
        let empty = Coverage::default();
        assert!(empty.is_complete());
        assert!(!empty.covers_nothing());
    }

    #[test]
    fn a_missing_character_reads_as_its_unicode_name() {
        let missing = MissingChar {
            ch: '\u{0679}',
            codepoint: 0x0679,
            name: charname::name('\u{0679}').unwrap(),
            first_offset: 0,
            occurrences: 1,
        };
        assert_eq!(missing.to_string(), "U+0679 ARABIC LETTER TTEH");
    }
}
