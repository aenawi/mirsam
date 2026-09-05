# Roadmap

Milestones are ordered by *risk retired per unit of work*, not by format
popularity. Each ships something usable on its own.

Codenames come from `tagtastic`, theme `arabian_birds`.

---

## M0 — Foundation ✅ *shipped*

**v0.1.0 “Steppe Eagle”**

Hexagonal skeleton, bidi engine, eight rules, PPTX reader, `audit` / `explain`
/ `rules`, JSON output, 22 tests.

Proves the architecture end-to-end on one format before it is copied five times.

---

## M1 — Repair, byte-preserving

**v0.2.0** · *the hardest correctness problem in the project*

- `DocumentWriter` for PPTX: apply `Fix` through the token stream.
- `mirsam repair in.pptx out.pptx`, refusing to overwrite its source.
- Round-trip guarantee: a repair with zero applicable fixes produces a
  byte-identical file. Enforced by test, not by intention.
- Golden-file corpus: real decks in, diffs asserted.

**Done when** repairing a deck changes exactly the attributes named in the plan
and PowerPoint opens the result without a repair prompt.

**Risk** — `mc:AlternateContent` blocks and unusual namespace prefixes. The
token-stream approach is chosen precisely to survive them; the corpus must
include one.

---

## M2 — Inheritance resolution

**v0.3.0** · *the accuracy milestone*

Today the PPTX adapter marks a property `Explicit` or `Unset`. Real PowerPoint
resolves paragraph → placeholder → layout → master → theme.

- Walk the relationship graph; populate `Resolved::Inherited`.
- List-level properties (`lvl1pPr` … `lvl9pPr`).
- Theme font scheme for the complex-script slot.

**Done when** a deck whose direction is set only on the slide master reports
zero `direction-unset` warnings. **Met** by
[PLAN](PLAN.md) 2.2, on the corpus and on hand-built packages. The two
remaining bullets landed in 2.3: a paragraph's `a:pPr/@lvl` now selects which
of the nine levels answers it, and a `+mn-cs` reference resolves through the
theme's `a:fontScheme`. An inherited language tag is the one property left
unresolved, and deliberately: ADR 0007's agreement test is stated for
direction and alignment, and there is no decided answer for a `lang` that
disagrees with the letters.

This is where the `Resolved<T>` design pays for itself, and where a linter that
lacks it starts producing noise users learn to ignore. It is also where the
design needed a correction: resolving a value says what the reader will see, not
that anyone chose it, so an inherited value silences a finding only where it
agrees with the text
([ADR 0007](adr/0007-an-inherited-default-is-not-a-choice.md)).

---

## M3 — Word

**v0.4.0**

DOCX reuses the OOXML package layer; only the vocabulary differs
(`w:bidi`, `w:jc`, `w:lang/@w:bidi`, `w:rFonts/@w:cs`).

- **Landed:** the reader. `mirsam audit report.docx` works, and no line of
  `mirsam-core` changed to make it. `w:jc` turned out to be direction-relative
  in Word whatever the standard says, so `alignment-incoherent` is
  structurally silent on DOCX — see [`PLAN.md`](PLAN.md) §3.2.
- **Landed:** style-chain inheritance (`docDefaults` → style → direct
  formatting), plus the table styles a table and its cells reach — see
  [`PLAN.md`](PLAN.md) §3.3 and §3.4.
- **Landed:** tables. `w:bidiVisual` is reported missing only where the cells
  genuinely read right to left. A table style can supply it, which made a
  container the first unit able to inherit its direction — so ADR 0007's
  agreement test now covers containers too; see [`PLAN.md`](PLAN.md) §3.4.
- **Landed:** the shared conformance suite. One set of cases, stated in the
  shared model's vocabulary and run against both adapters through
  `DocumentReader`, with no case skipped and no adapter special-cased — and
  no line of `mirsam-core` changed to make DOCX pass, which was the question
  the suite existed to answer. Two Word documents joined the golden corpus.
  See [`PLAN.md`](PLAN.md) §3.5.
- **Landed:** the writer. `mirsam repair report.docx fixed.docx` works, and
  the corpus recorded the refusal it replaced, so it arrived as a diff on a
  real document rather than as a test somebody remembered to update. The
  interesting result is what the writer did *not* need: PowerPoint's rewriter
  must be told which way each paragraph reads before it can write a `Start`,
  because `algn` names physical edges, and Word's `w:jc` is already relative —
  so the whole inheritance pass is absent from this adapter rather than
  skipped in it. Two repairs are refused, and both refusals are the format's:
  Word cannot state a physical edge, and a typed bullet can only join a list
  the document already defines. See [`PLAN.md`](PLAN.md) §3.6.
- Numbering and fields.

