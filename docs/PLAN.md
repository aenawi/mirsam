# Implementation plan

Work items in dependency order. Each is small enough to land as one PR and
carries its own acceptance test. Milestones are defined in
[`ROADMAP.md`](ROADMAP.md); this file is how they get built.

Status legend: `[x]` done · `[ ]` ready · `[~]` blocked on the item above.

---

## M0 — Foundation `[x]`

- [x] Cargo workspace, edition 2024, three crates, pinned toolchain
- [x] `Resolved<T>` three-state property model
- [x] `TextUnit` / `Properties` / `Location` / `UnitId`
- [x] UAX#9 resolution: `resolve`, `auto_direction`, `dominant_direction`, `order_differs`
- [x] Arabic script detection; presentation forms; bidi controls preserving ZWJ/ZWNJ
- [x] `Diagnostic` / `Evidence` / `Report` with severity ordering
- [x] `Rule` trait + `Engine`; eight default rules
- [x] `DocumentReader` / `DocumentWriter` ports
- [x] PPTX reader over a `quick-xml` token stream
- [x] `audit`, `explain`, `rules`; text and JSON renderers; stable exit codes
- [x] 22 tests; clippy `-D warnings` clean

---

## M1 — Repair `[ ]`

The order matters: the round-trip guarantee must exist *before* the first
mutation, or there is nothing to prove the mutation didn't damage anything.

