//! Which typed U+0640 ARABIC TATWEEL are padding and which are the character
//! doing its job.
//!
//! Tatweel is not a defect. It is the kashida: the elongation Arabic
//! typography has justified text with for a thousand years, and a font's
//! `GSUB` inserts it at layout time — where it belongs, and where nothing in
//! this crate will ever see it, because it never lands in the stored string.
//! A tatweel that *is* in the string was typed, and a rule that concluded a
//! defect from that alone would be [ADR 0004]'s first failure mode: reporting
//! formatting the author chose.
//!
//! So this module states the threshold, and states it as a property of the
//! neighbours rather than of the character. Three uses are legitimate and all
//! three are recognisable:
//!
//! - **A base for a mark.** Tatweel then U+064E is how a fatha is written on
//!   its own, in a table of harakat or a keyboard legend. Delete the tatweel
//!   and the mark lands on whatever precedes it.
//! - **A letter's contextual form, shown alone.** Tatweel around a heh is
//!   medial heh in a primer or a dictionary; a noon with a tatweel after it is
//!   initial noon. The tatweel is what makes the form appear, and deleting it
//!   deletes the thing being shown.
//! - **A rule drawn with the character that draws rules.** A run of tatweel
//!   between two spaces is a separator, not an elongated word.
//!
//! What is left is elongation of a word to reach a width, and it is a defect
//! for a reason that has nothing to do with how it looks: the stored text is
//! no longer the word. A search for the heading will not match the heading, a
//! spell-checker will not know it, a screen reader will read it, and the width
//! it was measured against is gone the moment the box, the font or the point
//! size changes.
//!
//! [ADR 0004]: ../../../docs/adr/0004-prove-defects-dont-assert-them.md

use crate::joining::{JoiningType, joining_type};
use crate::script::TATWEEL;

/// U+0640 is two bytes in UTF-8, which is what makes a run's offsets
/// arithmetic rather than a second pass over the text.
const TATWEEL_LEN: usize = 2;

/// What a run of tatweel is there for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Two or more in a row, joined to a letter: the word was stretched to
    /// reach a width. No typography needs a second tatweel — one is enough to
    /// carry a mark or to show a form — so the repetition is the evidence.
    Stretched,
    /// One, between two characters that already join each other. Both take
    /// exactly the same contextual form without it, so it adds width and
    /// nothing else: the wedge that fakes an alignment inside a word.
    Wedged,
    /// One, at an edge where a join begins or ends. This is how a letter's
    /// initial, medial or final form is written on its own, and the tool
    /// cannot tell that from a single character of padding — so it does not
    /// guess.
    Displayed,
    /// The base a combining mark is written on.
    Carrier,
    /// Two or more, joined to nothing on either side: a rule or a separator,
    /// drawn with the character that draws them.
    Standalone,
}

impl Verdict {
    /// Whether this run is width the author typed rather than a character
    /// they meant.
    ///
    /// Only these two are reported, and only these two are ever deleted. A
    /// [`Wedged`](Verdict::Wedged) run is provably safe to delete — both
    /// neighbours keep the form they already had. A
    /// [`Stretched`](Verdict::Stretched) one need not be, and that is the
    /// point: a noon stretched to a width reverts to a plain noon, which is
    /// the correct rendering of the word nobody padded.
    pub fn is_padding(self) -> bool {
        matches!(self, Self::Stretched | Self::Wedged)
    }
}

/// A maximal stretch of consecutive tatweel, and what it is there for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// Byte offset of the first tatweel within the text passed to [`scan`].
    pub offset: usize,
    /// How many tatweel the run holds.
    pub length: usize,
    pub verdict: Verdict,
}

impl Run {
    /// The byte offset of each tatweel in the run, ascending.
    pub fn offsets(&self) -> Vec<usize> {
        (0..self.length)
            .map(|i| self.offset + i * TATWEEL_LEN)
            .collect()
    }
}

