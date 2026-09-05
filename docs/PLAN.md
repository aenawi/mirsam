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

## M1 — Repair `[x]`

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
layout-aware answer is 2.2, which has since landed. That deck's paragraphs
keep the template's left alignment because the master says `algn="l"`, and
`alignment-unset` now says so and names the master. **The application check on
that answer is `NOT RUN`:** nobody has opened a deck repaired since 2.2 to see
that a centred title is left centred and a left-edge paragraph moves.

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

## M2 — Inheritance `[x]`

### 2.1 Package relationship graph `[x]`
`mirsam-ooxml::rels` reads every `_rels/*.rels` in a package and answers, for
any part, which parts it inherits from and in what order:
`RelationshipGraph::inheritance_chain` and the `layout_of` / `master_of` /
`theme_of` accessors over one walk. `PptxDocument::relationships()` is where
2.2 reaches for it; `StyleIndex::from_graph` is what does. This item shipped
alone, so no report changed and the golden corpus was untouched — this is the
graph, not the resolution that walks it.

**A part's role is read from the graph, not from its directory.** What a part
*is* comes from the relationship type that points at it: the part
`presentation.xml` reaches with a `slide` relationship is a slide, wherever it
is stored. Matching `ppt/slides/` instead would be a convention OPC does not
require, and it cannot settle the one ambiguity that matters — a slide master
relates to its layouts *and* its theme, so "follow the first relationship
upward" walks a master back down into a layout. Knowing the role first makes
each hop exact. Relationship types are matched against the full standard
namespace, not by their last segment, so a Microsoft-namespaced extension of
the same name is never mistaken for a standard one.

**Targets resolve to stored item names.** A resolved target is handed back in
the form the ZIP actually stores, because the only useful thing to do with it
is read that part; the percent-decoded form is tried only when the encoded one
is absent. The torture deck's `ppt/media/my%20image.png` is encoded on both
sides and is what a resolver that decoded names would break.

*Acceptance:* met. Twelve unit tests over synthetic packages cover target
resolution (relative, `..`, `.`, package-absolute), external targets, a
foreign namespace, a missing attribute, both encodings, a dangling target, and
a relationship cycle — which terminates the walk rather than looping. Seven
tests over the corpus prove it against real decks: every slide in all five
reaches a layout, a master and a theme, each readable from the package; every
notes slide reaches its notes master; every resolved target is a part
`read_bytes` accepts, with the percent-encoded case asserted non-vacuous. The
load-bearing one is `the_role_read_from_the_graph_is_the_role_the_layout_convention_implies`:
the graph never reads a directory name, so agreeing with the convention on
every part of every deck is an independent check of the inference — and the
test that fails first if it is ever quietly replaced by a name match.

### 2.2 Property chain resolution `[x]`
`mirsam-ooxml::inherit` walks paragraph → shape list style → placeholder
(`p:ph/@type`, `@idx`) on the layout → the same placeholder on the master →
the master's named text style, populating `Resolved::Inherited` instead of
`Unset`. A shape that is not a placeholder — a text box, a table cell, a
chart's fallback drawing — takes the master's `otherStyle`; a notes slide
takes the notes master's one `p:notesStyle` whatever its placeholder type.
`Resolved::Inherited` carries an `Origin` naming the part and property that
supplied the value, and `Evidence` an `inherited_from` rendering it, so a
finding on an inherited value can be checked without opening the application.
The resolution is the adapter's, not the core's: `mirsam-core` still performs
no I/O and still receives units it can judge without knowing what a layout is.

**Direction and alignment only.** [ADR 0007](adr/0007-an-inherited-default-is-not-a-choice.md)
decides what to conclude from an inherited value by asking whether it agrees
with the text, and states that test for those two properties; there is no
decided answer for an inherited language tag, and resolving one would be
inventing the semantics rather than implementing them. The font slots are
worse than undecided — a real master writes `<a:cs typeface="+mn-cs"/>`, a
reference into the theme's `a:fontScheme` — and so is list level: `a:pPr/@lvl`
selecting between `lvl1pPr` and `lvl9pPr` is 2.3 by name. Level 1 is the level
a paragraph that states no `@lvl` uses, which is every paragraph in the corpus.
Both landed in 2.3 below; the language tag did not, and still has no decision.

*Acceptance:* met, on both halves. A deck with direction set only on the
master reports zero `direction-unset` warnings, and a right-to-left paragraph
a layout centres reports no `alignment-unset` note while one an English layout
leaves on the left edge still does, naming the part that left it there
(`tests/inherit.rs`, over hand-built packages). "Starts red" is kept rather
than described: `the_rtl_mastered_decks_lose_their_paragraph_level_unset_findings`
audits the same decks a second time with the chain deliberately not read and
asserts the findings *are* there, so it cannot pass by the decks having had
nothing to report.

*What the golden corpus measured, against what ADR 0007 predicted.* Three of
its four predictions held and one did not, which is what makes them worth
having written down:

| deck | predicted | measured |
| --- | --- | --- |
| `broken-arabic.pptx` | loses its paragraph `direction-unset` / `alignment-unset` | 2 and 2 → 0 and 0 ✓ |
| `clean.pptx` | the same | had none to lose; still none ✓ |
| `torture.pptx` | the same | 1 and 4 → 0 and 1 ✓ — the one left is in `ppt/charts/chart1.xml`, a part with no layout and no master, where `Unset` is the honest answer |
| `quarterly-report.pptx` | keeps the count it has today | `direction-unset` 7 → 7 ✓ (same severity, new reason, master named); `alignment-unset` 17 → 13 ✗ |
| `quarterly-report-correct.pptx` | stays clean | clean ✓ |

The four that went are its centred titles, and they went for the reason
[ADR 0007 §4](adr/0007-an-inherited-default-is-not-a-choice.md) gives in the
same breath as the prediction: `algn="ctr"` reads correctly in either
direction, and silencing it is what retires ADR 0006's cost note that
`--align` pushes a centred title to the right edge. Reading §4's table and
the Consequences' "must not get quieter" together, the table is the operative
rule and the prediction overlooked that this master's `titleStyle` is centred.
The repair diff shows the same thing from the other side: `repair --align` now
writes `rtl="1"` where it used to write `rtl="1" algn="r"`.

