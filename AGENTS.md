# AGENTS.md — mirsam

Instructions for AI coding agents (Claude, Codex, Grok, OpenCode, Pi, …) using
or extending this tool.

## What this is

**mirsam** is an independent Rust CLI that audits and repairs Arabic
right-to-left, bidirectional and typography defects in documents. Single static
binary: no Python, no Node, no Office, no runtime of any kind on the target
machine.

`audit` reads `.pptx` and `.docx`. `repair` writes `.pptx` only: a `.docx`
is refused as a readable format without a writer, which is a usage error
(exit `2`) and not a claim that the document was not understood.

Inspired by Sultan Alsafran's MIT-licensed `arabic-presentations` skill. No
shared code. See [`CREDITS.md`](CREDITS.md).

## Using it

```bash
mirsam audit deck.pptx --format json     # full diagnostic model
mirsam audit deck.pptx --strict          # warnings block too
mirsam audit deck.pptx --fonts           # also check the fonts installed here
mirsam repair deck.pptx fixed.pptx --format json   # repaired copy, both audits
mirsam explain "<text>"                  # reproduce a defect with no document
mirsam rules --format json               # every check and its id
```

Exit codes: `0` clean · `1` blocking findings · `2` bad invocation ·
`3` document unreadable (or output unwritable). Branch on these; do not parse
the human output.

### Reading a diagnostic

Every finding carries `evidence`. For a direction defect that means
`visual_declared` and `visual_expected` — the resolved orders. **These are
visual-order codepoint sequences.** Never print them to a terminal or paste
them into a chat: the display layer will apply the bidirectional algorithm to
already-reordered text and show something misleading. Compare them
programmatically, or render the escaped form as `mirsam explain` does.

`fixable: true` means a mechanical repair exists.

`evidence.inherited_from` names the part and property that supplied a value
the unit did not state itself — `ppt/slideMasters/slideMaster1.xml
bodyStyle/lvl2pPr@rtl`. It is present exactly when the finding is about an
inherited value, which is a defect in the deck's template rather than in the
paragraph. The repair still writes to the paragraph the finding names: setting
`rtl="1"` on a master would change every paragraph in the deck, including text
that is correctly left-to-right.

A `.docx` cites Word's own chain instead: `word/styles.xml
style[Heading1]/pPr@bidi` is a named style, `word/styles.xml
docDefaults/pPrDefault/pPr@jc` the document's defaults. A style is cited by the
`w:styleId` that actually stated the value, not by the one the paragraph asked
for — a paragraph naming `Heading1` may well be answered by the `Normal` it is
`w:basedOn`, and citing `Heading1` would send a reviewer to a style that says
nothing about it.

The list level in that path is the level the paragraph is actually at
(`a:pPr/@lvl`, zero-based, so `lvl="1"` cites `lvl2pPr`). A font slot may name
a theme part instead — `ppt/theme/theme1.xml fontScheme/minorFont/cs@typeface`
— because a master writes `+mn-cs`, a reference, and the theme is where the
typeface itself is written.

**`alignment-incoherent` never fires on a `.docx`, and that is correct.**
Word's `w:jc` is direction-relative whatever the standard says — `left` is the
start edge of the paragraph, so a Word author has no way to write the hard
left that rule reports. Arabic starting on the wrong edge in Word is a
`w:bidi` defect, and `direction-mismatch` / `direction-unset` report it. Do
not read the rule's silence on Word as coverage that is missing.

Unit ids are adapter-issued and opaque: `<part>#p<n>` is a paragraph;
`<part>#tbl<n>` a table, `<part>#cols<n>` a text body laid out in two or more
columns, and `<part>#catax<n>` / `#legend<n>` / `#dlbls<n>` a chart's category
axis, legend or data labels. Everything but the paragraph is a *container*: a
unit of its own, judged from the text it lays out, whose one property is its
direction. Echo them back; never parse them.

