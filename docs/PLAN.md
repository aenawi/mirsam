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