Every work item below has landed. The milestone's application check —
`ROADMAP.md`'s "PowerPoint opens the result without a repair prompt" — cannot
be proven by a test. A person ran it on 2026-09-03
([#6](https://github.com/aenawi/mirsam/issues/6)): the repaired
`quarterly-report.pptx`, built on PowerPoint's own template, opens without a
prompt; `torture.pptx` prompted *before* any repair, so the check was
inconclusive there ([#9](https://github.com/aenawi/mirsam/issues/9)). The
cause was the fixture, not the tool: all three hand-built decks were invalid
against the ECMA-376 schema — every `p:spTree` was missing the `p:grpSpPr`
the schema requires, the theme carried only a font scheme, the bar chart had
no axes, the notes slide had no notes master, and `clean.pptx` had
relationships to parts that were not in the package. They were regenerated and
validated, and `clean.pptx` and `broken-arabic.pptx` stopped prompting;
`torture.pptx` did not. A 23-deck bisect run by a person on PowerPoint 2016
on 2026-09-04 — first the skeleton plus one hazard each, then the whole
deck minus one suspect each, then the media part under every name and
encoding — named the last cause exactly: **PowerPoint 2016 does not
resolve a relationship to a part whose name carries a non-ASCII octet.**
Raw UTF-8 or percent-encoded, in the item name or the `.rels` target or
both, Arabic or a single Latin letter, it prompts to repair and a picture
shape pointing at the part shows "The picture can't be displayed". The
same PNG under an ASCII name opens clean, and so does a percent-encoded
ASCII name (`my%20image.png`), so percent-encoding itself is fine and
the schemas were never going to see this — the decks all validated. The
torture deck now carries the encoded-space name instead; the byte-copying
of names is still exercised, and a non-ASCII part name is not a hazard a
deck an application opens can carry. The same pass confirmed by eye that a
`SetDirection` repair turns the torture deck's left-to-right title
right-to-left, that `a:tblPr/@rtl` mirrors the corpus table's columns, and
that an RTL paragraph with no alignment sits on the left edge exactly where
`alignment-unset` says it does. The regenerated `torture.pptx` and its
repaired copy were then opened on the same machine: neither prompts, the
first shows its seeded left-to-right title and the second shows it
right-to-left. **The M1 application check is verified on every deck in
the corpus**, on PowerPoint 2016 / Windows 10. The pass also found chart axis labels with no text
properties at all, which no rule sees — a container of the same family as
tables, tracked separately. The same session saw that
the repaired deck's Arabic paragraphs keep the template's left alignment,
which the audit did not report —
[#8](https://github.com/aenawi/mirsam/issues/8), the first visual evidence
for what M2 has to resolve. Answered from the text alone by
`alignment-unset` and `repair --align`
([ADR 0006](adr/0006-judge-from-the-text-not-the-template.md)); the
layout-aware answer is 2.2.

### 1.1 Round-trip harness `[x]`
`mirsam-ooxml::package` opens a package once and rewrites it entry by entry,
copying every untouched entry's **already-compressed** bytes across verbatim
(`raw_copy_file`) rather than decompressing and re-deflating it. Only a part
carrying an edit is decoded and re-encoded.

*Acceptance:* met. `tests/fixtures/torture.pptx` carries `mc:AlternateContent`
with `mc:Ignorable` naming a prefix, an embedded chart, the chart's `.xlsx`
workbook (a ZIP inside the ZIP), speaker notes, a percent-encoded item name
(`ppt/media/my%20image.png`, which a rewriter that decodes names would
silently rename), and four compression settings across its 25 entries. It
carried an Arabic part name until #9 showed PowerPoint 2016 does not open a
package with a non-ASCII part name in any encoding; see the M1 preface.
It is generated by `scripts/make-torture-fixture.py` with Python's `zipfile` —
deliberately not the `zip` crate under test, so the assertion is not circular.

It is also a *document*: every part declared in `[Content_Types].xml`, every
relationship resolving, a notes master behind the notes slide, a complete
theme, a chart with its axes. `corpus_packages.rs` asserts that of every deck
in the corpus on every `cargo test`, and `make validate-fixtures` validates
them against the published ECMA-376 transitional schemas — the strongest claim
a machine can make about "an application would open this". That check is well
calibrated against this corpus: the two decks built on PowerPoint's own
template pass it untouched, and every defect it reported on the hand-built
three was real ([#9](https://github.com/aenawi/mirsam/issues/9)).

*Scope of the guarantee, stated exactly.* For every entry the rewrite did not
edit, the name (as text and as stored bytes), package position, compression
method, timestamp, CRC-32, both sizes and the raw compressed bytes are
identical. Whole-file byte equality is **not** claimed: the `zip` crate
normalises three container fields — `version needed to extract` (20 → 10 on
stored entries), the `version made by` host byte (MS-DOS → Unix) and the
external attributes that follow it. All three are spec-valid, none can alter a
document's content, and no public API exposes them. Achieving literal
whole-file equality would mean hand-rolling a ZIP writer; that trade is
recorded here rather than hidden behind a weaker test.

Both properties are mutation-tested: replacing `raw_copy_file` with a
decompress-recompress cycle fails on `compressed size changed — the entry was
re-encoded`, and corrupting an untouched part fails on `CRC-32 changed`.

### 1.2 `Fix` application in the token stream `[x]`
`mirsam-ooxml::rewrite` reads a part into an event vector, edits what a repair
names, and writes it back. Token round-trip is byte-identical on every part of
the acceptance deck, so "and nothing else" is a property with a test under it.

- [x] `SetDirection` → `a:pPr/@rtl`, creating `a:pPr` in schema position
- [x] `SetAlignment` → `a:pPr/@algn`, lowering the direction-relative value
      against the paragraph's resolved direction
- [x] `SetLanguage` → `a:rPr/@lang`, `a:defRPr/@lang`, `a:endParaRPr/@lang`,
      creating `a:rPr` for a run that has none
- [x] `SetComplexFont` → `a:cs/@typeface`, by rank
- [x] `RemoveControls` → offsets applied back-to-front
- [x] `ConvertLiteralBullet` → strip the glyph, add `a:buChar`, set
      `marR`/`indent`
- [x] `NormalizePresentationForms` → each run's text through
      `script::normalize_presentation_forms`, one character at a time; runs
      without a form are not rewritten. The mapping itself lives in core, per
      [ADR 0005](adr/0005-presentation-forms-via-unicode-normalization.md).

*Acceptance:* met for all seven. Each has a test asserting the **entire**
rewritten part, so any unintended byte — a re-quoted attribute, a resolved
character reference, a moved child — fails.

*What the seventh needed, and why it waited.* Mapping presentation forms to
logical codepoints is NFKC, and `mirsam-core` had no dependency that could do
it. Core's dependency count is an architectural constraint, so adding one was
an ADR. The measured cost of `unicode-normalization` turned out to be small —
two tiny transitive crates, about 125 KB — and the real hazard was elsewhere:
NFKC over a whole run also composes canonical pairs the author typed, expands
compatibility characters of other scripts in the same run, and expands word
ligatures such as ﷺ into whole phrases. The adapter therefore never
normalises a string; core maps one flagged character at a time, and the word
ligatures U+FDF0–U+FDFF are reported as a warning and left as written.

*Third trap, found by looking.* `is_presentation_form` was a range check
over the two blocks, which also matched U+FEFF, the ornate parentheses, the
pedagogical symbol dots and sixty unassigned codepoints — forty-one assigned
characters no normalisation can change. A run with a stray byte-order mark
was reported as pre-shaped text and marked fixable; the repair would have
applied, changed nothing, and the after-audit would have reported it again.
The predicate is now "the repair will change this character", so the rule
and the repair cannot disagree.

*Trap, confirmed real.* Attributes are spliced in their raw bytes rather than
rebuilt, so editing `rtl` next to `algn='l'` leaves the single quotes alone.
Children are inserted by rank against the `CT_TextParagraphProperties` and
`CT_TextCharacterProperties` sequences: `a:cs` lands between `a:latin` and
`a:sym`, `a:buChar` after `a:buFont`.

*Second trap, found the hard way.* quick-xml reports a character or entity
reference as its own event, separate from the text around it. The scanner read
only `Event::Text`, so a run written as `&#1585;&#1587;&#1605;` — an ordinary
way for Office to store Arabic — came through empty, and an empty run is
dropped. Such paragraphs produced no unit and therefore no finding, in a tool
whose entire purpose is to reason about that text. Both the scanner and the
rewriter now read `Event::GeneralRef` as content.

### 1.3 `repair` command `[x]`
`mirsam repair <in> <out>` with `--lang`, `--font`, `--convert-bullets`,
`--force`; refuses `in == out`; re-audits the output and reports both.

*Acceptance:* met. `crates/mirsam-cli/tests/cli.rs` repairs the M0 fixture
through the binary with `--convert-bullets` and asserts that the after-audit
is empty, that `audit --strict` on the written file exits `0`, and that a
second `repair` of that output stages nothing and reproduces it byte for byte.
`crates/mirsam-ooxml/tests/writer.rs` proves the same fixed point at the port
level on the torture deck, and that a repair naming one paragraph leaves every
other entry's compressed bytes untouched.

*Shape.* `PptxDocument` implements `DocumentWriter`. `apply` groups repairs by
part and then by paragraph, rewrites each part once, and stages nothing unless
every part succeeds; `write` hands the staged parts to `Package::rewrite`,
which copies everything else raw. The port grew one default method,
`supports`, so a fix an adapter cannot express is reported as *not applied*
while the rest of the deck is still repaired — one paragraph the adapter
cannot handle must not stop the other forty from being fixed, and must not be
reported as fixed either. (Until 1.2's last variant landed, that was
`NormalizePresentationForms`; the PPTX adapter now expresses every variant,
and the mechanism stays for the next adapter.)

*The options belong to the engine, not the CLI.* `--lang`, `--font` and
`--convert-bullets` populate `RepairOptions`, which configures the rules that
propose those fixes; `Engine::with_default_rules` now delegates to
`Engine::with_options`. `--font` is the only way `complex-font-missing`
becomes fixable, because choosing a typeface is an authoring decision.
`--convert-bullets` is opt-in because it is the one fix that edits text
rather than the properties around it. `--lang` is validated by the same
predicate the rule checks with, so `repair` can never write a tag its own
re-audit reports. `--force` replaces an existing output and never extends to
the input.

*Exit code* follows the after-audit, exactly as `audit` would judge the
written file: `0` clean, `1` blocking, `--strict` promoting warnings. Every
refusal is `2`.

*Trap, found on the way.* `Fix` was declared internally tagged for serde,
which cannot carry a newtype variant whose payload is a string or a list:
`SetDirection(Rtl)` serialised as `{"kind":"set_direction","rtl":null}` and
`RemoveControls` failed outright. Nothing emitted it until
`repair --format json` did. It is adjacently tagged now, like `Resolved<T>`.

*Second trap.* Lowering `SetAlignment(Start)` consulted only the paragraph's
own `rtl`, so a left-aligned paragraph inheriting its direction from the body
lowered `Start` to the left edge and reproduced the defect it was repairing.
The writer now passes what the scanner resolved as inherited into the
rewriter. While there: `RemoveControls` is applied before
`ConvertLiteralBullet` whatever order the plan gives them, since stripping the
marker shifts every offset the controls were found at.

### 1.4 Golden corpus `[x]`
Real decks under `tests/fixtures/`, each with a committed expected report.

*Acceptance:* met. `crates/mirsam-cli/tests/golden.rs` treats every `.pptx`
under `tests/fixtures/` as a corpus deck and compares, byte for byte, its
committed `<deck>.expected.json` against what the binary does to it now: the
`audit --format json` report; the `repair --format json` report under
`--convert-bullets --font Dubai`, so every fix the adapter can express is
exercised and the one it cannot is recorded as skipped; a tag-level diff of
every package entry the repair changed; and the exit codes of `audit`,
`repair`, and both under `--strict`. A deck without a report fails, and so
does a report without a deck. Reports are regenerated with
`MIRSAM_UPDATE_GOLDEN=1 cargo test -p mirsam-cli --test golden` (`make
golden`), which refuses to run under `CI` — so the only way a diff reaches
`main` is inside the commit that explains it. The suite also asserts, against
the binary rather than the file names, that at least one deck is left
completely alone and that at least one carries `mc:AlternateContent`, the
structure the roadmap says the corpus must include.

Five decks. `clean.pptx` and `quarterly-report-correct.pptx` are left
completely alone: no finding at any severity, nothing applied, nothing
skipped, no entry changed. `broken-arabic.pptx` and `torture.pptx` are the M0
and 1.1 fixtures, enrolled. `quarterly-report.pptx` is the first deck shaped
like a real one: six slides on python-pptx's default template — a genuine
PowerPoint theme with a master, eleven layouts and their English prompt text —
carrying every defect the rule set knows across placeholders, a text box, a
grouped text box, a table and speaker notes, beside a correct slide and an
English one. It reports 3 errors, 19 warnings and 17 notes; repair under
`--convert-bullets --font Dubai --align` applies 39 fixes and skips none,
and the written deck audits clean under `--strict`. (Until 1.2's last
variant landed it applied 19 and skipped the presentation-forms paragraph;
until 1.5 its table was already right-to-left. Each report said exactly
what the tool did at the time.)
`quarterly-report-correct.pptx` is the same deck authored properly. Both come from `scripts/make-corpus.py` (`make corpus`),
which is deterministic: re-running it reproduces the committed bytes.

*What "real" means here, stated honestly.* No deck in the corpus was captured
from the wild. The two generated ones are built on PowerPoint's own template
with the attribute habits PowerPoint has, but their slide content came from
python-pptx and this script. The harness enrols any `.pptx` dropped into the
directory, so a deck from a real author is one file and one `make golden`
away. The script can also re-save the deck through LibreOffice Impress
(`--impress`) for a second application's dialect; that needs Impress
installed, which was not available where this landed, so it is not part of
the committed corpus.

*Observation, for M2.* The scanner audits every `ppt/**/*.xml` part, so the
template's layouts and master contribute a hundred units of prompt text
("Click to edit Master title style") to a six-slide deck — 126 units scanned
(one of them the table), 24 of them Arabic. They are English and report nothing, but a template with
Arabic prompts would be reported at the layout, not the slide. 2.1's
relationship graph is what will know a layout from a slide.

### 1.5 Container direction: tables and columns `[x]`
A table's direction is its own — `a:tblPr/@rtl`, which decides whether the
first column sits on the right or the left — and no paragraph rule can see
it. Found by a person looking at the repaired corpus deck after #8.

*Shape.* `TextUnit` gained a `kind`: a container is a unit beside the
paragraphs inside it, its text the text it lays out, its direction its own.
`Rule::applies_to` says which kind a rule judges, so the paragraph rules
never see a container and `container-direction` never sees a paragraph. The
adapter issues `<part>#tbl<n>` ids and the rewriter sets `a:tblPr/@rtl`,
creating `a:tblPr` first in `CT_Table`'s sequence when absent; any other fix
on a table is refused.

*Acceptance:* met. The corpus's broken deck carries a table with no
direction; the report shows `container-direction` on `slide3.xml#tbl1`, the
repair adds `rtl="1"` to the existing `tblPr` in place, and the correctly
authored twin, whose table is right-to-left, reports nothing. Rewriter tests
assert the whole part for creation, in-place edit, numbering and the
composition with a cell's own repair.

*Columns, the second container* ([#12](https://github.com/aenawi/mirsam/issues/12)).
A text body with `a:bodyPr/@numCol` ≥ 2 flows its columns left-to-right
unless `@rtlCol="1"`, which for Arabic text starts the reader in the wrong
column — the same defect as a table's, one attribute along. It is
`UnitKind::Columns`, id `<part>#cols<n>`, text every enclosed paragraph's
text, direction the body's `rtlCol`; the repair sets that one attribute and
any other fix on a body is refused. A single-column body is deliberately not
a unit of this kind: `rtlCol` on one column changes nothing a reader sees,
so there is nothing to judge and nothing to repair.

*One rule, not two.* `table-direction` became `container-direction`, applying
to every kind that is not a paragraph. The judgement does not vary with the
container — mostly-Arabic text, direction unset or contrary, warning with a
`SetDirection` fix — only the attribute an adapter lowers the repair onto
does, and that is the adapter's business. The finding still names what it
found: a table's says its columns run the wrong way, a body's that they flow
the wrong way. The rule id moved in every committed report; the messages did
not.

*The ordinal counts every body, not every columned one.* `#cols<n>` is the
nth `a:bodyPr` in the part, exactly as `#p<n>` counts every `a:p` including
the ones that produce no unit. A numbering that skipped the bodies the tool
has nothing to say about would drift the moment a deck carried one, and the
repair would land on the wrong shape.

*Corpus.* `clean.pptx` and `broken-arabic.pptx` gained the same two-column
box, correct in one (`numCol="2" rtlCol="1"`) and without a column direction
in the other. The paragraphs inside are identical and correct in both, which
is the point: a container's direction is not its paragraphs'. `clean.pptx`
is still reported on at no severity at all.

### 1.6 Container direction: chart text `[x]`
The third container, and the first whose text is not paragraphs at all
([#18](https://github.com/aenawi/mirsam/issues/18)). A chart's category
labels and series names come from `c:strCache/c:pt/c:v` — cached strings —
and the direction they are drawn in belongs to the container that draws
them: `c:catAx/c:txPr`, `c:legend/c:txPr`, `c:dLbls/c:txPr`, each an
`a:bodyPr` + `a:lstStyle` + one `a:p` whose `a:pPr/@rtl` governs every
string in it. Found by a person: during the #9 application check the torture
deck's Arabic axis labels rendered with no direction selected at all, and
`audit` said nothing, because the only `a:p` in the chart part is its title.

*Shape.* `mirsam-ooxml::chart` is a second pass over a part whose root is
`c:chartSpace`; it reads only as far as that root on every other part, so
the adapter can hand it everything. `UnitKind::ChartText`, ids
`<part>#catax<n>` / `#legend<n>` / `#dlbls<n>`, the ordinal counting the
elements of that kind in the part. The rewriter sets `a:pPr/@rtl` inside the
container's `c:txPr`, creating the whole `c:txPr` in schema position when
there is none — which is the usual case, since most generated charts have
none — and bringing an `xmlns:a` declaration with it in the rare part that
declares no DrawingML prefix. Any other fix on such a container is refused.

*What each container draws is read from the file, never assumed.* This is
the whole difficulty: a finding on strings a container does not draw is a
false positive on text the reviewer cannot even find. A category axis draws
the categories of the chart group that names it in `c:axId`; a legend draws
its series' names, or its categories when the chart is a pie, where the
legend lists the slices; data labels draw whichever of those their
`c:showCatName` / `c:showSerName` flags turn on, and nothing when they show
only values, which are numbers. An axis *title* has a `c:txPr` of its own,
and reading that as the axis's would silence a real finding — so the
container's `c:txPr` is taken only when it is a direct child.

*Deliberately not covered, and why.* A value axis draws formatted numbers,
which have no direction to get wrong. The chart-space-level `c:txPr` is a
default for text other containers draw rather than a container that draws
strings of its own. Neither can be given text to show a reviewer, so neither
is a unit.

*Acceptance:* met for the machine half. `torture.pptx` already carried the
broken shape: its report now shows `container-direction` on
`chart1.xml#catax1` with the four cached quarter names as evidence, the
repair creates a `c:txPr` carrying `rtl="1"` between `c:axPos` and
`c:crossAx`, and `make validate-fixtures` accepts the repaired chart part
against the ECMA-376 schemas. Rewriter tests assert the whole part for
creation in each of the three containers, for editing a direction already
there, for a `c:txPr` that has no `a:pPr`, for per-kind numbering, and for
the namespace declaration.

- [ ] **Application check, `NOT RUN`.** The issue's last criterion is a
      person opening the repaired deck and seeing the labels laid out
      right-to-left. That cannot be proven by a test, and it has not been
      run. Schema validity is not the same claim.

---

## M2 — Inheritance `[ ]`

### 2.1 Package relationship graph `[ ]`
Parse `_rels/*.rels`; resolve slide → layout → master.

### 2.2 Property chain resolution `[~]`
Walk paragraph → placeholder (`p:ph/@type`,`@idx`) → layout → master → theme,
populating `Resolved::Inherited` instead of `Unset`.

*Acceptance:* a deck with direction set only on the master reports zero
`direction-unset` warnings — and the same deck reports them today, so the test
is written first and starts red. And, from #8: a right-to-left paragraph a
layout centres reports no `alignment-unset` note, while one an English
layout leaves on the left edge reports it as a finding rather than a note.
What to conclude from a master whose own body style says `rtl="0"` is this
item's ADR to write.

### 2.3 List levels and theme fonts `[~]`
`lvl1pPr`…`lvl9pPr` by `a:pPr/@lvl`; `a:fontScheme` for the `cs` slot.

---

## M3 — Word `[ ]`

### 3.1 Extract the shared package layer `[~]` *(partly done in 1.1)*
ZIP access, part enumeration and the byte-preserving rewrite already live in
`mirsam-ooxml::package` — 1.1 needed them there, because a writer that reads
the package through a second code path is a writer whose round-trip guarantee
covers only half of what ships. `pptx.rs` is DrawingML vocabulary plus its
paragraph scanner.

What remains for M3 is the *token-rewrite* scaffold, which 1.2 will build
against DrawingML first. Generalise it by extraction once it works, not by
anticipation — the shape is only known from a working adapter.

### 3.2 DOCX reader `[~]`
`w:p`, `w:pPr/w:bidi`, `w:jc`, `w:lang/@w:bidi`, `w:rFonts/@w:cs`.

### 3.3 Style-chain inheritance `[~]`
`docDefaults` → linked styles → direct formatting.

### 3.4 Tables `[~]`
`w:bidiVisual` only where semantic reading order is RTL.

### 3.5 Conformance suite `[~]`
One suite both adapters run unchanged. If DOCX needs a core change to pass, the
abstraction was wrong — fix the abstraction, not the test.

---

## M4 — Shaping `[ ]`

### 4.1 `rustybuzz` shaping `[ ]`
Shape each Arabic run; assert joining forms are produced.

### 4.2 Font coverage `[~]`
`ttf-parser` over the resolved font; report missing codepoints by name.

### 4.3 `shaping-broken` and `font-coverage` rules `[~]`

*Acceptance:* a deck using a Latin-only font for Arabic is reported with the
exact characters that will not render.

---

## M5–M6 — Web, spreadsheets, PDF `[ ]`

Adapters only; no core changes expected. If one is needed, record an ADR
explaining what the core got wrong.

PDF implements `DocumentReader` **only**.

---

## M7 — Distribution `[ ]`

- [ ] `cargo-dist` or a release workflow producing five static targets
- [ ] Publish `mirsam-core`, `mirsam-ooxml`, `mirsam-cli` to crates.io
- [ ] JSON schema, versioned, with a compatibility test
- [ ] SARIF renderer
- [ ] Agent skills: one `SKILL.md` per format over one binary

---

## Standing rules

1. **The round-trip test is sacred.** No repair merges while it is red.
2. **A rule that fires on `Resolved::Inherited` is a bug**, not a preference.
3. **Findings carry evidence.** A diagnostic a reviewer cannot verify without
   opening the app is not finished.
4. **Report only what was verified.** `NOT RUN` is an honest result; inferred
   compatibility is not. Inherited from this project's prior art, and
   non-negotiable.
5. **Adapters lower; the core decides.** Format vocabulary in `mirsam-core` is
   a design failure, however convenient.