A table is a container in both formats — `a:tblPr/@rtl` in PowerPoint,
`w:tblPr/w:bidiVisual` in Word — and both say the same thing: the cells are
displayed right to left with the file's own cell order unchanged. The
paragraphs inside stay units in their own right, because neither format makes
a cell's text inherit the table's column order, so both are reported
separately. A Word cell's paragraph names its cell in `location.container`:
`table 1 row 2 cell 3`.

A container's direction can be inherited — a Word table style states
`w:bidiVisual` — and is judged the way any inherited value is: silent where it
agrees with the text, reported as an absent one where it contradicts it, with
`evidence.inherited_from` naming the style. The repair still writes to the
table, never to the style.

### `tatweel-padding` reports padding, not tatweel

U+0640 is legitimate. It is the kashida, and a font inserts it during layout
— which is why it never reaches the stored string, and why a paragraph
justified with kashida has nothing in it for this rule to see. A tatweel that
*is* in the string was typed, and only some of those are a defect:

- **Reported** — two or more in a row joined to a letter (a word stretched to
  a width), or one wedged between two characters that already join each other
  (width, and no change to any letter's form).
- **Not reported** — one carrying a harakat, which is the base that mark is
  written on; one at a joining edge, which is how a letter's initial, medial
  or final form is written on its own; and a run joined to nothing at all,
  which is a rule somebody drew.

`evidence.offenders` gives each run as `U+0640 ARABIC TATWEEL ×N @OFFSET`,
the offset being a byte offset into `evidence.logical`. A warning, not an
error: the text renders as arranged, and what is wrong is the string behind
it — search, spell-check and screen readers all read the padding, and it does
not survive a change of width, font or size. `--strip-tatweel` deletes
exactly the runs the finding listed and nothing else.

### The two font checks, which are off by default

`font-coverage` and `shaping-broken` are the only rules that ask a question
about **the machine running mirsam** rather than about the document, so they
run only when `--fonts` asks for them. Every report says which of the two
audits it is — `fonts NOT RUN` in the human output, `"fonts": {"checked":
false}` in JSON — and you must read that before concluding anything from their
silence. A check that did not run is `NOT RUN`, never a pass.

`--font-dir <DIR>` (repeatable) searches those directories instead of the
platform's, which is what makes the result reproducible: a fixed directory
gives a fixed answer on any machine.

**The claim runs one way only.** A font that is *here* and cannot draw the
text will not draw it anywhere — a `cmap` and a `GSUB` travel with the font —
which is what makes a finding worth reporting. A font that is here and draws
the text perfectly proves **nothing** about the reader's machine. Do not turn
silence from these two rules into "the Arabic will render".

They are two defects, not one, and the advice differs:

- `font-coverage` — the font has no glyph for some of the Arabic; it renders
  as empty boxes. `evidence.offenders` is the exact list, by Unicode name.
  An error when the font answers for *none* of the text, a warning when it
  misses part of it.
- `shaping-broken` — the font has every letter and no shaping tables; the
  Arabic renders as disconnected letters. Reported only on the aggregate —
  no joins produced where at least four were required and the font had the
  glyphs — because one letter drawn standalone is not a defect (invariant 7).

Neither is `fixable`. The repair for both is a different typeface, and which
one is an authoring decision; unlike `complex-font-missing`, which fills an
empty slot, these would be overwriting a font the author chose.

### Repairing

`repair <in> <out>` never modifies `<in>` and refuses `<out> == <in>` under
every flag. It changes only what a finding named and copies every other part
across as its original compressed bytes. Its exit code is the audit of
`<out>`, re-read from disk.

The JSON report carries `options`, `repairs.applied`, `repairs.skipped` (a
fix the adapter cannot express yet — listed, never claimed), and `before` and
`after`, each an audit in the same shape `audit --format json` emits.

Four repairs need a decision the text cannot supply, so they are off until
asked: `--font <TYPEFACE>` for `complex-font-missing`, `--convert-bullets`
for `literal-bullet`, `--strip-tatweel` for `tatweel-padding`, and `--align`
for `alignment-unset`. A `literal-bullet` finding in `after` with
`convert_bullets: false` in `options` is not a failed repair; it is one you
did not request. The same goes for a `tatweel-padding` warning with
`strip_tatweel: false`, and for an `alignment-unset` note with `align:
false` — and a note never blocks, so it never changes the exit code.

## Reporting honestly

This is inherited from the project's prior art and is not negotiable.

- Report structural, visual and application QA **separately**.
- Anything not actually run is `NOT RUN`, never inferred.
- `mirsam` proves Unicode and structural correctness. It does **not** prove
  that PowerPoint, Word or a browser renders the file correctly, and it does
  not prove a font is installed on anyone else's machine. Do not claim it does.

## Extending it

Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) first, then
[`docs/PLAN.md`](docs/PLAN.md) for the ordered work items.

