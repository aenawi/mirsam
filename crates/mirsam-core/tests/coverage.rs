//! Coverage against the three hand-built fonts.
//!
//! The fixtures map U+0621..U+064A and nothing else — the Arabic letters, and
//! not the harakat above them, not the Arabic-Indic digits, not a single
//! letter of Persian or Urdu. That is a narrower font than anything shipped,
//! which is what makes it useful here: every boundary the check draws shows up
//! as a character the fixture has or has not got.
//!
//! They differ from one another only in shaping, so the three must agree about
//! coverage exactly. A test that separated them would have found a coverage
//! check reading a shaping table.

use mirsam_core::coverage::coverage;
use mirsam_core::shape::{Font, shape};

const JOINING: &[u8] = include_bytes!("fonts/joining.ttf");
const NONJOINING: &[u8] = include_bytes!("fonts/nonjoining.ttf");
const PARTIAL: &[u8] = include_bytes!("fonts/partial.ttf");

fn font(data: &[u8]) -> Font<'_> {
    Font::parse(data, 0).expect("the fixture is a font")
}

/// What the report says, in the form the assertions below read in.
fn missing(font: &Font, text: &str) -> Vec<String> {
    coverage(font, text)
        .missing
        .iter()
        .map(|m| m.to_string())
        .collect()
}

#[test]
fn a_font_that_has_the_letters_is_complete() {
    let report = coverage(&font(JOINING), "مرحبا");
    assert!(report.is_complete());
    assert!(!report.covers_nothing());
    assert_eq!(report.checked, 5);
    assert_eq!(report.missing_occurrences(), 0);
}

#[test]
fn a_missing_letter_is_reported_by_name() {
    // Persian peh and gaf, Urdu tteh: Arabic script, and outside the range
    // the fixtures map.
    assert_eq!(
        missing(&font(JOINING), "پگٹ"),
        [
            "U+067E ARABIC LETTER PEH",
            "U+06AF ARABIC LETTER GAF",
            "U+0679 ARABIC LETTER TTEH",
        ]
    );
}

#[test]
fn a_font_that_answers_for_none_of_the_text_says_so() {
    // The shape of the defect this exists for: a font handed text it has no
    // letter of. Every character will render as an empty box, and no shaping
    // table would have changed that.
    let report = coverage(&font(JOINING), "ٱپچگ");
    assert_eq!(report.checked, 4);
    assert_eq!(report.missing_occurrences(), 4);
    assert!(report.covers_nothing());
}

#[test]
fn one_missing_letter_in_a_covered_word_is_not_a_font_that_covers_nothing() {
    let report = coverage(&font(JOINING), "مرحباپ");
    assert_eq!((report.checked, report.missing_occurrences()), (6, 1));
    assert!(!report.covers_nothing());
    assert!(!report.is_complete());
}

#[test]
fn a_repeated_character_is_named_once_and_counted_every_time() {
    let report = coverage(&font(JOINING), "پ مرحبا پ پ");
    assert_eq!(report.missing.len(), 1);

    let peh = &report.missing[0];
    assert_eq!(peh.ch, 'پ');
    assert_eq!(peh.occurrences, 3);
    assert_eq!(peh.first_offset, 0);
    assert_eq!(report.missing_occurrences(), 3);
}

#[test]
fn the_font_is_not_judged_for_text_it_never_draws() {
    // The Latin slot draws the Latin, the European digits and the ASCII
    // punctuation. Reporting an Arabic font for having no `Q` would be
    // reporting a font for text it was never asked for.
    let report = coverage(&font(JOINING), "Q4 2026 — 100%");
    assert_eq!(report.checked, 0);
    assert!(report.is_complete());
    assert!(!report.covers_nothing());

    // And in a mixed paragraph, only the Arabic is counted.
    let mixed = coverage(&font(JOINING), "ارتفع الأداء بنسبة 25% في Q4 2026.");
    assert!(mixed.is_complete());
    assert_eq!(mixed.checked, "ارتفعالأداءبنسبةفي".chars().count());
}

#[test]
fn a_format_character_is_not_a_missing_glyph() {
    // U+0600 ARABIC NUMBER SIGN, U+06DD ARABIC END OF AYAH, U+061C ARABIC
    // LETTER MARK: meaning, not shape. No font draws them, and the fixture
    // has no glyph for any of them.
    let report = coverage(&font(JOINING), "\u{0600}\u{06DD}\u{061C}");
    assert_eq!(report.checked, 0);
    assert!(report.is_complete());
}

#[test]
fn a_presentation_form_is_left_to_the_rule_that_owns_it() {
    // Pre-shaped text is already a blocking defect with a repair that maps it
    // back to logical order. A second finding here, with a different repair,
    // would be the tool arguing with itself about one character.
    let report = coverage(&font(JOINING), "ﻣﺮﺣﺒﺎ");
    assert_eq!(report.checked, 0);
    assert!(report.is_complete());
}

#[test]
fn a_font_with_no_harakat_drops_them_and_the_report_says_which() {
    // Vowelled Arabic is ordinary Arabic, and a mark is a glyph like any
    // other. The fixtures stop at U+064A, so every harakat above these
    // letters is genuinely absent from the font.
    assert_eq!(
        missing(&font(JOINING), "مَرْحَبًا"),
        [
            "U+064E ARABIC FATHA",
            "U+0652 ARABIC SUKUN",
            "U+064B ARABIC FATHATAN",
        ]
    );
}

#[test]
fn the_three_fonts_disagree_about_nothing_except_shaping() {
    // Same cmap, different GSUB. Coverage must not be able to tell them
    // apart, and shaping must.
    let text = "مرحبا پ";
    let reports: Vec<_> = [JOINING, NONJOINING, PARTIAL]
        .iter()
        .map(|data| coverage(&font(data), text))
        .collect();
    assert_eq!(reports[0], reports[1]);
    assert_eq!(reports[1], reports[2]);

    let joins: Vec<usize> = [JOINING, NONJOINING, PARTIAL]
        .iter()
        .map(|data| shape(&font(data), text).joins_produced())
        .collect();
    assert_eq!(joins, vec![5, 0, 3]);
}