*One thing §3 costs.* `direction-mismatch` deliberately does *not* judge an
inherited direction — `judged_direction` falls back to auto-detection rather
than to `Resolved::effective` — because ADR 0007 §3 says the rule stays a
finding about a direction the author wrote. Without that, five of
`quarterly-report.pptx`'s seven `direction-unset` warnings become
`direction-mismatch` errors, on the strength of an English template's
untouched `rtl="0"`. Those paragraphs really do render in the wrong order, so
the tool now says so at warning rather than error severity, naming the master.
`--strict` still blocks on them.

### 2.3 List levels and theme fonts `[x]`
Both walk the chain 2.2 built. A style source now states its properties nine
times over — `a:lvl1pPr` … `a:lvl9pPr` — and a paragraph's `a:pPr/@lvl`
selects which one answers it at *every* hop, the shape's own list style and
the layout's as much as the master's named styles. `@lvl` is zero-based, so
`lvl="1"` reads `lvl2pPr`; a paragraph that states no `@lvl` is at level 1 as
before, which is every paragraph in the corpus but one.

**A level is answered only by the same level above it.** A master stating
`lvl1pPr` and nothing else supplies nothing to a paragraph at level 3, and the
walk carries on to the next source rather than falling back to level 1 here.
PowerPoint's own fallback for a level a master leaves out is its application
default, not that master's first level, so reaching for level 1 would report a
value no reader will see — and report it as *inherited*, which is a claim
about the document rather than a guess.

**The complex-script slot resolves; the Latin slot deliberately does not.**
`complex-font-missing` asks whether *any* complex-script font is named, not
whether the named one suits the text, so ADR 0007's agreement test has nothing
to decide here and inheriting the slot can only ever make the tool quieter.
Read the other way round, the same rule is why the Latin slot stays where it
was: the finding fires only where a Latin font is *chosen*, and a template's
`+mn-lt` is not a choice anyone made about this paragraph. Inheriting it would
manufacture the rule's precondition on every Arabic paragraph in every deck
sitting on a stock theme — a rule firing on formatting nobody chose, which
standing rule 2 calls a bug.

**`+mn-cs` is a pointer, not a font.** A real master writes
`<a:cs typeface="+mn-cs"/>`, a reference into the theme's `a:fontScheme`;
`RelationshipGraph::theme_of` reaches the part and `inherit::FontScheme` reads
it. The finding names the *theme*, not the master that wrote the reference,
because the theme is where a reviewer can read the typeface the reader will
see in one look (invariant 6). Before this, a run writing `+mn-cs` on itself
was recorded as a font by that name — a font nobody has — and a report said
so; it now resolves, or stays `Unset` where nothing answers it.

**An empty answer is not an answer.** The stock Office theme states
`<a:cs typeface=""/>`, which is a theme naming no complex-script font. It
resolves to nothing rather than to the empty string, so
`quarterly-report.pptx` keeps all four of its `complex-font-missing` warnings
— the deck really does name no Arabic font anywhere. Its
`<a:font script="Arab" typeface="Times New Roman"/>` sits beside those slots
and is the application's per-script fallback rather than the slot a reference
names; what it means for an empty `cs` is not a question this milestone
answers, and it is left alone rather than guessed at.

*An inherited language tag is still not resolved,* and that is the one bullet
of this item deliberately not built. ADR 0007's agreement test is stated for
direction and alignment; there is no decided answer for an inherited `lang`
that disagrees with the letters, and resolving one would be inventing the
semantics rather than implementing them.

*Acceptance:* met, and the golden corpus moved by exactly four lines — the
`@rtl` and `@algn` evidence on `quarterly-report.pptx`'s one `lvl="1"`
paragraph, before and after repair, now citing `bodyStyle/lvl2pPr` rather
than `lvl1pPr`. No finding appeared, disappeared or changed severity, which is
the intended shape: every stock master states all nine levels alike, so the
cited level is the *only* thing that can tell a resolver reading `@lvl` from
one that always reads the first. Eleven tests in `crates/mirsam-ooxml/tests/inherit.rs`
cover the rest. Two are load-bearing:
`a_paragraph_at_the_second_level_reads_the_sources_second_level` puts the same
Arabic under a master whose `lvl2pPr` contradicts its `lvl1pPr` — nothing real
ships that, which is the point, because two levels that agree cannot fail —
and `the_torture_deck_resolves_its_complex_font_through_the_theme` asserts a
resolved typeface arrived from a `ppt/theme/` part, so a resolver that read
only the name `otherStyle` states outright and never opened the theme fails it.

*One fixture changed.* `torture.pptx`'s master used to name `Dubai` outright
in all three text styles. It now writes `+mj-cs` in `titleStyle` and `+mn-cs`
in `bodyStyle` — what PowerPoint itself writes — and keeps the literal name in
`otherStyle`, so one deck exercises both forms. It is a byte change with no
report change, and `make validate-fixtures` still passes on all five decks.

---

## M3 — Word `[x]`

### 3.1 Extract the shared package layer `[x]`
ZIP access, part enumeration and the byte-preserving rewrite already lived in
`mirsam-ooxml::package` — 1.1 needed them there, because a writer that reads
the package through a second code path is a writer whose round-trip guarantee
covers only half of what ships. This item did the same one level down, for the
*token-rewrite* scaffold 1.2 built against DrawingML: extraction after a
working adapter, not anticipation of one.

`rewrite.rs` split in two along one line — *does this code name an element?*
Everything that does not is now `token.rs`: reading a part into events and
writing it back, `passthrough`, the raw-byte attribute splice, finding
elements and their ranges, creating a child at a rank the caller's schema
sequence decides, and reading and rewriting run text. Everything that does
stayed in `rewrite.rs`, which keeps its name and its public API and is now
what it always was — DrawingML's repair vocabulary. It performs no XML
editing of its own; every mutation there is a scaffold call with a DrawingML
name in it.

