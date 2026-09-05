//! Resolved bidirectional order, via the full UAX#9 algorithm.
//!
//! This is what separates `mirsam` from an attribute linter. Rather than
//! asserting "the RTL flag is absent", the engine resolves what the text will
//! actually look like under each candidate base direction and reports a defect
//! only when the two differ. Plenty of Arabic paragraphs render identically
//! either way; those deserve a hygiene note, not a delivery blocker.

use crate::text::Direction;
use unicode_bidi::{BidiInfo, Level};

fn level(direction: Direction) -> Level {
    match direction {
        Direction::Rtl => Level::rtl(),
        Direction::Ltr => Level::ltr(),
    }
}

/// The resolved presentation of one text unit under a given base direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    pub base: Direction,
    /// Codepoints in visual (left-to-right on screen) order.
    ///
    /// Never print this to a terminal: the terminal will apply bidi to it a
    /// second time. It exists to be compared, and to be rendered by a client
    /// that knows to treat it as pre-ordered.
    pub visual: String,
}

/// Resolve `text` under `base` using UAX#9.
pub fn resolve(text: &str, base: Direction) -> Order {
    let info = BidiInfo::new(text, Some(level(base)));
    let visual = match info.paragraphs.first() {
        Some(para) => info.reorder_line(para, para.range.clone()).into_owned(),
        None => String::new(),
    };
    Order { base, visual }
}

/// The natural base direction of `text`, per UAX#9 rule P2/P3.
pub fn auto_direction(text: &str) -> Direction {
    let info = BidiInfo::new(text, None);
    match info.paragraphs.first() {
        Some(para) if para.level.is_rtl() => Direction::Rtl,
        _ => Direction::Ltr,
    }
}

/// Does `text` render differently under the two base directions?
///
/// `false` means the declared direction is cosmetically irrelevant for this
/// particular string — useful for downgrading a would-be error to a note.
pub fn order_differs(text: &str, a: Direction, b: Direction) -> bool {
    a != b && resolve(text, a).visual != resolve(text, b).visual
}

/// U+2068 FIRST STRONG ISOLATE and U+2069 POP DIRECTIONAL ISOLATE.
///
/// Used only to build the *hypothetical* string [`isolating_changes_order`]
/// resolves, and stripped out of both renderings before they are compared.
/// Invariant 4 forbids writing a control into a document; asking what the
/// algorithm would have produced had one been there is the question that makes
/// the finding provable rather than asserted.
const FSI: char = '\u{2068}';
const PDI: char = '\u{2069}';

/// The order a document *imposes* on `text` when it overrides the algorithm
/// rather than declaring a direction.
///
/// An override sets every character in the run to one level, so the run is laid
/// out in that direction whatever its characters are: left to right leaves them
/// as stored, right to left lays them out in reverse. That is exactly what
/// U+202D and U+202E do to a run, and exactly what `<bdo>` does to an element.
///
/// The result is a *visual*-order string, like [`Order::visual`], and carries
/// the same warning: it exists to be compared, never to be stored or printed.
/// Invariant 5 forbids a document holding one; computing one as evidence is how
/// the tool shows what the document asked for.
pub fn imposed(text: &str, direction: Direction) -> String {
    match direction {
        Direction::Ltr => text.to_string(),
        Direction::Rtl => text.chars().rev().collect(),
    }
}

/// How `text` would resolve under `base` if the byte range
/// `offset..offset + len` were isolated from its surroundings.
///
/// This is the whole of the `<bdi>` question, asked so that it can be answered
/// from the text rather than asserted about the markup. A run that is isolated
/// cannot reorder anything outside it; a run that is not can, because the
/// neutrals on either side resolve against whatever strong direction it happens
/// to begin or end with. Comparing this with [`resolve`] says whether the run's
/// content is deciding the layout of text that is not in it.
///
/// The isolates are taken back out of the result, so the two renderings can be
/// compared on the characters they actually share. `None` for a range that does
/// not fall on character boundaries: an adapter that miscounted costs a finding
/// rather than a panic.
pub fn resolve_isolating(text: &str, base: Direction, offset: usize, len: usize) -> Option<Order> {
    let end = offset.checked_add(len)?;
    let (head, run, tail) = (
        text.get(..offset)?,
        text.get(offset..end)?,
        text.get(end..)?,
    );
    let order = resolve(&format!("{head}{FSI}{run}{PDI}{tail}"), base);
    Some(Order {
        base,
        visual: order
            .visual
            .chars()
            .filter(|c| ![FSI, PDI].contains(c))
            .collect(),
    })
}

