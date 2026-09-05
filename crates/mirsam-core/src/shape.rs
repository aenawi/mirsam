//! Shaping: what a font actually does with an Arabic run.
//!
//! [`crate::joining`] says what the text requires — this seen must be drawn
//! initial. This module hands the run to a real OpenType shaper and reports
//! what came back, letter by letter. The gap between the two is a defect no
//! attribute in any document format can express and no amount of reading XML
//! will find: text that is correct Unicode, correct direction, correctly
//! aligned, and renders as a row of disconnected letters because the font
//! answering for it has no Arabic shaping tables.
//!
//! ## What "joined" is decided by
//!
//! A font's `cmap` maps a character to the glyph it draws when nothing has
//! shaped it — the standalone form. A shaper applies `init`, `medi` and
//! `fina` and substitutes that glyph for another. So the question "did this
//! letter join?" is answered without knowing anything about the font's
//! design: **the character's own glyph is not among the ones that came
//! back.** A font with no `GSUB` cannot produce any other, which is exactly
//! the defect. Nothing here needs a glyph name, a design or a rendering.
//!
//! ## One standalone letter is not a defect, and a shipped font proves it
//!
//! The tempting next step — report every letter that came back standalone —
//! is wrong, and a real font proves it. Shaping `مرحبا` through macOS's
//! Arial leaves the reh on its `cmap` glyph, and the word renders perfectly:
//! reh only ever takes a join from its right, and the connecting stroke that
//! makes the join belongs to the letter *before* it, so a design needs no
//! separate final glyph for it. Arial's `fina` skips reh and meem for
//! exactly that reason. A check that concluded a defect from one such letter
//! would fire on one of the most widely installed Arabic fonts there is.
//!
//! What survives that is the aggregate: a font with no shaping tables
//! produces **no** joins in a run that required several, and no design
//! choice can look like that. `partial.ttf` in the shaping fixtures is built
//! to be Arial in this respect, so a rule that regresses to a per-letter
//! verdict fails a test rather than a user's deck.
//! `docs/adr/0008-a-standalone-letter-is-not-a-shaping-defect.md` records the
//! whole of it.
//!
//! So this module reports and does not judge. It says which letters were
//! required to join, which the font drew standalone, and which it has no
//! glyph for. Whether that adds up to a finding is a rule's decision
//! (PLAN §4.3), and a rule can only make it honestly if the facts arrive
//! unweighted.
//!
//! ## No I/O, here as everywhere
//!
//! [`Font::parse`] takes bytes. Which typeface a paragraph resolves to, and
//! where that file lives on which machine, are questions about the world and
//! belong to an adapter — invariant 1. The domain's business is what happens
//! to the text once the bytes are in hand.

use rustybuzz::{Direction, Face, UnicodeBuffer, script};

use crate::joining::{self, JoiningForm};

/// A parsed font, ready to shape.
///
/// Borrows its bytes: the caller owns the file it read, and the domain never
/// opened one.
pub struct Font<'a>(Face<'a>);

impl<'a> Font<'a> {
    /// Parse `data` as a font, taking face `index` from a collection.
    ///
    /// `None` if the bytes are not a font this shaper can read — which is a
    /// fact about the file, reported by whoever supplied it, and never a
    /// finding about the document.
    pub fn parse(data: &'a [u8], index: u32) -> Option<Self> {
        Face::from_slice(data, index).map(Font)
    }

    /// Whether the font has a glyph for `c` at all.
    ///
    /// Coverage is PLAN §4.2's subject. It appears here only so that shaping
    /// can tell "the font drew this letter unjoined" apart from "the font has
    /// no such letter", which are different defects with different repairs.
    pub fn covers(&self, c: char) -> bool {
        self.0.glyph_index(c).is_some()
    }

    fn unshaped_glyph(&self, c: char) -> Option<u16> {
        self.0.glyph_index(c).map(|id| id.0)
    }
}

/// What a font did with one letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Outcome {
    /// The character's own `cmap` glyph is not among the ones the shaper
    /// produced: the font substituted something for it.
    Contextual,
    /// The character's own `cmap` glyph came back: the font drew the
    /// standalone shape.
    ///
    /// Correct for a letter that joins to nothing, and — on its own — not a
    /// defect for one that does either. See the module documentation: a
    /// design may share one glyph between a letter's standalone and final
    /// forms, and several do.
    Standalone,
    /// The font has no glyph for the character. A coverage problem, not a
    /// shaping one, and reported as itself so that §4.2 owns it.
    Unmapped,
}

