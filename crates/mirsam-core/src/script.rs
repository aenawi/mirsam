//! Arabic script detection and joining behaviour.
//!
//! Table-driven over Unicode blocks for the questions a block answers: the
//! blocks below are stable and "is this Arabic script, and does it join?" needs
//! nothing finer. The one question a block cannot answer — which logical
//! codepoints a pre-shaped presentation form stands for — is put to
//! `unicode-normalization`, one character at a time; see
//! `docs/adr/0005-presentation-forms-via-unicode-normalization.md`.

use std::iter::once;
use unicode_normalization::UnicodeNormalization;

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

/// The two blocks pre-shaped Arabic lives in: Presentation Forms-A and -B.
///
/// Membership alone proves nothing. Beside the contextual letter forms, the
/// blocks hold word ligatures, ornate parentheses, pedagogical symbols, sixty
/// unassigned codepoints and U+FEFF — a byte-order mark that leaked into a
/// run is not pre-shaped text, and no normalisation will change it.
fn in_presentation_blocks(c: char) -> bool {
    matches!(c as u32, 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}

/// The word ligatures U+FDF0..=U+FDFF: ﷺ, ﷼, ﷽ and their kin.
///
/// These are authored content, not shaping artefacts. An author who typed
/// U+FDFA meant one symbol, and expanding it to the eighteen codepoints of
/// its spelled-out phrase would rewrite what they wrote. They are reported
/// and left alone.
pub fn is_word_ligature(c: char) -> bool {
    matches!(c as u32, 0xFDF0..=0xFDFF)
}

/// The logical-order codepoints a pre-shaped contextual form stands for.
///
/// `None` for anything that is not such a form: a logical-order letter, a
/// word ligature, a codepoint in the blocks with no decomposition, or any
/// other character. So `logical_form(c).is_some()` is exactly the set of
/// characters [`normalize_presentation_forms`] will change, and the rule that
/// reports them and the repair that maps them cannot disagree.
///
/// The mapping is NFKC applied to the single character in isolation:
/// compatibility decomposition to the base letters, then canonical
/// composition, so U+FE83 comes back as U+0623 ALEF WITH HAMZA ABOVE rather
/// than as alef followed by a combining hamza. Nothing outside the character
/// takes part, which is what keeps a neighbouring `ﬁ` or a combining mark the
/// author placed exactly as they were.
pub fn logical_form(c: char) -> Option<String> {
    if !in_presentation_blocks(c) || is_word_ligature(c) {
        return None;
    }
    let mapped: String = once(c).nfkc().collect();
    (mapped.chars().ne(once(c))).then_some(mapped)
}

/// A pre-shaped contextual form of an Arabic letter, stored where a
/// logical-order codepoint belongs.
///
/// Its presence in a source document is a red flag: it means someone
/// pre-shaped the text instead of storing logical-order Unicode, which
/// defeats every downstream renderer, search and reflow. True for exactly
/// the characters [`logical_form`] maps.
pub fn is_presentation_form(c: char) -> bool {
    logical_form(c).is_some()
}

/// Replace every pre-shaped contextual form with the codepoints it stands
/// for, and leave every other character — word ligatures included — exactly
/// as it was.
pub fn normalize_presentation_forms(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match logical_form(c) {
            Some(mapped) => out.push_str(&mapped),
            None => out.push(c),
        }
    }
    out
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
    fn a_form_is_only_what_a_repair_can_map() {
        // Inside the blocks, but not pre-shaped text and not mappable: a
        // byte-order mark, the ornate parentheses, a pedagogical symbol, an
        // unassigned codepoint. Reporting these as presentation forms would
        // propose a repair that changes nothing.
        for c in ['\u{FEFF}', '\u{FD3E}', '\u{FD3F}', '\u{FBB2}', '\u{FBC3}'] {
            assert!(!is_presentation_form(c), "U+{:04X}", c as u32);
            assert_eq!(logical_form(c), None, "U+{:04X}", c as u32);
        }
        // Contextual forms and letter ligatures are.
        for c in ['\u{FEF2}', '\u{FEFB}', '\u{FE83}', '\u{FBEA}', '\u{FE71}'] {
            assert!(is_presentation_form(c), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn word_ligatures_are_reported_but_never_mapped() {
        for c in ['\u{FDF0}', '\u{FDFA}', '\u{FDFC}', '\u{FDFD}', '\u{FDFF}'] {
            assert!(is_word_ligature(c), "U+{:04X}", c as u32);
            assert!(!is_presentation_form(c), "U+{:04X}", c as u32);
            assert_eq!(logical_form(c), None, "U+{:04X}", c as u32);
        }
        assert!(!is_word_ligature('\u{FEF2}'));
        assert_eq!(normalize_presentation_forms("\u{FDFA}"), "\u{FDFA}");
    }

    #[test]
    fn maps_a_form_to_its_logical_letters() {
        assert_eq!(logical_form('\u{FEF2}').as_deref(), Some("\u{064A}"));
        // A letter ligature stands for two letters.
        assert_eq!(
            logical_form('\u{FEFB}').as_deref(),
            Some("\u{0644}\u{0627}")
        );
        // Tatweel carrying a mark.
        assert_eq!(
            logical_form('\u{FE71}').as_deref(),
            Some("\u{0640}\u{064B}")
        );
        assert_eq!(normalize_presentation_forms("ﻣﺮﺣﺒﺎ"), "مرحبا");
    }

    #[test]
    fn hamza_forms_come_back_precomposed() {
        // Compatibility decomposition alone yields alef + combining hamza,
        // which is canonically equivalent but not what any keyboard or
        // application stores. The mapping composes, so the letter comes back
        // as the single codepoint the author would have typed.
        assert_eq!(logical_form('\u{FE83}').as_deref(), Some("\u{0623}"));
        assert_eq!(
            logical_form('\u{FEF7}').as_deref(),
            Some("\u{0644}\u{0623}")
        );
        assert_eq!(
            logical_form('\u{FBEA}').as_deref(),
            Some("\u{0626}\u{0627}")
        );
    }

    #[test]
    fn normalisation_touches_nothing_it_was_not_sent_for() {
        // alef + madda above as two codepoints, a Latin ligature, a
        // superscript, a word ligature and a byte-order mark: whole-string
        // NFKC would change every one of them. Only the form changes.
        let text = "\u{0627}\u{0653} \u{FB01} \u{00B2} \u{FDFA} \u{FEFF} \u{FEF2}";
        assert_eq!(
            normalize_presentation_forms(text),
            "\u{0627}\u{0653} \u{FB01} \u{00B2} \u{FDFA} \u{FEFF} \u{064A}"
        );
        assert_eq!(normalize_presentation_forms("مرحبا hello"), "مرحبا hello");
    }

    #[test]
    fn normalisation_is_idempotent() {
        let once = normalize_presentation_forms("ﺍﻟﺘﻘﺮﻳﺮ ﺍﻟﻔﺼﻠﻲ");
        assert_eq!(normalize_presentation_forms(&once), once);
        assert!(!once.chars().any(is_presentation_form));
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