/// Does isolating that range change how the rest of `text` is laid out?
///
/// `false` when it does not, and `false` for a range this text does not have:
/// silence is the only safe answer to a question that could not be asked.
pub fn isolating_changes_order(text: &str, base: Direction, offset: usize, len: usize) -> bool {
    resolve_isolating(text, base, offset, len)
        .is_some_and(|isolated| isolated.visual != resolve(text, base).visual)
}

/// The direction the *bulk* of `text` is written in.
///
/// Distinct from [`auto_direction`], which follows UAX#9's first-strong rule.
/// The two disagree exactly in the case that traps bilingual authors: a
/// mostly-Arabic sentence opening with a Latin acronym is semantically RTL but
/// auto-detects as LTR.
pub fn dominant_direction(text: &str) -> Direction {
    let (mut rtl, mut ltr) = (0usize, 0usize);
    for c in text.chars() {
        if crate::script::is_arabic_letter(c) {
            rtl += 1;
        } else if c.is_alphabetic() && c.is_ascii() {
            ltr += 1;
        }
    }
    if rtl > ltr {
        Direction::Rtl
    } else {
        Direction::Ltr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: &str = "ارتفع الأداء بنسبة 25% في Q4 2026.";
    const PLAIN: &str = "مرحبا بالعالم";

    #[test]
    fn mixed_text_reorders_differently_per_base_direction() {
        assert!(order_differs(MIXED, Direction::Rtl, Direction::Ltr));
    }

    #[test]
    fn plain_arabic_is_direction_insensitive() {
        // No neutrals at the edges, no LTR runs: nothing for the base
        // direction to change. Reporting this as an error would be noise.
        assert!(!order_differs(PLAIN, Direction::Rtl, Direction::Ltr));
    }

    #[test]
    fn auto_direction_follows_first_strong_character() {
        assert_eq!(auto_direction(MIXED), Direction::Rtl);
        assert_eq!(auto_direction("GPS and TCP/IP"), Direction::Ltr);
        // A leading Latin acronym flips the auto-detected base — the classic
        // trap that makes bilingual paragraphs render wrongly.
        assert_eq!(auto_direction("GPS يعتمد عليه النظام"), Direction::Ltr);
    }

    #[test]
    fn dominant_direction_ignores_the_first_strong_trap() {
        let text = "GPS يعتمد عليه النظام في تحديد المواقع";
        assert_eq!(auto_direction(text), Direction::Ltr);
        assert_eq!(dominant_direction(text), Direction::Rtl);
    }

    #[test]
    fn resolution_is_order_preserving_for_pure_ltr() {
        let order = resolve("hello world", Direction::Ltr);
        assert_eq!(order.visual, "hello world");
    }

    #[test]
    fn an_imposed_order_is_the_run_laid_out_one_way_whatever_it_says() {
        // The difference between declaring a direction and overriding the
        // algorithm, in one assertion: under an override the digits go
        // backwards too, which is the defect `<bdo>` and U+202E share.
        assert_eq!(imposed("Q4 2026", Direction::Ltr), "Q4 2026");
        assert_eq!(imposed("Q4 2026", Direction::Rtl), "6202 4Q");
        assert_eq!(resolve("Q4 2026", Direction::Rtl).visual, "Q4 2026");
    }

    #[test]
    fn isolating_a_run_that_decides_its_surroundings_changes_the_order() {
        // The `<bdi>` case: an Arabic name interpolated into an English line.
        // Without isolation the trailing neutrals resolve against the Arabic
        // and move; with it they stay where the English put them.
        let text = "Owner: مالك المشروع - 5";
        let name = text.find('م').expect("the name is in there");
        let len = "مالك المشروع".len();
        assert!(isolating_changes_order(text, Direction::Ltr, name, len));
    }

    #[test]
    fn isolating_a_run_that_decides_nothing_changes_nothing() {
        // A run of the same direction as everything around it has nothing to
        // leak, and a rule built on this stays silent on ordinary markup.
        let text = "ارتفع الأداء في الربع الرابع";
        let word = text.find("الأداء").expect("the word is in there");
        assert!(!isolating_changes_order(
            text,
            Direction::Rtl,
            word,
            "الأداء".len()
        ));
    }

    #[test]
    fn a_range_the_text_does_not_have_is_answered_with_silence() {
        // Byte offsets arrive from an adapter, and one that miscounted must
        // cost a finding rather than panic on a user's document.
        let text = "مرحبا";
        assert_eq!(resolve_isolating(text, Direction::Rtl, 1, 2), None);
        assert!(!isolating_changes_order(text, Direction::Rtl, 0, 999));
        assert!(!isolating_changes_order(
            text,
            Direction::Rtl,
            usize::MAX,
            1
        ));
    }
}