Three functions took a parameter where they had a constant. `a:t` became the
`text_element` argument of the text helpers, because `w:t` is the same thing
in the other vocabulary. `normalize_presentation_forms` and
`remove_controls`, which were the domain's mapping and the domain's offsets
wrapped in a run-splice, became `map_runs` and `remove_at_offsets` — the
splice without the mapping, so the caller supplies both the element name and
what to do with the text. Nothing else changed shape.

*Acceptance:* `crates/mirsam-ooxml/tests/token.rs`, sixteen tests written
entirely in **WordprocessingML** — `w:p`, `w:pPr`, `w:bidi`, `w:jc`, `w:t` —
a vocabulary nothing in the crate reads yet. That is the only assertion that
can settle whether the extraction is real: a scaffold with `a:t` or `a:pPr`
still baked into it passes every DrawingML test in `rewrite.rs` and fails
here. `a_text_element_of_another_vocabulary_is_not_run_text_here` is the
narrow version of the same claim — an `a:t` sitting in a Word part is a
foreign element, not a run. The golden corpus did not move by a line, and the
byte-identical passthrough over every part of the torture deck still holds;
this is a refactor, and a refactor that changed a report would be a bug.

### 3.2 DOCX reader `[x]`
`w:p`, `w:pPr/w:bidi`, `w:jc`, `w:lang/@w:bidi`, `w:rFonts/@w:cs`.

`docx.rs` is a `DocumentReader` and nothing else. `DocumentWriter` is a
separate port precisely so an adapter can arrive one half at a time, and
`mirsam repair` refuses a `.docx` as a *readable* format without a writer
rather than as an unknown extension — the audit path reads it, and one
command denying what the other does would be the tool contradicting itself.
`mirsam-core` did not move by a line, which is what M3 is testing.

**`w:jc` is direction-relative, and that is the one real decision here.** The
standard says its values "are always specified relative to the page, and do
not change semantic from right-to-left and left-to-right documents". Word does
not implement that. Its own note is explicit: *"Word evaluates the value of
this attribute based on the value of the bidi element: Left is the right side
of a right-to-left paragraph, and right is the left side of a right-to-left
paragraph"* ([MS-OE376] Part 4 §2.3.1.13, note b). So `left` is the *start*
edge — the same value ISO 29500 Strict later spelled `start` — and this
adapter lowers `left`/`right` onto `Start`/`End`. **No Word paragraph ever
produces `Alignment::Left`**, so `alignment-incoherent` is structurally silent
on DOCX. That is not a gap: a Word author cannot write the defect that rule
reports, because the attribute they would use is direction-relative. Reading
`left` as a physical edge would have manufactured the finding on every
left-aligned Arabic paragraph in Word — invariant 2 reached through the
adapter instead of through the rule.

Three smaller decisions, each of which is a defect if taken the other way.
`<w:bidi/>` with no `w:val` is *on*, which is the form Word writes far more
often than `w:val="1"`; reading a missing attribute as false would report
every correctly-marked Arabic paragraph in Word. A `w:sectPr` inside a
paragraph's `w:pPr` — where the last section's properties live — is the
section's statement, not that paragraph's. And an `mc:Fallback` is skipped
beside the `mc:Choice` it stands in for, because both spell out the same text
box and reading both reports every defect in it twice, under two unit ids
naming one paragraph. Paragraphs are held on a stack rather than in a slot,
because `w:txbxContent` nests them and a single slot loses the outer
paragraph's text when the inner one closes.

`is_true` moved from `pptx.rs` to `token.rs`. `ST_OnOff` is defined once in
ECMA-376 and names no element; leaving it in DrawingML's reader would have
made WordprocessingML's depend on it to agree about what a document says.

*Acceptance:* `crates/mirsam-ooxml/tests/docx.rs`, twenty-four cases, and four
in `cli.rs`. Every `w:jc` value this adapter reads is asserted *not* to raise
`alignment-incoherent`; a DrawingML `a:p` sitting in a Word part is asserted
to be no paragraph at all, which is `token.rs`'s claim run in the other
direction. The golden corpus did not move: no `.pptx` reads differently for
any of this.

[MS-OE376]: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f

### 3.3 Style-chain inheritance `[x]`
`docDefaults` → linked styles → direct formatting.

`style.rs` is WordprocessingML's answer to `inherit.rs`, and the two share no
element name. Word's chain is not a walk between parts: every source lives in
one `word/styles.xml`, and a paragraph reaches its own by *name* — `w:pStyle`,
a run's `w:rStyle`, and the `w:basedOn` above each — rather than by a
relationship. So the graph is not the thing being walked here, and
`rels` supplies exactly two edges: which part is the stylesheet and which is
the theme.

The order, nearest first, is the character style's chain, then the paragraph
style's, then `w:docDefaults`. **A paragraph that names a style does not also
take the document's default one** ([ECMA-376] Part 1 §17.7.2) — the style it
named is the whole answer, and its `w:basedOn` chain is where it looks next. A
walk that consulted the default anyway would report `Normal`'s direction on a
paragraph laid out by a style that states none, which is a value no reader
will see.

*What is resolved is exactly what `inherit.rs` resolves*, for the reasons ADR
0007 gives, plus the list. Direction and alignment have the agreement test;
the `cs` slot needs none because `complex-font-missing` asks only whether *a*
font is named, so resolving it can only make the tool quieter; the Latin slot
is left alone because inheriting a template's `w:asciiTheme` would manufacture
that rule's precondition on every paragraph in every document; and `w:lang`
stays unresolved because ADR 0007 has no answer for a tag that disagrees with
the letters. The list is the quieter-only argument again — Word's own list
styles carry the `w:numPr`, so a paragraph in one has a real list its `w:pPr`
says nothing about, and reporting a typed glyph there is invariant 2 reached
through the chain. `Bullet` has no `Inherited` state and so records no origin,
which is sound only because a resolved list can never *raise* a finding.

