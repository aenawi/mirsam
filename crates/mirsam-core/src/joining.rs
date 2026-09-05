//! Which contextual form each Arabic letter is required to take.
//!
//! This is the expectation half of shaping, and it is pure Unicode: given the
//! logical-order text alone, and no font whatsoever, it says that the seen of
//! `سلام` must be drawn in its initial form and the lam in its medial one.
//! [`crate::shape`] then asks a font whether it actually did that, and the
//! two answers together are what a shaping defect is.
//!
//! Keeping the expectation here rather than in the shaping module is what
//! makes the claim checkable: every rule in this file can be tested against
//! the Unicode standard with no font in the room, and a disagreement with a
//! shaper is then a real disagreement rather than a tautology.
//!
//! The table states the Arabic block, U+0600..U+06FF, and nothing else.
//! Characters outside it are non-joining, which is exact for the Latin text,
//! digits, punctuation and spaces an Arabic run actually neighbours — and
//! wrong for a letter of Arabic Supplement or Extended-A, which do join. Those
//! blocks are [`JoiningType::Unstated`] rather than quietly assumed, and
//! [`forms`] states no form for an unstated letter or for either of its
//! neighbours. A letter this module cannot classify is one nothing downstream
//! will claim anything about.

use crate::script;

/// The Joining_Type of a character: which sides of it a neighbour may link to.
///
/// The Unicode property, restricted to the values the Arabic block uses, plus
/// [`Unstated`](JoiningType::Unstated) for the Arabic-script codepoints this
/// table deliberately does not answer for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoiningType {
    /// `D` — links on both sides. Most Arabic letters: beh, seen, lam, meem.
    Dual,
    /// `R` — links only to the letter before it. Alef, dal, reh, waw.
    Right,
    /// `L` — links only to the letter after it. No Arabic letter is `L`; the
    /// value exists in other scripts and is carried so the rule below reads
    /// as the standard states it.
    Left,
    /// `C` — joins its neighbours to each other without taking a form of its
    /// own: U+0640 TATWEEL and U+200D ZERO WIDTH JOINER.
    Causing,
    /// `T` — invisible to joining altogether. Harakat, shadda, superscript
    /// alef, U+061C ARABIC LETTER MARK: a letter joins straight through them.
    Transparent,
    /// `U` — breaks a join on both sides. Hamza, digits, punctuation, and
    /// every character outside the Arabic script.
    NonJoining,
    /// Arabic-script codepoints outside the block this table states.
    ///
    /// Not a Unicode value. It is the difference between "this letter does
    /// not join" and "this table does not say", and collapsing the two would
    /// make [`forms`] claim an isolated form for text it has never read.
    Unstated,
}

/// The contextual form a letter takes from its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum JoiningForm {
    /// Joined on neither side.
    Isolated,
    /// Joined to the letter that follows it.
    Initial,
    /// Joined on both sides.
    Medial,
    /// Joined to the letter that precedes it.
    Final,
}

impl JoiningForm {
    /// Whether this form requires the font to draw something other than the
    /// letter's standalone shape.
    ///
    /// An isolated letter is what an unshaped font already produces, so it is
    /// the one form that proves nothing about the font either way.
    pub fn is_joined(self) -> bool {
        self != Self::Isolated
    }

    /// The OpenType feature tag a shaper applies to reach this form.
    ///
    /// Present so a diagnostic can name the feature a font is missing —
    /// `init` — rather than only the letter it failed to shape.
    pub fn feature(self) -> &'static str {
        match self {
            Self::Isolated => "isol",
            Self::Initial => "init",
            Self::Medial => "medi",
            Self::Final => "fina",
        }
    }
}

/// One letter of a run, and the form its neighbours require of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Letter {
    /// Byte offset of the character within the text passed to [`forms`].
    pub offset: usize,
    pub ch: char,
    pub form: JoiningForm,
}

