# 7. An inherited default is not the author's choice

**Status:** accepted · 2026-09-04

## Context

ADR 0006 left one question open, and 2.2 cannot be written without answering
it: what should the tool conclude from a slide master whose own body style
says `rtl="0"`.

It matters because of invariant 2 — *a rule that fires on `Resolved::Inherited`
is a bug* — and because of what `Inherited` is documented to mean:
"Not set here, but supplied by an ancestor. Correct, and must not be 'fixed'."
That invariant exists for a good reason. It is the whole of ADR 0004's answer
to the prior art's first failure mode: a paragraph taking its alignment from a
layout is not defective, and a tool that "repairs" it silently overwrites a
design the author chose. Users learn to ignore a tool that does that.

Read literally, though, the invariant breaks the milestone it is supposed to
serve. Today `quarterly-report.pptx` reports seven `direction-unset` warnings
and seventeen `alignment-unset` notes on Arabic paragraphs that carry no
`rtl` and no `algn` of their own. Once 2.2 resolves the chain, those
paragraphs stop being `Unset`: they become `Inherited(Ltr)` and
`Inherited(Left)`, taken from the master. Under a literal reading of invariant
2, every one of those findings disappears. The tool would report *less* about
Arabic laid out left-to-right after gaining the ability to see why it is laid
out left-to-right. That is the exact defect #8 was filed for, made worse by
the milestone meant to fix it.

The corpus settles what the master's `rtl="0"` actually signifies. Both
PowerPoint-authored decks sit on the same master, and all three of its text
styles say the same thing:

```xml
titleStyle  <a:lvl1pPr algn="ctr" rtl="0" …>
bodyStyle   <a:lvl1pPr algn="l"   rtl="0" …>
otherStyle  <a:lvl1pPr algn="l"   rtl="0" …>
```

`quarterly-report-correct.pptx` — the deck the tool must leave completely
alone — inherits none of it. Every Arabic paragraph in it writes
`algn="r" rtl="1"` on itself, and only the numeric table cells take `algn="l"
rtl="0"`, also explicitly. The author of a correct Arabic deck on an English
template does not adjust the master; they state the direction where the text
is. Meanwhile the three hand-built decks, which *are* Arabic by design, carry
`rtl="1" algn="r"` in all three master styles.

So a master's `rtl="0"` is not evidence of a right-to-left design decision. It
is what every English PowerPoint template ships with, untouched, on a deck
whose author never considered Arabic. It is the absence of a decision wearing
the same clothes as one.

## Decision

Three questions were put to the maintainer with a recommendation each, worked
through the corpus paragraphs above; the answers below are theirs. §1 restates
invariant 2, which `AGENTS.md` calls non-negotiable, so it could not be
settled any other way.

**Inheritance resolves the value. It does not, by itself, establish that
anyone chose it.**

1. **`Inherited` is evidence of a choice only when it agrees with the text.**
   A property whose inherited value is consistent with the paragraph's own
   dominant direction (ADR 0006 §1) is the author's layout doing its job, and
   no rule may fire on it. A property whose inherited value *contradicts* the
   text is a default nobody aimed at the text, and the tool reports it exactly
   as it reports an absent one.

   Invariant 2 is restated accordingly, in `AGENTS.md` and in `PLAN.md`'s
   standing rule 2: *a rule that fires on formatting the author chose is a
   bug; `Inherited` is evidence of a choice only where it agrees with the
   text.*
   Nothing about the prior-art failure mode changes — an Arabic paragraph
   under an Arabic master is still silent, which is the case invariant 2 was
   written to protect.

2. **The master is consulted for what the reader will see, never for whether
   something is a defect.** ADR 0006 §1 said a paragraph is judged from its
   own letters, because no deck can be assumed to come from any particular
   template. Resolving the chain does not change that; it only makes the
   declared direction knowable where it was previously absent. What a defect
   *is* stays a question about the text.

3. **A contradicting inherited value keeps the severity the absent one has.**
   `direction-unset` and `alignment-unset` fire on `Inherited` that
   contradicts the text at the severity they already carry — warning and note
   respectively. `direction-mismatch` keeps its tiers from ADR 0006 §3 and
   stays a finding about a direction the author *wrote*; an inherited one is
   not written. This is chosen so that M2 changes the *reason* for a finding
   on an English-template deck without changing the finding, which is what
   makes the golden corpus a usable check on the milestone rather than a wall
   of churn.