**`w:link` is not a hop.** A linked style is one paragraph style and one
character style Word shows as a single entry, and it writes the run properties
into *both* halves. Following the link would resolve a value already stated
where the walk is looking, and on a document whose halves disagree it would
prefer the half Word does not apply.

*Two decisions the schema does not make for you.* `@w:cstheme` names a slot of
the theme's `a:fontScheme` — `minorBidi` is `+mn-cs` in the other spelling —
and a reader that took it for a typeface would put `complex_font: "minorBidi"`
in a report, the WordprocessingML form of the defect 2.3 fixed in DrawingML.
The theme part is DrawingML in *both* formats, so `FontScheme` reads Word's
unchanged; only the reference syntax differs. And where a `w:rFonts` states
both `@w:cs` and `@w:cstheme`, the theme wins, because the theme is what Word
renders and `@w:cs` is the resolved value it caches beside it for consumers
without theme support — which is exactly what that name becomes when the
theme's slot is empty or the package has no theme at all.

*Two smaller ones.* `w:numId w:val="0"` says the opposite of the `w:numPr`
enclosing it: it *removes* the list a style supplies, which is
`Bullet::Suppressed`, and a paragraph that suppressed its list and then typed a
glyph is exactly the defect `literal-bullet` reports. And `Role::Presentation`
became `Role::OfficeDocument`: one relationship type reaches the main part of
both formats, and a `Presentation` role on `word/document.xml` would be the
graph misreporting what it read.

*Acceptance:* `crates/mirsam-ooxml/tests/style.rs`, twenty-four cases. Three
are load-bearing. `a_table_styles_own_alignment_is_not_the_paragraphs` puts a
`w:jc` in a `w:tblPr` and a whole `w:pPr` in a `w:tblStylePr`, both inside a
`w:style`: a reader matching element names instead of the element *path* reads
a table's alignment as a paragraph's and passes every other case in the file.
`a_scan_resolves_every_part_against_the_stylesheet_the_relationships_name`
stores the stylesheet and theme under names no reader could guess, so anything
hard-coding `word/styles.xml` resolves nothing and reports every paragraph in
that package undeclared. And the DrawingML case is `token.rs`'s claim run once
more in this direction — an `a:lvl1pPr` is PowerPoint's style vocabulary and
answers nothing here.

The golden corpus did not move: no `.pptx` reads differently for any of this,
and the corpus has no `.docx` in it yet — which is 3.5's to add, not this
item's.

[ECMA-376]: https://ecma-international.org/publications-and-standards/standards/ecma-376/

### 3.4 Tables `[x]`
`w:bidiVisual` only where semantic reading order is RTL.

A Word table is the container 2.4 already built. `w:tblPr/w:bidiVisual` says
the cells are displayed right to left with the file's own ordering unchanged —
"the first logical cell with text is stored first in the file format, and
displayed on the rightmost" ([ECMA-376] Part 1 §17.4.1) — which is word for
word what `a:tblPr/@rtl` says in DrawingML. So a `w:tbl` lowers onto
`UnitKind::Table` under the same id shape (`word/document.xml#tbl1`), and
`container-direction` asks the only question the item states: **does the text
in this table read right to left, and does its column order agree?** Nothing
in `mirsam-core` needed a new concept for that, which is what M3 is testing.

The paragraphs in the cells stay units in their own right, because Word does
not make a cell's text inherit the table's column order. What each gains is a
location naming its cell — `table 1 row 2 cell 3` — which is the one thing
Word names around body text and, on a table of any size, the difference
between a finding a reviewer can act on and a hunt. `w:tbl` nests, so tables
are held on a stack exactly as 3.2 holds paragraphs, and a nested table's text
belongs to both containers: each judges it under its own direction.

**A table style can state `w:bidiVisual`, so table styles could not stay out
of `style.rs`.** `CT_TblPrBase` carries it, which means a table that names a
right-to-left table style is laid out right to left while its own `w:tblPr`
says nothing. Reading that as undeclared would report `container-direction` on
every correctly-styled Arabic table in the document — invariant 2 reached
through the adapter, which is the trap 3.2 and 3.3 each spent an item
avoiding. `StyleSheet::resolve_table` walks the chain the table names, or the
document's default table style for a table naming none, and records the answer
as `Inherited` with the style that stated it.

*That makes a container inherit, which nothing did before, and ADR 0007's
consequences said no container would.* The rule had an arm for it that
returned silence unconditionally, on the stated premise that a container's
direction "is stated on the container or not at all". 3.4 makes that premise
false, so the arm now does what [ADR 0007] §1 decides for every other
inherited value: **agrees with the text, silent; contradicts it, reported
exactly as an absent one**, at the severity an absent one carries (§3), naming
the style that supplied it (§5), and repaired on the table rather than on the
style (§6). This is the one core change in M3 so far. It adds no Word
vocabulary — it is `Resolved<Direction>` and `bidi::dominant_direction`, both
already there — and it applies to a PowerPoint container the day one can
inherit. The ADR carries a dated note recording it.

Word's hierarchy also puts a table style *below* the paragraph style and above
`w:docDefaults` ([ECMA-376] Part 1 §17.7.2), so a cell's paragraph reaches one
too, and `StyleSheet::resolve` gained that hop. **`w:tblStylePr` is still not
read**: a table style's conditional formatting applies only to the parts
`w:tblLook` turns on and only at the cell positions it names, and reading it
without that mask would put a header row's formatting on every cell. Only the
unconditional `w:pPr` and `w:rPr` are taken, which is what Word applies
everywhere in the table. 3.3's case putting a whole `w:pPr` in a
`w:tblStylePr` still holds and still asserts it.

*One defect fixed on the way past.* `w:tblPrChange` holds a table's column
order as it stood **before** a tracked change, in the same element that states
it now and written after it — so the reader would have taken the layout
somebody has already corrected. The same is true one level down of
`w:pPrChange`, which 3.2 was reading: a paragraph whose alignment was revised
reported the value it no longer has. Every `*Change` in the family is now
skipped, on the same depth counter `w:sectPr` and `mc:Fallback` use.

