//! Shaping against a real OpenType shaper and three real fonts.
//!
//! The fixtures are built by `scripts/make-shaping-fixture.py` and differ
//! from one another in exactly one thing each — same cmap, same glyph order,
//! same metrics throughout. `joining.ttf` has a `GSUB` carrying `init`,
//! `medi` and `fina`; `nonjoining.ttf` has no `GSUB` at all, which is the
//! defect M4 exists for; `partial.ttf` shapes everything except the final
//! forms of the right-joining letters, which is what Arial does and is not a
//! defect at all. A test that separates the three is proving the detection
//! rather than the shaper.
//!
//! The generator's glyph order is public and the assertions use it: standalone
//! forms run in codepoint order from glyph 2, and each contextual form sits a
//! fixed distance above its standalone one. A test can therefore name the
//! exact glyph it expects instead of asserting only that something changed.

use mirsam_core::joining::JoiningForm;
use mirsam_core::shape::{Font, Outcome, ShapedLetter, shape};

const JOINING: &[u8] = include_bytes!("fonts/joining.ttf");
const NONJOINING: &[u8] = include_bytes!("fonts/nonjoining.ttf");
const PARTIAL: &[u8] = include_bytes!("fonts/partial.ttf");

const FIRST_CP: u32 = 0x0621;
const LETTERS: u16 = 42;
const GID_STANDALONE: u16 = 2;

/// The glyph the fixture fonts map `c` to before anything shapes it.
fn standalone(c: char) -> u16 {
    GID_STANDALONE + (c as u32 - FIRST_CP) as u16
}

/// The glyph the fixture's `GSUB` substitutes for `form`.
fn contextual(c: char, form: JoiningForm) -> u16 {
    let step = match form {
        JoiningForm::Isolated => 0,
        JoiningForm::Initial => 1,
        JoiningForm::Medial => 2,
        JoiningForm::Final => 3,
    };
    standalone(c) + step * LETTERS
}

fn font(data: &[u8]) -> Font<'_> {
    Font::parse(data, 0).expect("the fixture is a font")
}

/// What a letter came back as, in the form the assertions below read in.
fn seen(letter: &ShapedLetter) -> (char, JoiningForm, Outcome, Vec<u16>) {
    (
        letter.ch,
        letter.required,
        letter.outcome,
        letter.glyphs.clone(),
    )
}

#[test]
fn a_font_that_shapes_produces_every_joining_form() {
    let shaping = shape(&font(JOINING), "سلام");

    assert_eq!(
        shaping.letters.iter().map(seen).collect::<Vec<_>>(),
        vec![
            (
                'س',
                JoiningForm::Initial,
                Outcome::Contextual,
                vec![contextual('س', JoiningForm::Initial)],
            ),
            (
                'ل',
                JoiningForm::Medial,
                Outcome::Contextual,
                vec![contextual('ل', JoiningForm::Medial)],
            ),
            (
                'ا',
                JoiningForm::Final,
                Outcome::Contextual,
                vec![contextual('ا', JoiningForm::Final)],
            ),
            // Nothing joins to an alef on its left, so the meem is standalone
            // and standalone is what the font is supposed to draw.
            (
                'م',
                JoiningForm::Isolated,
                Outcome::Standalone,
                vec![standalone('م')],
            ),
        ]
    );

    assert_eq!(shaping.joins_required(), 3);
    assert_eq!(shaping.joins_produced(), 3);
    assert_eq!(shaping.drawn_standalone().count(), 0);
}

#[test]
fn a_font_with_no_shaping_tables_leaves_every_letter_standalone() {
    let shaping = shape(&font(NONJOINING), "سلام");

    for letter in &shaping.letters {
        assert_eq!(
            (letter.outcome, letter.glyphs.as_slice()),
            (Outcome::Standalone, [standalone(letter.ch)].as_slice()),
            "{} came back shaped by a font with no GSUB",
            letter.ch
        );
    }

    assert_eq!(shaping.joins_required(), 3);
    assert_eq!(shaping.joins_produced(), 0);
    assert_eq!(
        shaping.drawn_standalone().map(|l| l.ch).collect::<Vec<_>>(),
        vec!['س', 'ل', 'ا']
    );
    // The isolated meem is not among them: it rendered exactly as required.
    assert!(!shaping.drawn_standalone().any(|l| l.ch == 'م'));
}

#[test]
fn the_two_fonts_disagree_about_nothing_except_shaping() {
    let text = "مدرسة سلام";
    let good = shape(&font(JOINING), text);
    let bad = shape(&font(NONJOINING), text);

    // Same letters, same requirements, same offsets: the text is the text.
    let required = |s: &mirsam_core::Shaping| {
        s.letters
            .iter()
            .map(|l| (l.offset, l.ch, l.required))
            .collect::<Vec<_>>()
    };
    assert_eq!(required(&good), required(&bad));
    assert_eq!(good.joins_required(), bad.joins_required());

    // And a flat disagreement about what the font did with them.
    assert_eq!(good.joins_produced(), good.joins_required());
    assert_eq!(bad.joins_produced(), 0);
}