**Done when** the writer lands and `repair` stops refusing. **Met.** The
abstraction question — does one suite hold both adapters unchanged — is
answered, and so is its writing half: three vocabularies now sit on one
token-rewrite scaffold.

---

## M4 — Shaping and fonts

**v0.5.0** · *the capability nothing else has*

Add `rustybuzz` and `ttf-parser`:

- **Landed:** Arabic letters are shaped through a real OpenType shaper, and
  `shaping-broken` reports a font that produces no joining form where at
  least four were required — text correct in Unicode that renders as
  disconnected glyphs. See [`PLAN.md`](PLAN.md) §4.1 and §4.3.
- **Landed:** `font-coverage` checks the resolved font for every Arabic
  codepoint used and names the missing ones — `U+067E ARABIC LETTER PEH`.
  Both checks are opt-in behind `--fonts`, because they read the machine
  rather than the document; see [`PLAN.md`](PLAN.md) §4.2 and §4.3.
- **Landed:** `tatweel-padding` separates tatweel typed as visual padding
  from tatweel doing its job — carrying a harakat, showing a letter's
  contextual form, drawing a rule — and `--strip-tatweel` deletes only the
  padding. See [`PLAN.md`](PLAN.md) §4.4.

Replaces the "render every slide and eyeball it" step that every Arabic
workflow currently pushes onto a human.

---

## M5 — Web and spreadsheets

**v0.6.0**

- **Landed:** the HTML reader. `dir`, `lang` and the part of CSS that decides
  direction — `<style>`, a `style` attribute and a stylesheet linked by a
  relative path, with selectors, specificity and `!important`, because on the
  web the direction is usually in the stylesheet rather than in the document.
  A stylesheet on a server is not fetched, and the report names every one it
  did not read rather than letting an absent value look decided. See
  [`PLAN.md`](PLAN.md) §5.1 and
  [ADR 0009](adr/0009-a-source-the-adapter-could-not-read-is-part-of-the-report.md).
- **Landed:** four rules for the defects only the web can write.
  `bidi-override` for `<bdo>`, which is an embedded U+202E in markup;
  `isolation-missing` for the `<bdi>` that should have been around an
  interpolated name, proved by resolving the paragraph with and without the
  run isolated; `inset-physical` for `margin-left` where
  `margin-inline-start` was meant; and `order-reversed` for a layout made to
  look right to left by reversing its boxes rather than declaring a direction.
  They live in `mirsam-core` and the other two formats return `Inexpressible`,
  which is four new refusals rather than four skips. See
  [`PLAN.md`](PLAN.md) §5.2.
- **Landed:** the XLSX adapter, and the second format with a writer. A cell is
  a paragraph and the worksheet around it is the container, because
  `sheetView/@rightToLeft` decides which side column A sits on — reported only
  where the sheet has two or more columns of text, since one column has no
  column order to get wrong. `readingOrder` and `horizontal` are the cell's own
  word, `cellStyleXfs` the named style behind them, and the sheet supplies the
  reading order to any cell that states none. A formula's cached result is not
  judged and is named in `sources.unread`, which is ADR 0009 reaching a second
  format. `repair` writes a workbook: it appends a format record and repoints
  one cell's `@s` rather than editing what forty cells share, appends a shared
  string rather than rewriting one a dozen cells show, and leaves every `<f>`
  and every `<definedName>` byte for byte. Excel has no language slot on a
  cell and no list feature, so `SetLanguage` and `ConvertLiteralBullet` are
  refused and listed rather than claimed. See [`PLAN.md`](PLAN.md) §5.3.

---

## M6 — PDF, read-only

**v0.7.0**

- Text extraction order — a searchable PDF must extract sensibly.
- Detect pre-shaped presentation forms and reversed strings, the two ways
  Arabic PDFs are commonly built wrong.
- Report embedded-font coverage.

**Never repairs.** A broken Arabic PDF is rebuilt from source. `DocumentWriter`
is not implemented, and the type system says so.

---

## M7 — Distribution and agents

**v1.0.0**

- Static binaries for `linux-{x64,arm64}`, `darwin-{x64,arm64}`, `windows-x64`.
- `cargo install mirsam-cli`; Homebrew tap.
- Agent skills — one thin `SKILL.md` per format, all delegating to one binary,
  no duplicated prose.
- Stable JSON schema, versioned and documented.
- SARIF output for code-scanning pipelines.

**Done when** an agent on a machine with no Python, no Node and no Office can
audit and repair an Arabic deck.

---

## Explicitly out of scope

- **Authoring or translating Arabic.** This tool checks correctness; it does
  not write content.
- **Rendering.** No rasterisation, no layout engine, no screenshots.
- **Font embedding.** Licensing is the user's to reason about; the tool reports
  substitution risk and stops there.
- **In-place PDF repair.** See M6.