*Acceptance:* thirteen new cases across
`crates/mirsam-ooxml/tests/docx.rs` and `tests/style.rs`, and two in
`mirsam-core`. Four are load-bearing.
`an_arabic_table_laid_out_left_to_right_is_the_flagship_table_finding` is the
item's own sentence run in both directions — the Arabic table with no
`w:bidiVisual` is reported, and the same table with one, and an English table
with none, are both silent.
`a_table_style_supplies_the_column_order_the_table_did_not_state` is the false
positive that made table styles necessary, and
`a_style_chain_answers_the_table_and_a_contradicting_answer_is_still_reported`
is the other half of the same walk, which a reader that resolved the chain and
then trusted it would fail.
`a_superseded_direction_in_a_revision_record_is_not_the_tables` puts a
`w:bidiVisual w:val="0"` inside a `w:tblPrChange` beside the live one, and a
scanner matching the element without the guard reads the corrected layout as
current. `a_drawingml_table_is_not_a_word_table` is `token.rs`'s claim run
once more in this direction.

The golden corpus did not move: no `.pptx` reads differently for any of it —
no PowerPoint container can inherit, so the rule's changed arm is unreachable
there — and the corpus still has no `.docx`, which is 3.5's to add.

[ADR 0007]: adr/0007-an-inherited-default-is-not-a-choice.md

### 3.5 Conformance suite `[x]`
One suite both adapters run unchanged. If DOCX needs a core change to pass, the
abstraction was wrong — fix the abstraction, not the test.

Every test file before this one asks whether an adapter reads its own format.
`crates/mirsam-ooxml/tests/conformance.rs` asks the question M3 was actually
testing: **do the adapters agree?** A case states a situation once, in the
shared model's own vocabulary — "a paragraph of Arabic with no direction
declared, under a chain that states nothing" — and each format lowers it into
a real package on disk, which the suite opens through `DocumentReader` and
nothing else. **No case that asserts what the tool reports names an element,
an attribute or a format** — the only assertions that name one are the two
refusals below, which exist to say where the formats differ. A case that had
to know which adapter it was looking at would be the hexagon leaking, and the
thing to fix would be the abstraction rather than the case, which is what this
item says.

*Twenty-seven cases in four groups.* The port's own contract — a stable
format name, a location naming a part the package really holds, unique ids
stable across two scans, an ordinal on a paragraph and none on a container,
text that comes back in logical order carrying nothing the reader invented.
The shared model — `Explicit` where the unit states a value, `Unset` where
nothing does, `Inherited` with an origin a reviewer can open where the chain
supplies one, a table as a container beside the paragraphs in its cells. Then
the load-bearing group, which asserts the same *rules* fire on the same
situation in both formats: undeclared Arabic, Arabic declared left-to-right,
correct Arabic, English, a table with and without a column order, a chain that
agrees and a chain that contradicts, a typed bullet, a bidi control, a
pre-shaped run, an empty complex-script slot.

*The answer to "did DOCX need a core change to pass" is no, and the suite was
written to be able to say so.* Nothing in `mirsam-core` moved for this item.
The one thing that did move was the corpus, which is the other half of the
claim: a suite that only reads packages it built itself proves the two
adapters agree about XML this repository wrote.

**The formats are not identical, and pretending otherwise would be the second
way to make the file lie.** Word's `w:jc` is direction-relative — its
`left` is the *start* edge ([ECMA-376] Part 1 §17.18.44) — so a hard left edge
cannot be written in Word at all. DrawingML is the exact mirror: `a:pPr/@algn`
names physical edges and has no direction-relative spelling. Neither is a gap
in an adapter, and neither may be silently skipped, so a vocabulary that cannot
state a situation returns `Inexpressible` **with the reason**, the case runs
against the formats that can, and `every_refusal_is_one_the_design_intended`
holds the whole list of refusals against the committed one. It is two entries
long, they are the same fact seen from either side, and a format that quietly
stopped expressing anything else fails there rather than passing. That is also
what settles the one asymmetry a user could mistake for missing coverage:
`alignment-incoherent` is structurally silent on Word because Word has no way
to write the defect, and a case named for that fact
(`a_hard_left_edge_under_arabic_is_reported_by_the_format_that_can_state_it`)
asserts it, rather than leaving it to a paragraph of `AGENTS.md`.

*Two Word documents joined the golden corpus*, built by
`scripts/make-word-fixture.py` the way `make-torture-fixture.py` builds the
decks — hand-written XML in a hand-written container, deterministic, and
schema-valid against `wml.xsd`. `quarterly-review.docx` carries every defect
the reader can find next to text that is correct and text that is English;
`quarterly-review-correct.docx` is the same document authored properly and
must be left completely alone, which is what makes exit code `0` provable for
`.docx` and not only for `.pptx`. Both exercise Word's own chain, because a
document that never inherits anything would let a broken `StyleSheet` pass:
`Normal` states the document's right-to-left default, `EnglishBody`
contradicts the Arabic under it, and `RtlTable` supplies a column order the
table does not state.

**`repair` refuses a `.docx`, and the corpus now records the refusal rather
than working around it.** A report for a readable format with no writer holds
the sentence the binary printed and the exit code it used — `2`, a usage
error, because the audit above it read the file perfectly well. The day the
Word writer lands, that shows up as a diff on a real document instead of as a
test somebody remembered to update. The three OPC-level invariants in
`corpus_packages.rs` — content types, relationships resolving, ASCII item
names — now run over every corpus document rather than the decks alone, which
is the package layer being held to the guarantee the second format reuses it
for; the checks that name `p:spTree` or a notes master stay with the decks.

*Acceptance:* `cargo test` runs the conformance suite against both adapters
with no case skipped and no adapter special-cased, and `make golden`
regenerates a corpus that now holds documents of both formats. The five
committed reports changed in one line each — the key naming the file is
`document` rather than `deck`, because the corpus is no longer only decks.

---

## M4 — Shaping `[x]`

### 4.1 `rustybuzz` shaping `[x]`
Shape each Arabic run; assert joining forms are produced.