/// The Joining_Type of `c`.
///
/// Exact for U+0600..U+06FF and for the two zero-width characters that take
/// part in joining. [`JoiningType::Unstated`] for Arabic-script codepoints
/// elsewhere — the supplements, the extended blocks, and the presentation
/// forms, which are pre-shaped text and the business of `presentation-forms`
/// rather than of any font. Everything else is non-joining.
pub fn joining_type(c: char) -> JoiningType {
    use JoiningType::*;

    match c as u32 {
        0x200D => Causing,    // ZERO WIDTH JOINER
        0x200C => NonJoining, // ZERO WIDTH NON-JOINER

        // --- the Arabic block, as ArabicShaping.txt states it ---
        0x061C => Transparent,          // ARABIC LETTER MARK
        0x0610..=0x061A => Transparent, // honorifics above and below
        0x0620 => Dual,
        0x0621 => NonJoining, // HAMZA
        0x0622..=0x0625 => Right,
        0x0626 => Dual,
        0x0627 => Right, // ALEF
        0x0628 => Dual,
        0x0629 => Right, // TEH MARBUTA
        0x062A..=0x062E => Dual,
        0x062F..=0x0632 => Right, // DAL, THAL, REH, ZAIN
        0x0633..=0x063F => Dual,
        0x0640 => Causing, // TATWEEL
        0x0641..=0x0647 => Dual,
        0x0648 => Right, // WAW
        0x0649..=0x064A => Dual,
        0x064B..=0x065F => Transparent, // harakat, shadda, sukun
        0x0670 => Transparent,          // superscript alef
        0x0671..=0x0673 => Right,
        0x0674 => NonJoining, // HIGH HAMZA
        0x0675..=0x0677 => Right,
        0x0678..=0x0687 => Dual,
        0x0688..=0x0699 => Right,
        0x069A..=0x06BF => Dual,
        0x06C0 => Right,
        0x06C1..=0x06C2 => Dual,
        0x06C3..=0x06CB => Right,
        0x06CC => Dual, // FARSI YEH
        0x06CD => Right,
        0x06CE => Dual,
        0x06CF => Right,
        0x06D0..=0x06D1 => Dual,
        0x06D2..=0x06D3 => Right, // YEH BARREE
        0x06D5 => Right,          // AE
        0x06D6..=0x06DC => Transparent,
        0x06DF..=0x06E4 => Transparent,
        0x06E7..=0x06E8 => Transparent,
        0x06EA..=0x06ED => Transparent,
        0x06EE..=0x06EF => Right,
        0x06FA..=0x06FC => Dual,
        0x06FF => Dual,
        // Everything else in the block — the number signs, the digits, the
        // punctuation, U+06DD END OF AYAH, the small high letters — breaks a
        // join on both sides.
        0x0600..=0x06FF => NonJoining,

        // --- Arabic script this table does not state ---
        0x0700..=0x074F        // Syriac
        | 0x0750..=0x077F      // Arabic Supplement
        | 0x0860..=0x08FF      // Syriac Supplement, Arabic Extended-A and -B
        | 0xFB50..=0xFDFF      // Presentation Forms-A
        | 0xFE70..=0xFEFF      // Presentation Forms-B
        | 0x10D00..=0x10D3F    // Hanifi Rohingya
        | 0x1EE00..=0x1EEFF    // Arabic Mathematical Alphabetic Symbols
        => Unstated,

        _ => NonJoining,
    }
}