**Adding a rule** — implement `Rule` in `crates/mirsam-core/src/rules/`,
register it in `Engine::build`. Nothing else changes. If its repair
needs a choice the text cannot make, the choice is a `RepairOptions` field,
not a CLI flag with logic behind it. A rule that needs to know something about
the *machine* rather than the document takes a port instead, the way
`rules/font.rs` takes a `FontSource` — `RepairOptions` is for authoring
preferences, and putting a source there would make an audit's result depend on
which computer ran it without saying so anywhere.

**Adding a format** — new crate implementing `DocumentReader`; add
`DocumentWriter` only if the format can be faithfully edited in place.

**Resolving a font** — `mirsam-core` never opens one. `ports::FontSource`
takes a family name and answers with bytes, `mirsam-fonts` implements it over
the platform's font directories, and `shape` and `coverage` work on what comes
back; `Engine::with_fonts` is what arms the two rules over it, and
`Engine::with_options` leaves them registered and unrun. A source answering
`None` means this machine has no such font, which those rules treat as
silence: a fact about the computer, not a defect in the deck. Note which way
the claim runs: a font
that is *here* and cannot draw the text will not draw it anywhere, because a
`cmap` travels with the font — but a font that is here proves nothing about
the reader's machine, and no report may suggest otherwise.

**A new adapter must pass the conformance suite unchanged.**
`crates/mirsam-ooxml/tests/conformance.rs` states each situation once, in the
shared model's vocabulary, and runs it against every adapter through
`DocumentReader` — the same situation must come back as the same finding
whichever application wrote the file. No case that asserts what the tool
reports names an element, an attribute or a format, and none may: a case that
had to know which adapter it was looking at is the hexagon leaking, and the
thing to fix is the abstraction, not the case. A format that genuinely cannot
state a situation — Word has no way to write a hard left edge, DrawingML no
way to write a direction-relative one — returns `Inexpressible` with the
reason, and the committed list of those refusals is asserted, so a format that
quietly stopped expressing something fails rather than passes.

**Any change to what is reported or written** shows up in the golden corpus:
`cargo test` compares every `.pptx` and `.docx` under `tests/fixtures/` with
its committed `<document>.expected.json` and fails on any difference. When the
difference is intended, run `make golden`, read the diff, and commit the
regenerated reports with the change that explains them. Never regenerate to
make a failure you do not understand go away. A new document is one file
dropped into that directory plus `make golden`. Two corpus documents may not
share a stem: the report is named for it.

A report for a format the tool reads but cannot write holds the refusal
`repair` gave and the exit code it used, in place of a repair report — so the
day a writer lands, it shows up as a diff on a real document.