Two modules, and the split between them is the point. `mirsam-core::joining`
says what the *text* requires — the seen of `سلام` must be drawn initial, the
lam medial, the alef final, and the meem after the alef standalone — from the
logical-order codepoints alone, with no font in the room. It is the Unicode
Joining_Type property and the four-line rule the standard states over it, and
every one of its tests is checkable against ArabicShaping.txt.
`mirsam-core::shape` then hands each run to `rustybuzz` — HarfBuzz's shaping
algorithm, ported, and the same code Firefox's successor engines run — and
reports what actually came back. **A shaping defect is the gap between the
two**, and it is the first defect in this tool that no amount of reading XML
could ever find: the text is correct Unicode, correctly directed, correctly
aligned, and it renders as a row of disconnected letters.

*What "joined" is decided by.* A font's `cmap` gives the glyph a character
draws when nothing has shaped it; a shaper applies `init`, `medi` or `fina`
and substitutes another. So the question is answered without knowing anything
about a font's design, its glyph names or its rendering: **the character's own
glyph is not among the ones that came back.** A font with no `GSUB` cannot
produce any other.

**One letter drawn standalone is not a defect, and a shipped font proves it.**
The tempting rule — report every letter that came back standalone — fires on
macOS's Arial, which renders Arabic perfectly and whose `fina` covers neither
reh nor meem. It does not need to: a reh only ever takes a join from its
right, and the stroke that makes that join is drawn by the letter *before* it.
So `shape` reports and does not judge. It says which letters were required to
join, which the font drew standalone and which it has no glyph for; whether
that adds up to a finding is 4.3's decision, and 4.3 can only make it honestly
if the facts arrive unweighted. The one signal that survives is the aggregate:
a font with no shaping tables produces **no** joins in a run that required
several, and no design choice can look like that.
[ADR 0008](adr/0008-a-standalone-letter-is-not-a-shaping-defect.md).

*Three fonts, built here, differing in one thing each.*
`scripts/make-shaping-fixture.py` writes them byte by byte with Python's
`struct` and nothing else — no `fontTools`, for the reason
`make-torture-fixture.py` gives about `zipfile`, and nothing in the generator
has seen `ttf-parser` or `rustybuzz`. `joining.ttf` carries all three form
features; `nonjoining.ttf` is the same font with no `GSUB` at all, which is
the defect M4 exists for; `partial.ttf` shapes everything except the final
forms of the right-joining letters, which is Arial's behaviour and is not a
defect at all. A rule that regresses to a per-letter verdict fails against
`partial.ttf` rather than on a user's deck. They are ~1.8 KB each, they carry
no outlines because shaping never reads one, and `make fonts` regenerates
them deterministically.

*The boundary this item does not cross.* `Font::parse` takes bytes.
Which typeface a paragraph resolves to, and where that file lives on which
machine, are questions about the world and belong to an adapter — invariant 1
holds exactly as before, and `mirsam-core` still opens nothing. That is 4.2's
work, and until it lands nothing in the audit path calls any of this.

*Acceptance:* met. `cargo test` shapes real text through three real fonts and
asserts the exact glyph each letter came back as, and the three fonts disagree
about nothing except shaping. `مرحبا` produces five joins through
`joining.ttf`, three through `partial.ttf` and none through `nonjoining.ttf`.
Vowelled text is asserted too, because a shaper merges a harakat into its
base's cluster and a check that demanded one glyph per letter would report
every vowelled word in Arabic. Validated beyond the fixtures by hand against
macOS's SF Arabic, Mishafi, Times New Roman and Arial: no false join, no
false failure, including on the lam-alef ligature.

### 4.2 Font coverage `[x]`
`ttf-parser` over the resolved font; report missing codepoints by name.

Unblocked by 4.1: `shape::Font::covers` and `Shaping::unmapped` already
separate "the font has no glyph for this letter" from "the font drew it
standalone", because shaping could not report honestly without the
distinction. What is left is the part that is not the domain's: resolving the
typeface a paragraph names — `Properties::complex_font`, and the theme or
style that supplied it — to bytes on a machine, which is an adapter's work and
where the I/O lives.

*Two different defects.* A font with no shaping tables renders Arabic as
disconnected letters; a font with no Arabic renders it as a row of empty
boxes, and no shaping table would have saved it. "Install a font" is the wrong
advice for the first and the only advice for the second, so
`mirsam-core::coverage` is its own module rather than a field on a shaping
result.

*What a font is judged over.* Only the characters it answers for. A
complex-script slot draws the Arabic in a mixed paragraph and nothing else, so
reporting an Arabic font for having no `Q` would be reporting a font for text
it was never asked to draw — ADR 0004's first failure mode through a new door.
Format characters are excluded because they are meaning rather than shape
(U+0600 prefixes a numeral, U+06DD encloses a verse number), and presentation
forms because pre-shaped text is already a blocking defect with a repair of
its own: a second finding there would be the tool arguing with itself about
one character.

*By name.* `charname` is the Arabic script's Unicode names, generated by
`scripts/make-char-names.py` from the UCD Python ships — no network, no
third-party module — and refreshed with `make names`. A finding reads
`U+067E ARABIC LETTER PEH`, not `U+067E`. It answers `None` outside the four
logical-order Arabic blocks rather than inventing a name, the same refusal
`joining::JoiningType::Unstated` makes.

*The port and its adapter.* `ports::FontSource` is the boundary — a family
name in, a `FontFile` out, `None` when the machine has no such font, which is
a reportable state and not an error. `mirsam-fonts` is the adapter: it indexes
the platform's font directories by reading each file's naming table alone, not
the file, which is 690 families in 118 ms on macOS instead of half a gigabyte
of outlines. Two things it learned from real fonts rather than from the spec:
macOS states English family names in Mac Roman and keeps the Unicode records
for the localisations, so refusing the legacy encoding leaves `Helvetica.ttc`
nameless; and a family is spread over many files that all state the family
name, so a document saying `Arial` must be answered with the *regular* face —
taking the first match gives `Arial Bold Italic.ttf`, which on macOS has no
Arabic while `Arial.ttf` has. `bold.ttf` joined the font fixtures so that
regressing on the second fails a test instead of a user's deck.