/// Every run of tatweel in `text`, judged by what surrounds it.
///
/// A run is *maximal*, so a tatweel, a fatha and a second tatweel are two runs
/// and not one: the mark between them is written on the first, and only the
/// first.
pub fn scan(text: &str) -> Vec<Run> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut runs = Vec::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index].1 != TATWEEL {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].1 == TATWEEL {
            index += 1;
        }

        // The character the run ends against, taken as it is: a mark there is
        // written *on* the run, which is the one thing that must not be
        // stepped over.
        let after = chars.get(index).map(|&(_, c)| joining_type(c));
        // And the last character before it that joining can see. A mark on the
        // preceding letter is invisible to the join, exactly as it is
        // everywhere else in `crate::joining`.
        let before = chars[..start]
            .iter()
            .rev()
            .map(|&(_, c)| joining_type(c))
            .find(|kind| *kind != JoiningType::Transparent);

        runs.push(Run {
            offset: chars[start].0,
            length: index - start,
            verdict: verdict(index - start, before, after),
        });
    }
    runs
}

/// The threshold, and the whole of it.
fn verdict(length: usize, before: Option<JoiningType>, after: Option<JoiningType>) -> Verdict {
    // A mark after the run is written on it. Whatever else the run is, it is
    // the base that mark needs, and deleting it would move the mark onto
    // whatever precedes.
    if after == Some(JoiningType::Transparent) {
        return Verdict::Carrier;
    }

    // Whether a join actually crosses the run: something before it that joins
    // forward, something after it that takes a join.
    let gives = matches!(
        before,
        Some(JoiningType::Dual | JoiningType::Left | JoiningType::Causing)
    );
    let takes = matches!(
        after,
        Some(JoiningType::Dual | JoiningType::Right | JoiningType::Causing)
    );

    match length {
        // Attached to a letter on either side and repeated: elongation.
        // Attached to neither: a horizontal rule somebody drew.
        2.. if gives || takes => Verdict::Stretched,
        2.. => Verdict::Standalone,
        // One, with the join already crossing it: pure width.
        _ if gives && takes => Verdict::Wedged,
        // One, at an edge: a form being shown, and not the tool's to judge.
        _ => Verdict::Displayed,
    }
}