**The shaping fixtures are fonts, and they are generated too.** The five
under `crates/mirsam-core/tests/fonts/` are written byte by byte by
`scripts/make-shaping-fixture.py` — no `fontTools`, for the reason the
document generators avoid `python-pptx` — and regenerate with `make fonts`.
Three differ in one thing each: `joining.ttf` carries `init`/`medi`/`fina`,
`nonjoining.ttf` has no `GSUB` at all, `partial.ttf` shapes everything except
the final forms of the right-joining letters. Their glyph order is public and
the tests name exact glyph ids against it, so changing the generator's layout
means changing the tests with it. The fourth, `bold.ttf`, is not a shaping
fixture: it claims the family `Mirsam Joining` a second time, sorts before
`joining.ttf` and shapes nothing, so a font source answering a family name
with whichever file it met first fails a test rather than a user's deck. That
is macOS's Arial in miniature — eleven files state that family, and only
`Arial.ttf` has any Arabic. The fifth, `latin.ttf`, is the other defect
entirely: printable ASCII, no Arabic at all, no `GSUB`, which is Helvetica
under a complex-script slot. It is what `font-coverage` is proved against, and
shaping through it must stay silent — a letter a font has no glyph for says
nothing about whether the font can join.

**The Unicode name table is generated as well.** `charname.rs` comes from
`scripts/make-char-names.py`, which reads the UCD Python itself ships — no
network, no third-party module — and refreshes with `make names`. Never edit
it by hand. It answers `None` outside the four logical-order Arabic blocks
rather than inventing a name, and `coverage::judges` is built on that
refusal: a character the table cannot name is one nothing downstream claims
anything about.

**A corpus document must be one an application opens.** The hand-built ones —
`torture.pptx`, `clean.pptx`, `broken-arabic.pptx` from
`scripts/make-torture-fixture.py`, and `quarterly-review.docx` and
`quarterly-review-correct.docx` from `scripts/make-word-fixture.py` —
regenerate with `make fixtures`, never by editing the package. `cargo test`
asserts the structural invariants of every one of them
(`corpus_packages.rs`: the OPC-level checks run over both formats, the
PresentationML ones over the decks), and `make validate-fixtures` validates
all of them against the published ECMA-376 schemas. A document the application
offers to repair cannot answer "does it open the repaired file without a
prompt", which is the M1 application check.

### Non-negotiable invariants

1. **`mirsam-core` performs no I/O.** If it needs to open a file, the design
   has gone wrong.
2. **A rule that fires on formatting the author chose is a bug.**
   `Resolved::Inherited` is evidence of a choice only where it *agrees* with
   the text: Arabic under a master saying `rtl="1"`, or under a layout that
   centres or right-aligns it, is the layout doing its job and is never
   reported. An inherited value that contradicts the text is a template
   default nobody aimed at the text — an English master's `rtl="0" algn="l"`
   under Arabic — and is reported exactly as an absent one is.
   [ADR 0007](docs/adr/0007-an-inherited-default-is-not-a-choice.md).
   The agreement test is stated for direction and alignment. It has nothing to
   decide for the complex-script font slot, whose rule asks only whether *a*
   font is named, and so nothing there is resolved that could make the tool
   louder; the Latin slot is not inherited at all, because that would
   manufacture `complex-font-missing`'s precondition on text nobody styled.
3. **Repairs are byte-preserving.** Everything a `Fix` does not address passes
   through untouched. The round-trip test guards this; it must stay green.
4. **Never insert bidi control characters.** Direction belongs to the
   container. Never strip ZWJ or ZWNJ — they are meaningful in Arabic and
   Persian orthography.
5. **Never reverse strings or emit presentation forms.** Storage is always
   logical-order Unicode.
6. **Findings carry evidence.** A diagnostic a reviewer cannot verify without
   opening the application is not finished.
7. **A letter a font drew standalone is not a shaping defect.** macOS's Arial
   gives no final form to reh or meem and renders Arabic perfectly: a reh
   only takes a join from its right, and the stroke that makes it belongs to
   the letter before. The only conclusion the evidence supports is the
   aggregate — a font that produced *no* joins in a run that required several
   — and `crates/mirsam-core/tests/fonts/partial.ttf` exists so that a check
   which forgets this fails a test rather than a user's deck.
   [ADR 0008](docs/adr/0008-a-standalone-letter-is-not-a-shaping-defect.md).

## Before pushing

```bash
make verify     # version check, fmt, clippy -D warnings, tests, build
```

The `pre-push` hook runs this; enable it with `make hooks-install`.