*The boundary this item does not cross.* Nothing in the audit path calls any
of this yet. Turning a coverage report into a finding is 4.3's decision, and
`Coverage::covers_nothing` is deliberately the only conclusion the evidence
supports on its own.

*Acceptance:* met. `cargo test` runs coverage against the hand-built fonts,
which map U+0621..U+064A and nothing else, so every boundary the check draws
shows up as a character the fixture has or has not got — and the four fonts
agree about coverage exactly while disagreeing about shaping. `mirsam-fonts`
resolves a family name to bytes and proves it got the right file by shaping
through it. Validated beyond the fixtures against macOS: `Helvetica` and
`Comic Sans MS` come back missing every Arabic letter of the text,
`Times New Roman` and `Geeza Pro` come back complete, `Mishafi` shapes
perfectly and is missing only Persian peh and Urdu tteh, and `Arial` resolves
to `Arial.ttf` with complete coverage and four joins of five — ADR 0008's reh,
still not a defect.

### 4.3 `shaping-broken` and `font-coverage` rules `[x]`

4.1 and 4.2 built two modules that refuse to draw a conclusion. This is where
the conclusions are drawn, and there are exactly two, because they are two
defects with two pieces of advice. **`font-coverage`** says the font has no
glyph for the Arabic: it renders as empty boxes, and no shaping table would
have saved it. **`shaping-broken`** says the font has every letter and no
shaping tables: it renders as a row of disconnected letters, and installing a
font with more coverage would change nothing. Telling an author to pick a
different typeface is the answer to both and for opposite reasons, which is
why one rule reporting both would be a rule that cannot be acted on.

**These are the first rules in this tool that ask a question about the
machine, and that is why they are off by default.** Every other check reads a
document and gives the same answer on any computer. These two need a font
file, and which fonts a computer has is not a property of the deck: an audit
that resolved them without being asked would report differently on a
developer's laptop and on a CI runner looking at one document, and neither run
could be reproduced from the file alone. So `mirsam audit --fonts` asks for
them, `--font-dir` pins the directories searched — which is what makes the
check reproducible at all — and **the report says which of the two audits it
is**, in both output formats: `fonts NOT RUN` for a human, `"fonts":
{"checked": false}` for an agent. That is standing rule 4 spent where it was
always going to be needed. The golden corpus never passes the flag, so the
six committed reports gained one key each and not a single finding, which is
the claim that nothing else moved.

*Which way the claim runs, and it only runs one way.* A font that is **here**
and cannot draw the text will not draw it anywhere, because a `cmap` and a
`GSUB` travel with the font — that is what makes a finding worth making. A
font that is here and draws the text perfectly proves nothing whatsoever
about the reader's machine, and silence from these rules is not a promise
that the document renders. `AGENTS.md` says so where an agent will read it.

*Three ways to say nothing, all of them deliberate.* A paragraph naming no
complex-script font is `complex-font-missing`'s, not theirs — guessing which
font the application will substitute is exactly the invention this design
refuses. A family this machine does not have is the tool losing the ability to
say what the reader will see, which is a fact about the computer and not a
defect in the deck; reporting it would fire on every runner with no fonts
installed. And a file the shaper cannot parse is a fact about the file,
reported by whoever supplied it.

**Where the shaping threshold sits, which ADR 0008 left to this item.** The
signal is `joins_produced == 0` against enough required joins to mean
something, and the number is **four**, counted over letters the font actually
has. The arithmetic is the argument: a letter is required to take a *final*
form only because the letter before it joined forwards, so every final has a
companion initial or medial, and initials and medials are the dual-joining
letters — the ones a font like Arial does shape. Two joins are therefore
*one* such letter, which is precisely the design choice ADR 0008 forbids
concluding a defect from, and which a two-letter word is the whole of. Four
are two of them, independently, both silent, and no font that shapes Arabic
at all looks like that. Unmapped letters are excluded from the count for a
different reason: a font cannot join a glyph it does not have, so counting
them would let a Latin-only font produce a `shaping-broken` finding next to
its `font-coverage` one — the tool arguing with itself about one paragraph,
with the same repair and two explanations.

*Severity is what 4.2 built `missing_occurrences` for.* A font answering for
**none** of the Arabic was not the font this text needed: the paragraph goes
blank, there is nothing to argue about, and it is an error. A font missing
*some* of it is an otherwise sound pairing meeting an unusual letter —
Mishafi has no Persian peh — which is still wrong for the characters it hits,
still worth an author's attention, and is not the same claim. Warning.

**Neither is fixable, and that is not an omission.** The repair for both is a
different typeface, and which one is an authoring decision the text cannot
supply — the reason `complex-font-missing` proposes nothing until `--font`
chooses one. It is stronger here: that rule fills an *empty* slot, while
these two would be overwriting a font the author deliberately put there.

*The fifth fixture font.* `latin.ttf` maps printable ASCII and no Arabic
whatsoever, written by the same `struct`-and-nothing-else generator as the
other four. It is Helvetica under Arabic reduced to its principle, and it is
what makes the acceptance a committed test rather than a manual check. The
four fonts now disagree about exactly two things — coverage and shaping — and
each rule is separated by the one it is named for.

*A cost, stated rather than optimised away.* Each rule resolves the typeface
per unit, so a paragraph's font file is read twice and a deck's font file
once per paragraph per rule. On a 126-unit deck resolving to macOS's 2.3 MB
`Helvetica.ttc` that is 42 ms — the operating system's page cache absorbs it —
against 5 ms for the same audit with the checks off. Memoising the bytes is
one `HashMap` in an adapter when a deck arrives that needs it, and
`ARCHITECTURE.md`'s KISS note is the reason it is not there already.