/// Every Arabic letter in `text`, with the contextual form its neighbours
/// require of it.
///
/// Transparent characters are stepped over, exactly as the standard says: a
/// letter joins through a fatha to the letter beyond it. They take no form of
/// their own and so appear in no result.
///
/// A letter whose classification, or whose neighbour's classification, this
/// module does not state is skipped rather than guessed at — see the module
/// documentation. Nothing is reported about it, which is the honest answer
/// and not a silent [`JoiningForm::Isolated`].
pub fn forms(text: &str) -> Vec<Letter> {
    // The joining sequence: every character that is not transparent, in
    // order, because those are the only ones a neighbour can see.
    let visible: Vec<(usize, char, JoiningType)> = text
        .char_indices()
        .map(|(offset, ch)| (offset, ch, joining_type(ch)))
        .filter(|(_, _, kind)| *kind != JoiningType::Transparent)
        .collect();

    let mut letters = Vec::new();
    for (index, &(offset, ch, kind)) in visible.iter().enumerate() {
        // Only a letter that can take a join has a form to take. Tatweel
        // joins its neighbours without being drawn as anything, and hamza
        // has one shape whatever surrounds it; both are Arabic letters by
        // `script::is_arabic_letter` and neither is shaped.
        let takes_a_form = matches!(
            kind,
            JoiningType::Dual | JoiningType::Right | JoiningType::Left
        );
        if !takes_a_form || !script::is_arabic_letter(ch) {
            continue;
        }
        let before = index.checked_sub(1).map(|i| visible[i].2);
        let after = visible.get(index + 1).map(|entry| entry.2);

        // The letter itself is stated — an unstated one takes no form above.
        // A neighbour that is not decides this letter's form, so an unstated
        // one means there is no form to state here either.
        if before == Some(JoiningType::Unstated) || after == Some(JoiningType::Unstated) {
            continue;
        }

        // A letter links to the character before it when it can take a join
        // on that side and that character can give one, and symmetrically
        // for the character after it. This is the whole of the rule.
        let links_before = matches!(kind, JoiningType::Dual | JoiningType::Right)
            && matches!(
                before,
                Some(JoiningType::Dual | JoiningType::Left | JoiningType::Causing)
            );
        let links_after = matches!(kind, JoiningType::Dual | JoiningType::Left)
            && matches!(
                after,
                Some(JoiningType::Dual | JoiningType::Right | JoiningType::Causing)
            );

        let form = match (links_before, links_after) {
            (true, true) => JoiningForm::Medial,
            (true, false) => JoiningForm::Final,
            (false, true) => JoiningForm::Initial,
            (false, false) => JoiningForm::Isolated,
        };
        letters.push(Letter { offset, ch, form });
    }
    letters
}

