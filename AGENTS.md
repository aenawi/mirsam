# AGENTS.md — mirsam

Instructions for AI coding agents (Claude, Codex, Grok, OpenCode, Pi, …) using
or extending this tool.

## What this is

**mirsam** is an independent Rust CLI that audits and repairs Arabic
right-to-left, bidirectional and typography defects in documents. Single static
binary: no Python, no Node, no Office, no runtime of any kind on the target
machine.

`audit` reads `.pptx`, `.docx`, `.xlsx` and `.html`/`.htm`. `repair` writes
`.pptx`, `.docx` and `.xlsx`: `.html` is refused as a readable format without a
writer, which is a usage error (exit `2`) and not a claim that the document was
not understood.

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

A table is a container in every format that has one — `a:tblPr/@rtl` in
PowerPoint, `w:tblPr/w:bidiVisual` in Word, `sheetView/@rightToLeft` on an
Excel worksheet — and they say the same thing: the cells are displayed right
to left with the file's own cell order unchanged. The paragraphs inside stay
units in their own right, and are reported separately. A Word cell's paragraph
names its cell in `location.container`: `table 1 row 2 cell 3`.

**Excel is the exception to "separately", and it is the format's rule rather
than the adapter's.** PowerPoint and Word do not make a cell's text inherit its
table's column order; `rightToLeft` is *also* the reading order every cell in
the sheet falls back to, so an Excel cell that states no `readingOrder` of its
own comes back `Inherited` naming the worksheet.

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

### XLSX, where a cell is a paragraph and the sheet is the table

A worksheet's `sheetView/@rightToLeft` decides which side column A sits on,
which is what `a:tblPr/@rtl` and `w:tblPr/w:bidiVisual` say in the other two
formats — so **a sheet is a `Table` container** and carries `<part>#tbl1`,
while each cell is a paragraph carrying `<part>#p<n>`. A unit's
`location.container` is the reference a person would type into the Name Box:
`الأرقام!C2` for a cell, `sheet "الأرقام"` for the grid.

Five things to know before reading a workbook's report:

- **The grid is reported only where the sheet has two or more columns of
  text.** With one column there is no question of which column a reader starts
  in, so there is nothing for `container-direction` to be about. This is the
  same threshold a `Columns` container has at `numCol >= 2`.
- **A cell inherits the sheet's direction, and that is Excel's own rule rather
  than an exception.** In PowerPoint and Word a cell's text does not inherit
  its table's column order; `rightToLeft` is also the reading order every cell
  in the sheet falls back to, so a cell stating no `readingOrder` comes back
  `Inherited` naming the worksheet part, and ADR 0007 judges it from there.
- **`readingOrder="0"` and `horizontal="general"` both come back unset.** The
  first is *context dependent* — the same guess-from-the-first-strong-character
  `dir="auto"` asks for. The second is Excel's default chosen by the cell's
  **data type**, text at the start edge and numbers at the end, which is not an
  edge anybody stated. Neither is a decision, so neither silences a rule.
- **`sources.unread` names formula cells, and it is a `NOT RUN`.** A `<c><f>`
  stores a *cached result* that Excel recomputes on open, so it is not the
  document's text and produces no unit. One whose cache carries Arabic is named
  — `الأرقام!D3` — so its absence from the findings can never look like a clean
  reading. One holding no Arabic is not named: there is nothing there this tool
  would have judged.
- **`language-missing` fires on every Arabic cell of an untagged workbook, and
  cannot be repaired.** SpreadsheetML has no language slot on a cell, a run, a
  font or a style; `docProps/core.xml`'s `dc:language` is the only place a
  workbook states one, so that is what every cell inherits — and `SetLanguage`
  is refused, because one tag answers for the whole file and writing it to
  satisfy the Arabic would relabel the English beside it. It shows in
  `repairs.skipped`, never as a repair that failed.

`complex-font-missing` never fires on a workbook, for HTML's reason: a cell has
one font and it draws every script, so a filled Latin slot beside an empty
complex-script one is not a document Excel can write. `literal-bullet` fires
and `--convert-bullets` is refused, because Excel has no list to convert to.

**Repairing a workbook appends rather than edits, and that is not an
optimisation.** `<c s="3"/>` names a record in `cellXfs` that forty cells may
share, so the repaired alignment is a *copy* of that record with the cell's
`@s` repointed at it; every other cell keeps the record it had, byte for byte.
Shared strings work the same way — a repaired string is a new `<si>`, cloned so
its rich-text runs survive, with the old one left for whatever else was
pointing there. `<f>` formulas and `xl/workbook.xml`'s `<definedNames>` come
through untouched, and mechanically so: `xl/workbook.xml` is not a part a
repair plan can name, and a cell repair edits `@s` or the content of a `<v>`
and nothing beside it.