/// The byte offset of every tatweel this module calls padding, ascending.
///
/// What a finding lists and what a repair deletes, so the two can never
/// disagree.
pub fn padding_offsets(text: &str) -> Vec<usize> {
    scan(text)
        .iter()
        .filter(|run| run.verdict.is_padding())
        .flat_map(Run::offsets)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdicts(text: &str) -> Vec<(usize, Verdict)> {
        scan(text)
            .into_iter()
            .map(|run| (run.length, run.verdict))
            .collect()
    }

    /// `n` tatweel between `head` and `tail`. Spelled out rather than typed
    /// into a literal, because a run of tatweel in source is a run nobody can
    /// count by eye.
    fn padded(head: &str, n: usize, tail: &str) -> String {
        format!("{head}{}{tail}", TATWEEL.to_string().repeat(n))
    }

    #[test]
    fn a_heading_stretched_to_a_width_is_padding() {
        // العنوان with five tatweel pushed onto the end of it.
        assert_eq!(
            verdicts(&padded("العنوان", 5, "")),
            vec![(5, Verdict::Stretched)],
            "a run joined to the letter before it is elongation"
        );
        // And on the leading side, where alef takes the join.
        assert_eq!(
            verdicts(&padded("", 3, "العنوان")),
            vec![(3, Verdict::Stretched)]
        );
        // Two is already a run, which is the whole of the threshold: one
        // tatweel is a character, two is a decision about width.
        assert_eq!(
            verdicts(&padded("العنوان", 2, "")),
            vec![(2, Verdict::Stretched)]
        );
    }

    #[test]
    fn one_wedged_between_two_letters_that_already_join_is_padding() {
        // السلام with a tatweel between the seen and the lam. Both are
        // dual-joining, so both keep the exact form they had; the character
        // buys width and nothing else.
        let text = padded("الس", 1, "لام");
        assert_eq!(verdicts(&text), vec![(1, Verdict::Wedged)]);
        assert_eq!(padding_offsets(&text), vec!["الس".len()]);
    }

    #[test]
    fn a_mark_written_on_a_tatweel_is_what_tatweel_is_for() {
        // How a fatha is written on its own, in a table of harakat.
        assert_eq!(verdicts("\u{0640}\u{064E}"), vec![(1, Verdict::Carrier)]);
        // And inside a word, where deleting it would move the mark.
        assert_eq!(
            verdicts("ب\u{0640}\u{064E}ت"),
            vec![(1, Verdict::Carrier)],
            "the tatweel is the base the mark sits on"
        );
        assert!(padding_offsets("ب\u{0640}\u{064E}ت").is_empty());
    }

    #[test]
    fn a_letters_form_shown_on_its_own_is_not_padding() {
        // Medial heh, as a primer or a dictionary writes it: two runs, each
        // at an edge, each the thing being shown.
        let medial = padded("", 1, &padded("ه", 1, ""));
        assert_eq!(
            verdicts(&medial),
            vec![(1, Verdict::Displayed), (1, Verdict::Displayed)]
        );
        assert!(padding_offsets(&medial).is_empty());
        // Initial noon, and final noon.
        assert_eq!(verdicts(&padded("ن", 1, "")), vec![(1, Verdict::Displayed)]);
        assert_eq!(verdicts(&padded("", 1, "ن")), vec![(1, Verdict::Displayed)]);
    }

    #[test]
    fn a_rule_drawn_with_tatweel_is_not_an_elongated_word() {
        // Joined to nothing on either side: a separator, not a stretched word.
        assert_eq!(
            verdicts(&padded("مرحبا ", 4, " عالم")),
            vec![(4, Verdict::Standalone)]
        );
        assert_eq!(verdicts(&padded("", 4, "")), vec![(4, Verdict::Standalone)]);
    }

    #[test]
    fn a_run_is_maximal_and_a_mark_splits_it() {
        // The mark is written on the first tatweel and on no other, so the
        // two sides are judged separately rather than as one run.
        assert_eq!(
            verdicts("\u{0640}\u{064E}\u{0640}"),
            vec![(1, Verdict::Carrier), (1, Verdict::Displayed)]
        );
    }

    #[test]
    fn text_with_no_tatweel_produces_nothing() {
        assert!(scan("التقرير الفصلي").is_empty());
        assert!(scan("Performance rose").is_empty());
        assert!(padding_offsets("التقرير الفصلي").is_empty());
    }

    #[test]
    fn offsets_are_byte_offsets_into_the_text_given() {
        let text = padded("العنوان", 5, " والباقي");
        for offset in padding_offsets(&text) {
            assert_eq!(
                text[offset..].chars().next(),
                Some(TATWEEL),
                "offset {offset} does not land on a tatweel"
            );
        }
        assert_eq!(padding_offsets(&text).len(), 5);
    }

    #[test]
    fn deleting_a_run_a_join_crosses_changes_no_letters_form() {
        // The property `Wedged` is defined by, asserted against the module
        // that decides forms rather than restated here — and it holds however
        // long the run is, which is why a stretched word can be repaired
        // without asking what its letters looked like.
        use crate::joining::forms;
        let shapes = |text: &str| -> Vec<_> { forms(text).into_iter().map(|l| l.form).collect() };
        for n in [1, 5] {
            let text = padded("الس", n, "لام");
            let stripped: String = text.chars().filter(|c| *c != TATWEEL).collect();
            assert_eq!(shapes(&text), shapes(&stripped), "{n} tatweel");
        }
    }
}
