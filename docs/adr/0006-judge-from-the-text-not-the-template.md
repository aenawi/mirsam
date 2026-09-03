# 6. Judge direction and alignment from the text, not the template

**Status:** accepted · 2026-09-03

## Context

The first time a person opened a repaired deck in PowerPoint (#6), the Arabic
paragraphs read right-to-left and sat on the left edge, and the audit had
called the deck clean. Both were true. The repair had set `rtl="1"` and
touched no alignment, because no alignment was written on those paragraphs:
they took it from the slide master of an English template, whose body style
says `algn="l" rtl="0"`. The alignment rule looked only at alignment written
on the paragraph itself, and the direction rule stayed silent on an
explicitly left-to-right title because pure Arabic reorders identically under
either base direction. Nothing was proven wrong, and the deck looked wrong.

ADR 0004 says defects are proven, not asserted, and invariant 2 says a rule
that fires on inherited formatting is a bug. Both were written to stop the
tool forcing a design choice — a centred title — onto an author. Neither
anticipated a template that never considered Arabic at all.

The adapter cannot yet read the template. Resolving the chain from paragraph
to layout to master is milestone M2, and until it lands an absent alignment
is `Unset`, not `Inherited`: the tool does not know what the paragraph gets.

Three questions were put to the maintainer with a recommendation each; the
answers below are theirs.

## Decision

1. **What a paragraph is, is judged from its own letters.** A paragraph is
   right-to-left when most of its strong letters are Arabic, wherever they
   sit — `bidi::dominant_direction`, which the rules already use — and not
   when its first letter is. The difference is a sentence that opens with a
   Latin acronym, `GPS يعتمد عليه النظام…`: an Arabic sentence, judged by
   counting; a left-to-right one, judged by its first letter. The template
   is never consulted for this, because no deck can be assumed to come from
   any particular template.

2. **Right-to-left text with no alignment of its own is a note, repaired on
   request.** `alignment-unset` fires on `Unset` — never on `Inherited`, so
   invariant 2 stands — at severity *note*, which never blocks, `--strict`
   or not. The tool cannot tell a body paragraph left on the left edge from a
   title the layout centres, so it says what it sees and does not act.
   `repair --align` writes `Start` onto those paragraphs, which the adapter
   lowers to the right edge for right-to-left text. Choosing the flag means
   accepting that a layout-centred title goes right as well; the corpus's
   correctly authored deck made the same choice by hand, titles included.

3. **An explicit direction contrary to the text is a warning even when the
   letter order is identical.** `direction-mismatch` keeps its error tier,
   proven by two differing renderings, and gains a warning tier for a
   direction the author *wrote* that disagrees with the text's own when the
   orders coincide. The paragraph direction still decides which edge is the
   start and where edge punctuation lands, and no alignment repair can be
   lowered correctly while it is wrong. The evidence carried is the two
   renderings, equal — which is exactly the claim being made. Absent
   direction stays `direction-unset`'s finding; inherited direction is the
   container's design and is not reported.

ADR 0004's severity table gains a row: *declared property contradicts the
text; rendering order unaffected, alignment affected* → warning.

## Consequences

- The corpus deck repaired with `--align` now carries the alignment its
  correctly authored twin has. The golden corpus repairs under `--align`, so
  the fix is exercised on every deck.
- Every Arabic paragraph on an English template without its own alignment
  is a note from now on. On the M0 fixture that is two notes; on a real deck
  it may be every paragraph. That is the honest count, and it costs nothing
  in exit codes.
- **Cost:** under `--align`, a title a layout centres is pushed right. Until
  M2 this is the caller's call, stated on the flag. When M2 resolves the
  chain, an `Inherited(Center)` or `Inherited(Right)` must silence the note
  and an `Inherited(Left)` on right-to-left text must become the finding;
  #8 is the acceptance test for that.
- **Cost:** the direction rule now has a tier that is not proven by a
  differing rendering. It is still proven by something a reviewer can check
  — the declared direction and the text's own — and it is a warning, not an
  error, for that reason.
- Not decided here: what M2 should conclude from a master whose own body
  style says `rtl="0"`. That is the evidence that separates an English
  template from a genuine right-to-left one, and the ADR that lands 2.2
  should say what to do with it.