/// One letter, the form the text required of it, and what the font did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ShapedLetter {
    /// Byte offset of the character in the text that was shaped.
    pub offset: usize,
    pub ch: char,
    /// The form [`crate::joining`] requires, from the text alone.
    pub required: JoiningForm,
    pub outcome: Outcome,
    /// The glyphs the shaper produced for this character's cluster.
    ///
    /// One, normally. Two or more when a mark or a ligature merged into the
    /// cluster — a shaper reports a base and the harakat above it as one
    /// indivisible unit, and Arabic text carrying harakat is ordinary text,
    /// not an edge case. None at all when the merge went the other way and
    /// this character's glyphs are counted under a neighbouring cluster, as
    /// they are for the second half of a lam-alef ligature.
    ///
    /// Which is why [`Outcome`] asks whether the standalone glyph is
    /// *among* them rather than whether it is the only one: that question
    /// has the same answer however the shaper chose to group its output.
    pub glyphs: Vec<u16>,
}

impl ShapedLetter {
    /// A letter the text required to join that the font drew standalone.
    ///
    /// A fact, deliberately not a verdict. A letter that was never required
    /// to join is excluded because standalone is what it is *supposed* to
    /// come back as, and an unmapped one is excluded because that is a
    /// coverage problem with a different repair — but among what remains
    /// are letters of perfectly good fonts. Read the module documentation
    /// before treating one of these as a finding.
    pub fn drew_standalone(&self) -> bool {
        self.required.is_joined() && self.outcome == Outcome::Standalone
    }
}

/// What a font did with every Arabic letter in a piece of text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Shaping {
    /// Every Arabic letter shaped, in document order.
    pub letters: Vec<ShapedLetter>,
}

impl Shaping {
    /// Letters the text required to join, whatever the font then did.
    ///
    /// Zero of them means the text proves nothing about the font: a single
    /// standalone letter shapes identically through the best Arabic font and
    /// through one with no shaping tables at all.
    pub fn joins_required(&self) -> usize {
        self.letters
            .iter()
            .filter(|l| l.required.is_joined())
            .count()
    }

    /// Letters that were required to join and did.
    pub fn joins_produced(&self) -> usize {
        self.letters
            .iter()
            .filter(|l| l.required.is_joined() && l.outcome == Outcome::Contextual)
            .count()
    }

    /// Letters that were required to join and came back standalone.
    ///
    /// [`ShapedLetter::drew_standalone`] over the whole text: facts, not
    /// findings. A font that produced no joins at all will list every one of
    /// them; a font that shapes will still list a few.
    pub fn drawn_standalone(&self) -> impl Iterator<Item = &ShapedLetter> {
        self.letters.iter().filter(|l| l.drew_standalone())
    }

    /// Letters the font has no glyph for at all.
    ///
    /// Coverage, which PLAN §4.2 owns. Here so that a caller counting joins
    /// can see how much of the text the font never answered for, rather than
    /// reading an absent letter as a shaping result.
    pub fn unmapped(&self) -> impl Iterator<Item = &ShapedLetter> {
        self.letters
            .iter()
            .filter(|l| l.outcome == Outcome::Unmapped)
    }
}

/// Shape every Arabic run in `text` through `font`.
///
/// The text is segmented by [`joining::arabic_runs`] first and each run is
/// shaped as Arabic, right to left. Handing a shaper a whole mixed paragraph
/// would mean declaring one script for text that has two; segmenting costs
/// nothing, because a run boundary is a place a join could not have crossed.
///
/// Offsets in the result are into `text`, not into the run.
pub fn shape(font: &Font, text: &str) -> Shaping {
    let mut letters = Vec::new();
    for (offset, run) in joining::arabic_runs(text) {
        letters.extend(shape_run(font, run).into_iter().map(|mut letter| {
            letter.offset += offset;
            letter
        }));
    }
    Shaping { letters }
}

/// Shape one run, which the caller has already established is Arabic.
fn shape_run(font: &Font, run: &str) -> Vec<ShapedLetter> {
    let mut buffer = UnicodeBuffer::new();
    buffer.push_str(run);
    buffer.set_direction(Direction::RightToLeft);
    buffer.set_script(script::ARABIC);
    let shaped = rustybuzz::shape(&font.0, &[], buffer);

    // Which glyphs came out of each character. Clusters are the shaper's own
    // answer to "which input does this glyph belong to", and are byte offsets
    // into the run because that is what the buffer was filled with.
    let mut glyphs: Vec<(u32, u16)> = shaped
        .glyph_infos()
        .iter()
        .map(|info| (info.cluster, info.glyph_id as u16))
        .collect();
    glyphs.sort_unstable();

    joining::forms(run)
        .into_iter()
        .map(|letter| {
            let produced: Vec<u16> = glyphs
                .iter()
                .filter(|(cluster, _)| *cluster as usize == letter.offset)
                .map(|(_, glyph)| *glyph)
                .collect();

            let outcome = match font.unshaped_glyph(letter.ch) {
                None => Outcome::Unmapped,
                Some(unshaped) if produced.contains(&unshaped) => Outcome::Standalone,
                Some(_) => Outcome::Contextual,
            };

            ShapedLetter {
                offset: letter.offset,
                ch: letter.ch,
                required: letter.form,
                outcome,
                glyphs: produced,
            }
        })
        .collect()
}