#[test]
fn a_letter_the_font_has_no_glyph_for_is_coverage_not_shaping() {
    // U+06A9 KEHEH is a dual-joining letter the fixtures do not map — they
    // stop at U+064A. The keheh is a coverage problem; the beh beside it
    // still shapes, and calling either by the other's name would send a
    // reader to the wrong repair.
    let shaping = shape(&font(JOINING), "کب");

    assert_eq!(shaping.letters[0].ch, 'ک');
    assert_eq!(shaping.letters[0].required, JoiningForm::Initial);
    assert_eq!(shaping.letters[0].outcome, Outcome::Unmapped);
    assert!(!shaping.letters[0].drew_standalone());

    assert_eq!(shaping.letters[1].ch, 'ب');
    assert_eq!(shaping.letters[1].outcome, Outcome::Contextual);
    assert_eq!(
        shaping.letters[1].glyphs,
        vec![contextual('ب', JoiningForm::Final)]
    );
}

#[test]
fn a_mark_is_stepped_over_by_the_shaper_too() {
    // `joining` says the fatha does not break the join. This asserts the
    // shaper agrees: the beh must still come back initial with the mark
    // sitting between it and the teh.
    //
    // It also pins the reason `Outcome` asks whether the standalone glyph is
    // among the cluster's glyphs rather than whether it is the only one. The
    // shaper merges the fatha into the beh's cluster, so the beh's cluster
    // holds two glyphs — the initial beh and the notdef the fixture has for
    // an unmapped mark — and a check that demanded exactly one glyph would
    // report every vowelled Arabic word in existence.
    let shaping = shape(&font(JOINING), "بَت");

    assert_eq!(
        shaping
            .letters
            .iter()
            .map(|l| (l.ch, l.required, l.outcome))
            .collect::<Vec<_>>(),
        vec![
            ('ب', JoiningForm::Initial, Outcome::Contextual),
            ('ت', JoiningForm::Final, Outcome::Contextual),
        ]
    );
    assert!(
        shaping.letters[0]
            .glyphs
            .contains(&contextual('ب', JoiningForm::Initial))
    );
    assert_eq!(shaping.joins_produced(), 2);

    // And through the font that cannot shape, the same word fails both joins
    // rather than disappearing into the merged cluster.
    let bad = shape(&font(NONJOINING), "بَت");
    assert_eq!(bad.drawn_standalone().count(), 2);
}

#[test]
fn a_font_that_shapes_can_still_draw_a_letter_standalone() {
    // `partial.ttf` gives no final form to the right-joining letters, which
    // is what macOS's Arial does: shaping مرحبا through Arial leaves the reh
    // on its cmap glyph and the word renders perfectly, because the stroke
    // that joins to a reh is drawn by the letter before it.
    //
    // So this font is correct, and every letter of it that came back
    // standalone is a letter a per-letter verdict would have reported. This
    // test is the reason the module refuses to give one.
    let shaping = shape(&font(PARTIAL), "مرحبا");

    assert_eq!(
        shaping
            .drawn_standalone()
            .map(|l| (l.ch, l.required))
            .collect::<Vec<_>>(),
        vec![('ر', JoiningForm::Final), ('ا', JoiningForm::Final)]
    );

    // What separates it from the font that cannot shape at all is the only
    // thing that ever could: it produced joins, and that one produced none.
    assert_eq!(shaping.joins_required(), 5);
    assert_eq!(shaping.joins_produced(), 3);
    assert_eq!(shape(&font(NONJOINING), "مرحبا").joins_produced(), 0);
    assert_eq!(shape(&font(JOINING), "مرحبا").joins_produced(), 5);
}

#[test]
fn every_arabic_run_of_a_mixed_paragraph_is_shaped() {
    // The paragraph a real deck holds: Arabic, Latin, digits, more Arabic.
    // Each Arabic stretch is shaped as Arabic, and every offset points back
    // into the text that was passed in and not into the run it came from.
    let text = "ارتفع الأداء بنسبة 25% في Q4 2026.";
    let shaping = shape(&font(JOINING), text);

    assert!(shaping.joins_required() > 0);
    assert_eq!(shaping.joins_produced(), shaping.joins_required());
    for letter in &shaping.letters {
        assert_eq!(
            text[letter.offset..].chars().next(),
            Some(letter.ch),
            "offset {} does not point at {}",
            letter.offset,
            letter.ch
        );
    }

    // The same paragraph through the unshaping font: every join fails.
    let bad = shape(&font(NONJOINING), text);
    assert_eq!(bad.joins_produced(), 0);
    assert_eq!(bad.drawn_standalone().count(), bad.joins_required());
}

#[test]
fn text_with_no_join_to_make_proves_nothing_about_a_font() {
    // A lone letter, or a word of right-joining letters only, renders
    // identically through both fonts. A check that reported on this text
    // would be reporting on the text's shape, not the font's.
    for text in ["ب", "درر", "ء", "hello 2026"] {
        for data in [JOINING, NONJOINING] {
            let shaping = shape(&font(data), text);
            assert_eq!(shaping.joins_required(), 0, "{text}");
            assert_eq!(shaping.drawn_standalone().count(), 0, "{text}");
        }
    }
}

#[test]
fn a_font_the_shaper_cannot_read_is_refused_not_guessed_at() {
    assert!(Font::parse(b"not a font", 0).is_none());
    assert!(Font::parse(&[], 0).is_none());
}