/// The maximal stretches of `text` a shaper should be handed as one Arabic
/// run, as `(byte offset, slice)` pairs.
///
/// A run is the characters joining can pass through — letters, the marks
/// between them, tatweel, a zero-width joiner. A space, a Latin word or a
/// digit ends one, and ending a run there changes nothing, because none of
/// them can carry a join across anyway.
///
/// Shaping the whole of a mixed paragraph in one pass would mean telling the
/// shaper one script for text that has two. Segmenting first is what lets
/// every run be shaped as the Arabic it is.
pub fn arabic_runs(text: &str) -> Vec<(usize, &str)> {
    let joins = |c: char| {
        matches!(
            joining_type(c),
            JoiningType::Dual | JoiningType::Right | JoiningType::Left | JoiningType::Causing
        ) || (joining_type(c) == JoiningType::Transparent && script::is_arabic_mark(c))
    };

    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for (offset, ch) in text.char_indices() {
        match (joins(ch), start) {
            (true, None) => start = Some(offset),
            (false, Some(from)) => {
                runs.push((from, &text[from..offset]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        runs.push((from, &text[from..]));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn forms_of(text: &str) -> Vec<(char, JoiningForm)> {
        forms(text).into_iter().map(|l| (l.ch, l.form)).collect()
    }

    #[test]
    fn classifies_the_letters_the_rule_turns_on() {
        assert_eq!(joining_type('س'), JoiningType::Dual);
        assert_eq!(joining_type('ا'), JoiningType::Right);
        assert_eq!(joining_type('ء'), JoiningType::NonJoining);
        assert_eq!(joining_type('\u{0640}'), JoiningType::Causing);
        assert_eq!(joining_type('\u{064E}'), JoiningType::Transparent);
        assert_eq!(joining_type('A'), JoiningType::NonJoining);
        assert_eq!(joining_type(' '), JoiningType::NonJoining);
    }

    #[test]
    fn shapes_the_word_every_arabic_primer_starts_with() {
        // سلام: seen joins forward, lam joins both ways, alef takes a join
        // from the lam but gives none, so the meem after it stands alone.
        assert_eq!(
            forms_of("سلام"),
            vec![
                ('س', JoiningForm::Initial),
                ('ل', JoiningForm::Medial),
                ('ا', JoiningForm::Final),
                ('م', JoiningForm::Isolated),
            ]
        );
    }

    #[test]
    fn a_right_joining_letter_stops_the_join_after_it() {
        // مدرسة: the dal and the reh are right-joining, so each takes a join
        // from the left and gives none onward.
        assert_eq!(
            forms_of("مدرسة"),
            vec![
                ('م', JoiningForm::Initial),
                ('د', JoiningForm::Final),
                ('ر', JoiningForm::Isolated),
                ('س', JoiningForm::Initial),
                ('ة', JoiningForm::Final),
            ]
        );
    }

    #[test]
    fn a_mark_is_stepped_over_not_joined_through() {
        // The fatha between the two letters is invisible to joining: the beh
        // is still initial and the taa still final.
        assert_eq!(
            forms_of("بَت"),
            vec![('ب', JoiningForm::Initial), ('ت', JoiningForm::Final)]
        );
        // And the mark itself takes no form.
        assert_eq!(forms("بَت").len(), 2);
    }

    #[test]
    fn tatweel_joins_its_neighbours_without_taking_a_form() {
        assert_eq!(
            forms_of("بـت"),
            vec![('ب', JoiningForm::Initial), ('ت', JoiningForm::Final)]
        );
    }

    #[test]
    fn a_space_breaks_the_join_and_latin_does_too() {
        assert_eq!(
            forms_of("بب بب"),
            vec![
                ('ب', JoiningForm::Initial),
                ('ب', JoiningForm::Final),
                ('ب', JoiningForm::Initial),
                ('ب', JoiningForm::Final),
            ]
        );
        assert_eq!(forms_of("بAب"), vec![('ب', JoiningForm::Isolated); 2]);
    }

    #[test]
    fn a_lone_letter_is_isolated() {
        assert_eq!(forms_of("ب"), vec![('ب', JoiningForm::Isolated)]);
        assert_eq!(forms_of("ء"), vec![]); // hamza is not a joining letter
    }

    #[test]
    fn offsets_are_byte_offsets_into_the_text_given() {
        let text = "مرحبا world";
        let letters = forms(text);
        assert_eq!(letters.first().map(|l| l.offset), Some(0));
        for letter in &letters {
            assert_eq!(text[letter.offset..].chars().next(), Some(letter.ch));
        }
    }

    #[test]
    fn a_block_the_table_does_not_state_is_never_guessed_at() {
        // U+0750 ARABIC LETTER BEH WITH THREE DOTS HORIZONTALLY BELOW is a
        // real dual-joining letter of Arabic Supplement, and this module has
        // not read that block. It must claim nothing about it, nor about the
        // beh beside it whose form depends on it.
        assert_eq!(joining_type('\u{0750}'), JoiningType::Unstated);
        assert_eq!(forms_of("\u{0750}ب"), vec![]);
        // A letter two places away is unaffected: only neighbours matter.
        assert_eq!(
            forms_of("\u{0750} بب"),
            vec![('ب', JoiningForm::Initial), ('ب', JoiningForm::Final)]
        );
    }

    #[test]
    fn presentation_forms_are_not_this_modules_business() {
        // Pre-shaped text is already reported by `presentation-forms`, and
        // asking a font to shape it would be asking the wrong question.
        assert_eq!(joining_type('\u{FE91}'), JoiningType::Unstated);
        assert_eq!(forms_of("\u{FE91}"), vec![]);
    }

    #[test]
    fn a_run_is_what_a_join_can_cross() {
        assert_eq!(arabic_runs("سلام"), vec![(0, "سلام")]);
        assert_eq!(arabic_runs("hello"), vec![]);
        assert_eq!(arabic_runs("بب بب"), vec![(0, "بب"), (5, "بب")]);
        // A mark travels with the letter it sits on.
        assert_eq!(arabic_runs("بَت"), vec![(0, "بَت")]);
        // A digit ends a run, exactly as the join it cannot carry does.
        assert_eq!(arabic_runs("ب1ب"), vec![(0, "ب"), (3, "ب")]);
    }

    #[test]
    fn segmenting_into_runs_changes_no_letters_form() {
        // The property that makes `arabic_runs` safe: a run boundary is
        // always a join boundary, so per-run forms equal whole-text forms.
        let text = "مرحبا world سلام 2026 مدرسة";
        let whole = forms(text);
        let by_run: Vec<Letter> = arabic_runs(text)
            .into_iter()
            .flat_map(|(offset, run)| {
                forms(run).into_iter().map(move |mut letter| {
                    letter.offset += offset;
                    letter
                })
            })
            .collect();
        assert_eq!(whole, by_run);
    }
}