4. **An agreeing inherited value silences the finding, and that is the
   milestone's acceptance.** For direction, "agrees" is the paragraph's own
   dominant direction. For alignment on right-to-left text, only `Left`
   contradicts:

   | Inherited on right-to-left text | Reader sees | Reported |
   |---|---|---|
   | `rtl="1"` | reads right-to-left | no |
   | `rtl="0"` | reads left-to-right | yes, `direction-unset` |
   | `algn="r"` / `Start` | starts on the right edge | no |
   | `algn="ctr"` | centred | no |
   | `algn="l"` | starts on the left edge | yes, `alignment-unset` |

   Centred is the case that earns the distinction: a layout that centres a
   title has made a design choice that reads correctly in either direction,
   and ADR 0006's cost note — that `--align` pushes such a title right —
   is what M2 retires. `algn="l"` under Arabic is the one that puts the text
   on the edge a reader does not start from.

   This is `ROADMAP.md`'s "done when" and the second half of #8, and it is
   what makes M2 worth shipping: the three RTL-mastered corpus decks must get
   quieter, and `quarterly-report.pptx` must not.

5. **A finding on an inherited value must show where the value came from.**
   Invariant 6 says a diagnostic a reviewer cannot verify without opening the
   application is not finished, and "the master says left-to-right" is not
   checkable unless the tool names the master. `Evidence` gains an optional
   `inherited_from` carrying the part and the property that supplied the value
   — `ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr@rtl` — rather than
   prose in `message`, because `AGENTS.md` tells consumers not to parse the
   human output. The relationship graph landed in 2.1 is what makes naming the
   part possible.

6. **A repair writes to the unit the finding names, never to the master.**
   Setting `rtl="1"` on a master would change every paragraph in the deck,
   including ones the tool never examined and text that is correctly
   left-to-right. The finding is about a paragraph; so is the fix.

Which of `titleStyle` / `bodyStyle` / `otherStyle` a given placeholder
resolves against, and how a layout's `a:lstStyle` sits between the paragraph
and the master, is lookup mechanics and belongs to 2.2. This ADR decides only
what to conclude once the value is found.

## Consequences

- 2.2 has a falsifiable acceptance in the golden corpus, stated before it is
  written: `broken-arabic.pptx`, `clean.pptx` and `torture.pptx` sit on
  `rtl="1" algn="r"` masters and must lose their paragraph-level
  `direction-unset` and `alignment-unset` findings;
  `quarterly-report.pptx` sits on an `rtl="0" algn="l"` master and must keep
  the count it has today; `quarterly-report-correct.pptx` states everything
  explicitly and must stay clean. **NOT RUN** — 2.2 does not exist yet, and
  these are the predictions the decision commits to, not measurements.

  **Measured, 2026-09-04, when 2.2 landed.** Four of the five held. The
  RTL-mastered decks lost every paragraph-level finding they had, except one
  in `torture.pptx` on a chart part — which has no layout and no master, so
  `Unset` there is the honest answer rather than a miss.
  `quarterly-report-correct.pptx` stayed clean.
  `quarterly-report.pptx` kept all seven `direction-unset` warnings at the
  same severity, with the reason changed and the master named, which is what
  §3 was chosen for. Its `alignment-unset` notes went from 17 to 13, so the
  prediction that it "must keep the count it has today" was wrong for
  alignment. The four that went are its centred titles, silenced by §4's own
  table — `algn="ctr"` reads correctly in either direction — which the
  prediction overlooked because it reasoned from this master's `bodyStyle`
  (`algn="l"`) and not its `titleStyle` (`algn="ctr"`). §4 is the operative
  rule and the outcome it gives is the one intended: retiring ADR 0006's cost
  note is stated two sentences above the prediction that contradicts it. The
  full table is in [`PLAN.md`](../PLAN.md) §2.2.
- Container findings (`container-direction`) are unaffected: a table's or a
  chart axis's direction has no inheritance chain of this shape, and ADR 0006
  judges it from the text it lays out.
- **Cost:** invariant 2 is now a conditional rather than an absolute, and a
  conditional is easier to get wrong. The condition is narrow and mechanical
  — does the inherited value agree with `bidi::dominant_direction` — and it
  is the same comparison the rules already make on `Explicit`. It is stated
  in `AGENTS.md` so that a future rule author meets it before writing one.
- **Cost:** `Evidence` gains a field, so the JSON report changes shape and
  every committed report regenerates. The field is optional and additive; the
  schema promised in M7 does not exist yet, which is the cheapest moment for
  this to happen.
- **Cost:** the tool still cannot distinguish a master that says `rtl="0"`
  because it is an untouched English default from one whose author set it
  deliberately on a mixed-language deck. It does not try. It reports what
  contradicts the text and names the part that said it, which lets a reader
  make that call in one look — and a deliberate `rtl="0"` over Arabic text is
  a thing worth showing its author anyway.