### Repairing a Word document, where two things are refused

A `.docx` repair writes `w:pPr/w:bidi`, `w:pPr/w:jc`, `w:rPr/w:lang@w:bidi`,
`w:rPr/w:rFonts@w:cs`, `w:pPr/w:numPr` and `w:tblPr/w:bidiVisual`, and every
one of them lands in its schema-sequence position — WordprocessingML's
property elements are `xsd:sequence`, so a correct element written in the
wrong place is a document Word offers to repair.

Three things about it are worth knowing before reading a repair report.

- **`w:jc` is written relative, so no direction is resolved to write one.**
  The PowerPoint writer has to know the direction a paragraph inherits before
  it can put a `Start` down, because `a:pPr/@algn` names physical edges. Word's
  does not: `start` is the start edge whichever way the paragraph runs. The
  same fact is why **a physical edge is refused** — `SetAlignment(Left)` and
  `SetAlignment(Right)` appear in `repairs.skipped`, never as a repair that
  failed, because Word has no spelling for "the left of the page whatever the
  direction". No Word unit ever produces one, so this is a refusal you will not
  see in practice; it is stated so the writer cannot start inventing one.
- **`--convert-bullets` needs a list the document already defines.** Word has
  no per-paragraph bullet: a list is a `w:numPr` pointing into the numbering
  part. The repair points at a bulleted definition the document already
  carries, preferring one that draws the marker the author typed, and
  otherwise the fix is **skipped**. A repair cannot add the numbering part
  itself — that is a part, a content-type override and a relationship, none of
  which is an edit to the paragraph the finding named.
- **`--font` clears `@w:cstheme` on the runs it fills.** `@w:cstheme` is what
  Word renders and `@w:cs` the value it caches beside it, so writing the
  typeface into the cache and leaving a theme reference standing would change
  what the file says and not what a reader sees.

A paragraph inside a text box is a unit of its own and is repaired as one; the
paragraph anchoring the box is not repaired through it, and an `mc:Fallback`
is neither counted nor edited — the same two regions the reader steps over.

### HTML, where the direction is usually not in the document

An `.html` page states direction in three places and the adapter reads all of
them: `dir`, a `style` attribute, and CSS — from `<style>` elements and from
`<link rel="stylesheet">` whose `href` is a **relative path on this machine**,
resolved against the document. Selectors, specificity and `!important` are
resolved as a browser resolves them, so `dir="rtl"` under a stylesheet saying
`direction: ltr` is reported as **ltr**: the tool reports what a reader sees.

Three consequences worth knowing before you read a report:

- **`sources.unread` is a `NOT RUN`, and it is in every report.** A stylesheet
  named by an absolute URL is not fetched — mirsam performs no network I/O —
  and one named by a root-relative path (`/css/site.css`) cannot be resolved
  without a document root. Rules in a sheet that was not read are rules nobody
  applied, so a `direction-unset` finding on such a page **may be answered by
  CSS the tool never saw**. Read `"sources": {"unread": [...]}` before
  concluding anything from an absent value. It is `{"unread": []}` for every
  `.pptx` and `.docx`, because a package holds everything that decides its own
  text; an `.xlsx` fills it with the formula cells whose cached result the tool
  did not judge
  ([ADR 0009](docs/adr/0009-a-source-the-adapter-could-not-read-is-part-of-the-report.md)).
- **`dir="auto"` comes back as unset, and that is not harshness.** `auto` asks
  the browser to guess from the first strong character, which is what `Unset`
  means. It gets `مرحبا 2026` right and `2026 مرحبا` wrong, so a paragraph is
  correct under it by luck of today's text. A **warning**, the fragile tier.
- **`complex-font-missing` never fires on HTML, and that is correct.** CSS
  gives an element one `font-family` for every script, not OOXML's Latin and
  complex-script pair, so a filled Latin slot beside an empty Arabic one is a
  document the web cannot write. The mirror holds too: CSS's `left` and
  `right` *are* physical edges, so `alignment-incoherent` is live on HTML in a
  way it is not on Word.

A unit id is `<file>#p<n>` / `#tbl<n>` / `#cols<n>`, the same shapes the OOXML
adapters issue, and `location.part` is the file's own name. A `Columns`
container is an element CSS lays out in two or more columns — or a flex
container that displays its boxes in the reverse of the order it stores them,
which is boxes side by side by another property.

### The four rules only the web can currently trigger