*Acceptance:* met. `cargo test` audits `broken-arabic.pptx` through the real
PPTX adapter on a machine whose every typeface is `latin.ttf`, and every
Arabic paragraph comes back a `font-coverage` error listing the exact
characters by name; the same deck through `nonjoining.ttf` comes back
`shaping-broken` and not `font-coverage`; through `joining.ttf`, neither.
Validated beyond the fixtures against macOS, on a copy of that deck retyped
to each face: `Helvetica` and `Comic Sans MS` report every Arabic character of
every paragraph, and `Times New Roman`, `Geeza Pro`, `Mishafi`, `Al Nile` and
`Baghdad` report nothing at all. `Arial` reports nothing either — ADR 0008's
reh, four joins of five, still not a defect.

### 4.4 `tatweel-padding` rule `[x]`

U+0640 ARABIC TATWEEL used as visual padding rather than as justification.
`ROADMAP.md` has asked for this since M4 was written and no item carried it,
which is what this entry fixes.

*The hard part is that tatweel is legitimate*, which makes this ADR 0004's
first failure mode waiting to happen for the third time. It is the kashida:
the letter-stretching Arabic typography has justified text with for a
thousand years, and a font's `GSUB` may insert it. What is a defect is a
*typed* one standing in for a layout the author could not get — a run of them
padding a heading to a width, or one wedged into a word to fake alignment —
and the tool cannot read intent. So the threshold is the whole design
question, the way 4.3's was: state it, argue it, and commit a fixture that
fails a rule which regresses to "any tatweel is a defect".

**The threshold, and why it is a property of the neighbours rather than of
the character.** A kashida a font inserts never reaches the stored string —
it is applied during layout and thrown away with the line boxes — so every
tatweel this tool can see was typed by somebody. `crate::tatweel` therefore
groups them into maximal runs and asks what each run is joined to. Two or
more in a row, joined to a letter on either side, is a word stretched to a
width: **no typography needs a second tatweel**, because one is already
enough to carry a mark or show a form, so the repetition *is* the evidence
and two is where the line falls. One, with a join already crossing it — the
character before joins forwards, the character after takes a join — is width
and nothing else, because both neighbours take exactly the form they had
without it; that is the wedge that fakes an alignment inside a word.

*Three ways to say nothing, and each one is a real document.* A tatweel
immediately followed by a combining mark is the base that mark is written on,
which is how a lone fatha appears in a table of harakat or a keyboard legend;
delete it and the mark lands on whatever precedes. A lone tatweel at a
joining edge is how a letter's initial, medial or final form is written on
its own, in a primer or a dictionary — the tatweel is the thing being shown.
And a run joined to nothing on either side is a rule or a separator, drawn
with the character that draws rules; reporting it would be inventing a
stretched word where there is no word. All three are committed as cases that
a rule regressing to "any tatweel is a defect" fails.

*Note that this is a defect in the text*, not in the properties around it, so
its repair edits characters — the second one ever to do so, after
`literal-bullet`, and it inherits that rule's caution: `--strip-tatweel`,
opt-in, because deleting a character the author typed is not the same as
changing an attribute they left blank. Whether stripping is safe turned out
to be the same question as the threshold, and it is asserted against
`joining::forms` rather than restated: deleting a run a join crosses changes
no letter's form, however long the run is. A *stretched* run at a word edge
is not form-preserving — a noon padded to a width reverts to a plain noon —
and that is the repair rather than a loss, because a plain noon is what the
unpadded word says. Nothing else is ever deleted, so the `Carrier` and
`Displayed` cases are safe by never being touched.

**A warning, not an error**, which is ADR 0004's severity table read
honestly: the text renders exactly as the author arranged it. What is wrong
is the string behind it, and every cost is one a screenshot cannot show — a
search for the heading no longer matches the heading, a spell-checker does not
know the word, a screen reader reads the padding aloud, and the width it was
measured against is gone the moment the box, the font or the point size
changes.

*The adapter learned one thing.* `RemoveControls` and `RemoveTatweel` both
carry byte offsets into the text *as scanned*, so a paragraph with a bidi
control in a padded heading would have had the second set applied to a string
the first had already shortened. `rewrite.rs` now deletes from one merged,
descending sequence, and the case is committed in both fix orders — the
planner's emission order is not a contract the rewriter may lean on.

*Acceptance:* met, in the conformance suite, so it holds for `.pptx` and
`.docx` alike. A heading padded with five typed tatweel comes back a
`tatweel-padding` warning naming the run's offset and length in both formats;
the same paragraph justified — where the kashida is the font's and the string
holds none of it — comes back with nothing, as do a lone fatha on a tatweel,
medial heh as a primer shows it, and a rule drawn with four of them. Through
the writer, a padded heading in a run of its own is repaired to the word it
was, and re-audits clean.

---

## M5–M6 — Web, spreadsheets, PDF `[ ]`

Adapters only; no core changes expected. If one is needed, record an ADR
explaining what the core got wrong.

PDF implements `DocumentReader` **only**.

---

## M7 — Distribution `[ ]`

- [ ] `cargo-dist` or a release workflow producing five static targets
- [ ] Publish `mirsam-core`, `mirsam-ooxml`, `mirsam-fonts`, `mirsam-cli` to
      crates.io
- [ ] JSON schema, versioned, with a compatibility test
- [ ] SARIF renderer
- [ ] Agent skills: one `SKILL.md` per format over one binary

---

## Standing rules

1. **The round-trip test is sacred.** No repair merges while it is red.
2. **A rule that fires on formatting the author chose is a bug**, not a
   preference. `Resolved::Inherited` is evidence of a choice only where it
   agrees with the text; one that contradicts it is a template default and is
   reported as an absent value is
   ([ADR 0007](adr/0007-an-inherited-default-is-not-a-choice.md)).
3. **Findings carry evidence.** A diagnostic a reviewer cannot verify without
   opening the app is not finished.
4. **Report only what was verified.** `NOT RUN` is an honest result; inferred
   compatibility is not. Inherited from this project's prior art, and
   non-negotiable.
5. **Adapters lower; the core decides.** Format vocabulary in `mirsam-core` is
   a design failure, however convenient.
