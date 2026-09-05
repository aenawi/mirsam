# 8. A standalone letter is not a shaping defect

**Status:** accepted · 2026-09-05

## Context

4.1 exists to catch the defect nothing in this tool could previously see:
Arabic that is correct Unicode, correctly directed, correctly aligned, and
renders as a row of disconnected letters because the font answering for it has
no shaping tables. Reading XML will never find it. Shaping the text will.

The mechanism is not in doubt. A font's `cmap` gives the glyph a character
draws when nothing has shaped it — the standalone form. A shaper applies
`init`, `medi` or `fina` and substitutes another. So "did this letter join?"
is answerable without knowing anything about a font's design, its glyph names,
its outlines or its rendering: the character's own glyph is not among the ones
that came back. A font with no `GSUB` cannot produce any other.

The question this ADR settles is what to *conclude* from one letter that came
back standalone, and the obvious answer is wrong.

Shaping `مرحبا` through macOS's Arial leaves the reh on its `cmap` glyph. So
does `بم` with its final meem. Arial renders Arabic perfectly, is one of the
most widely installed fonts on earth, and is the font a great many of the
decks this tool exists for actually use. It is not defective and neither is
its `fina`, which simply does not cover those letters. It does not need to: a
reh only ever takes a join from its right, and the stroke that makes that join
is drawn by the letter *before* it, so a design is free to use one glyph for
both the standalone reh and the final one. Several do.

A per-letter rule would therefore report a correct font on correct text. That
is ADR 0004's first failure mode — the false positive that teaches users to
disable the tool — arriving through a new door.

## Decision

**`shape` reports; it does not judge.** It says, for each letter, the form the
text required, whether the font produced a contextual glyph, drew the
standalone one, or had no glyph at all. `ShapedLetter::drew_standalone` is
named for the fact and not for a verdict, and its documentation says so.

**The only conclusion the evidence supports is the aggregate.** A font with no
shaping tables produces *no* joins in a run that required several. No design
choice looks like that: a font that shares a glyph between the standalone and
final forms of the right-joining letters still shapes every dual-joining one.
The signal is `joins_produced == 0` against a `joins_required` large enough to
mean something, and where that threshold sits is 4.3's to state.

**The fixtures make the false positive a test failure.**
`scripts/make-shaping-fixture.py` writes three fonts differing in one thing
each. `partial.ttf` shapes everything except the final forms of the
right-joining letters — Arial's behaviour, reduced to its principle — so a
rule that regresses to a per-letter verdict fails against a committed fixture
rather than on a user's deck.

**Two modules, because the expectation and the observation must be separately
checkable.** `joining` states Joining_Type and the contextual form each letter
is required to take, from the logical-order text alone with no font in the
room; every one of its tests is checkable against ArabicShaping.txt. `shape`
asks a real shaper what happened. Had the expectation been derived from the
shaper, agreement between them would prove nothing.

## Consequences

- The defect this milestone is for is still caught, and caught exactly: a
  Latin-only font with an Arabic `cmap`, or any font with no `GSUB`, produces
  zero joins on text that required many.
- **Cost:** a font that shapes *some* letters and not others cannot be
  reported at all, however wrong it looks. A design that dropped `medi` for
  one letter would pass. That is the price of not reporting Arial, and it is
  the right way round: this tool's silence is cheap and its false positives
  are not.
- **Cost:** the aggregate needs enough text to mean anything. A container
  holding one two-letter word gives a threshold nothing to work with, and 4.3
  must say so rather than reporting on it.
- `joining` states the Arabic block, U+0600..U+06FF, and marks every other
  Arabic-script block `Unstated`. A letter it cannot classify — Arabic
  Supplement, Extended-A, the presentation forms — produces no expectation,
  and neither do its neighbours. Silence over a guess, as everywhere else
  here. Widening the table is a change to one `match` and its tests.
- `mirsam-core` gains a fifth dependency and keeps invariant 1 intact.
  `Font::parse` takes bytes; which typeface a paragraph resolves to, and where
  that file lives, are questions about the world and stay in an adapter. That
  is 4.2, and until it lands nothing in the audit path calls any of this.