They are ordinary rules in `mirsam-core`, registered like every other one, and
they read vocabulary the shared model gained in M5: `TextUnit::spans`,
`Properties::inset` and `Properties::reversed`. OOXML can state none of it, so
the conformance suite carries four refusals rather than four skips, and these
rules are structurally silent on `.pptx` and `.docx` in the way
`complex-font-missing` is structurally silent on HTML. **Do not read that
silence as coverage that is missing.**

- **`bidi-override`** — `<bdo>`, or `unicode-bidi: bidi-override`, is the
  markup form of an embedded U+202E, and `bidi-control` reports the character
  version of the same defect. An **error** when the imposed order differs from
  what that run resolves to under the direction the override itself names —
  both renderings are in the evidence — and a **warning** when they agree,
  which is the fragile tier: nothing has moved today, and the algorithm is
  still off for the first digit or Latin word typed there tomorrow.
- **`isolation-missing`** — the `<bdi>` rule. Reported only for a run carrying
  a **strong** character of the direction opposite to the paragraph's, and only
  when isolating that run changes the paragraph's resolved order — the two
  orders are the evidence. A **warning**, and not because the tool is unsure:
  nothing is mis-rendered, the algorithm did what it is specified to do. What
  is wrong is that the order of text *outside* the run is decided by text
  *inside* it, so the line is laid out correctly for today's content and
  undefined for tomorrow's.
- **`inset-physical`** — `margin-left` / `padding-left` where
  `margin-inline-start` was meant, on right-to-left text. Only an *asymmetric*
  inset: equal on both sides is a page gutter and is direction-neutral.
  Reported **wherever it was stated**, unlike `alignment-incoherent`, and the
  distinction is deliberate — nothing fills in an absent inset, so a left inset
  under Arabic exists because somebody wrote it, and `evidence.inherited_from`
  names the declaration.
- **`order-reversed`** — `flex-direction: row-reverse` on a flex container:
  right-to-left appearance produced by reversing the boxes instead of stating
  the direction. Reported whichever direction is declared, because both are
  wrong and wrong differently. **A hand-reordered source that states nothing is
  invisible here, and that is a limit rather than an omission** — a container of
  Arabic in some order carries no evidence of which order was intended, and
  claiming otherwise would be asserting a defect rather than proving one.

Neither `inset-physical` nor `order-reversed` is `fixable`: the repair is an
edit to markup or to a stylesheet, not a value the shared `Fix` vocabulary can
name, and no adapter in this build can make it.

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

**Adding a format** — a new crate implementing `DocumentReader`, or a new
module in an existing adapter crate where the *container* really is shared:
XLSX lives in `mirsam-ooxml` beside PPTX and DOCX because it reuses `package`
and `token` and names no element either of them does. Add `DocumentWriter` only
if the format can be faithfully edited in place.

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
`crates/mirsam-conformance/tests/conformance.rs` states each situation once, in the
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
`cargo test` compares every `.pptx`, `.docx`, `.xlsx` and `.html` under
`tests/fixtures/` with
its committed `<document>.expected.json` and fails on any difference. When the
difference is intended, run `make golden`, read the diff, and commit the
regenerated reports with the change that explains them. Never regenerate to
make a failure you do not understand go away. A new document is one file
dropped into that directory plus `make golden`. Two corpus documents may not
share a stem: the report is named for it.

A report for a format the tool reads but cannot write holds the refusal
`repair` gave and the exit code it used, in place of a repair report — so the
day a writer lands, it shows up as a diff on a real document. That is what the
two `.docx` reports did when M3's writer landed, and `.html` is what carries
the refusal now.

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
`scripts/make-torture-fixture.py`, `quarterly-review.docx` and
`quarterly-review-correct.docx` from `scripts/make-word-fixture.py`, and
`quarterly-figures.xlsx` and `quarterly-figures-correct.xlsx` from
`scripts/make-excel-fixture.py` — regenerate with `make fixtures`, never by
editing the package. `cargo test` asserts the structural invariants of every
one of them (`corpus_packages.rs`: the OPC-level checks run over every packaged
format, the PresentationML ones over the decks), and `make validate-fixtures`
validates
all of them against the published ECMA-376 schemas. A document the application
offers to repair cannot answer "does it open the repaired file without a
prompt", which is the M1 application check.

**The two HTML corpus documents are the exception, and edited by hand.** A
page *is* its own source: there is no package to build, so `make fixtures` has
nothing to generate and `scripts/validate-ooxml.py` has nothing to validate.
`quarterly-page.html` carries one instance of each defect the adapter can see;
`quarterly-page-correct.html` is the page the tool must leave completely alone.
Change either and run `make golden`.

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
