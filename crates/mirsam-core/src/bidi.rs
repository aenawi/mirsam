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
}
