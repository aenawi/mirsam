//! Arabic script detection and joining behaviour.
//!
//! Deliberately table-driven over Unicode blocks rather than calling out to a
//! full character-database crate: the blocks below are stable and the whole
//! question the engine asks is "is this Arabic script, and does it join?".

/// Whether `c` is a letter of the Arabic script (excluding presentation forms).
pub fn is_arabic_letter(c: char) -> bool {
    matches!(c as u32,
        0x0620..=0x064A   // Arabic letters
        | 0x066E..=0x06D3 // extended letters
        | 0x06D5
        | 0x06E5..=0x06E6
        | 0x06EE..=0x06EF
        | 0x06FA..=0x06FC
        | 0x06FF
        | 0x0750..=0x077F // Arabic Supplement
        | 0x08A0..=0x08BD // Arabic Extended-A
    )
}

/// Arabic Presentation Forms A and B.
///
/// Their presence in a source document is a red flag: it means someone
/// pre-shaped the text instead of storing logical-order Unicode, which
/// defeats every downstream renderer.
pub fn is_presentation_form(c: char) -> bool {
    matches!(c as u32, 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

/// The Arabic-Indic and Eastern Arabic-Indic digit ranges.
pub fn is_arabic_indic_digit(c: char) -> bool {
    matches!(c as u32, 0x0660..=0x0669 | 0x06F0..=0x06F9)
}

/// Arabic combining marks (harakat, shadda, sukun, …).
pub fn is_arabic_mark(c: char) -> bool {
    matches!(c as u32, 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4)
}

/// U+0640 ARABIC TATWEEL, the kashida elongation character.
pub const TATWEEL: char = '\u{0640}';

/// Does this text contain any Arabic-script letter?
pub fn has_arabic(text: &str) -> bool {
    text.chars().any(is_arabic_letter)
}

/// Does this text contain strong-LTR or European-numeral content?
///
/// Used to decide whether a unit is bidirectionally mixed and therefore
/// worth resolving through the full UAX#9 algorithm.
pub fn has_ltr_or_digits(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_ascii_alphanumeric() || matches!(c as u32, 0x00C0..=0x024F))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_arabic_letters() {
        assert!(has_arabic("مرحبا"));
        assert!(has_arabic("hello مرحبا"));
        assert!(!has_arabic("hello world"));
        assert!(!has_arabic("12345"));
    }

    #[test]
    fn flags_presentation_forms() {
        // U+FEF2 ARABIC LETTER YEH FINAL FORM: a pre-shaped glyph.
        assert!(is_presentation_form('\u{FEF2}'));
        // U+064A ARABIC LETTER YEH: the correct logical-order codepoint.
        assert!(!is_presentation_form('\u{064A}'));
        assert!(is_arabic_letter('\u{064A}'));
    }

    #[test]
    fn separates_digits_from_letters() {
        assert!(is_arabic_indic_digit('\u{0660}'));
        assert!(!is_arabic_letter('\u{0660}'));
    }

    #[test]
    fn detects_mixed_content() {
        assert!(has_ltr_or_digits("النظام GPS"));
        assert!(has_ltr_or_digits("نسبة 25"));
        assert!(!has_ltr_or_digits("مرحبا"));
    }
}
