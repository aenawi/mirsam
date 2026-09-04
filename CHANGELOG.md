# Changelog

All notable changes to this project are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Working towards byte-preserving `repair` for PPTX. See
[`docs/PLAN.md`](docs/PLAN.md).

### Added

- **`mirsam-ooxml::docx`** — a Word reader. `mirsam audit report.docx` now
  works, reading `w:p`, `w:pPr/w:bidi`, `w:jc`, `w:lang/@w:bidi` and
  `w:rFonts/@w:cs` (with `@w:ascii`) from every `word/**/*.xml` part, so a
  header, a footer and a footnote are audited alongside the body. No line of
  `mirsam-core` changed to accommodate it, which is what M3 set out to test.
  `DocumentWriter` is deliberately not implemented yet.
- Word's `w:jc` is lowered as **direction-relative**: `left`/`right` become
  `Start`/`End`, because Word evaluates them against `w:bidi` whatever the
  standard says ([MS-OE376] Part 4 §2.3.1.13, note b). One consequence is
  worth stating outright — `alignment-incoherent` is structurally silent on
  DOCX, since a Word author cannot write the defect it reports. Reading
  `left` as a physical edge would have raised it on every left-aligned Arabic
  paragraph in Word.
- `is_true` moved from `pptx` to `token`, the format-neutral layer. `ST_OnOff`
  is defined once in ECMA-376 and names no element, so both vocabularies now
  read a document's booleans through one definition rather than Word's reader
  depending on PowerPoint's.

[MS-OE376]: https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oe376/26ecf09a-0f0b-4574-9907-ebd1ddf3015f

- **`mirsam-ooxml::package`** — the shared OOXML package layer, and the
  round-trip guarantee the repair milestone is built on. A rewrite copies every
  entry it was not asked to edit as already-compressed bytes, so no part can be
  silently re-encoded. Refuses to overwrite its own source, writes through a
  temporary and renames into place, and rejects an edit naming a part the
  package does not contain rather than discarding it.
- `tests/fixtures/torture.pptx`, the M1 acceptance deck: `mc:AlternateContent`,
  an embedded chart and its `.xlsx` workbook, speaker notes, a
  percent-encoded item name, and four compression settings across 25
  entries. Reproducible via
  `scripts/make-torture-fixture.py` (`make fixtures`).
- `tests/fixtures/clean.pptx`, a correctly marked deck the tool must leave
  alone, so exit code `0` from `audit` is provable rather than assumed.
- A CLI suite covering the exit-code contract, which previously had no test at
  any level.

- **`mirsam-ooxml::rewrite`** — token-stream repair for every `Fix` variant.
  Attributes are spliced in their raw bytes rather than rebuilt, so
  neighbouring attributes keep their exact quoting; inserted children are
  placed by DrawingML schema rank.
- **`NormalizePresentationForms` is expressed.** Pre-shaped Arabic
  Presentation Forms in a run are replaced by the logical-order codepoints
  they stand for, one character at a time, through
  `mirsam_core::script::normalize_presentation_forms`; a run without a form
  is not rewritten and keeps its character references verbatim. Hamza forms
  come back precomposed (U+FE83 becomes U+0623), and nothing beside a form —
  a combining mark the author placed, a Latin ligature, a word ligature — is
  touched. `mirsam-core` gained `unicode-normalization` for this, used only
  on single flagged characters; the decision, the measured cost and the
  reason whole-string NFKC was rejected are in
  [ADR 0005](docs/adr/0005-presentation-forms-via-unicode-normalization.md).
  On the corpus deck this moves the one skipped repair to applied, and the
  written deck audits clean under `--strict`.
- **`alignment-unset`** and **`repair --align`.** Right-to-left text with
  no alignment of its own takes one from a layout the tool cannot yet read;
  on an English template that is the left edge. It is reported as a *note*
  — never blocking, `--strict` or not — and repaired only when asked, with
  `--align`, because what the paragraph inherits may be a layout's design.
  Decided on visual evidence from the first application check (#6, #8) and
  recorded in
  [ADR 0006](docs/adr/0006-judge-from-the-text-not-the-template.md): a
  paragraph is judged from its own letters, never from an assumed template.
  The golden corpus repairs under `--align`, and the repaired corpus deck
  now carries the alignment its correctly authored twin has.
- **`container-direction`.** A container is a unit of its own kind beside the
  paragraphs inside it: its text is the text it lays out, its direction is its
  own, and that direction decides which side the reader starts on. Two
  containers so far — a **table** (`a:tblPr/@rtl`, which decides whether the
  first column sits on the right or the left) and a **text body in two or
  more columns** (`a:bodyPr/@rtlCol`, which decides which column the reader
  starts in; a single-column body is not a container, because `rtlCol` on it
  changes nothing a reader sees). A container whose text reads right-to-left
  but which declares no direction, or declares the contrary one, is a warning
  with a repair that sets that direction and nothing else — the paragraphs
  inside stay the paragraph rules' business, because DrawingML does not make
  them inherit from their container. Judged from the letters, like a
  paragraph. Tables were found by a person looking at the repaired corpus
  deck (#8 follow-up); columns are [#12](https://github.com/aenawi/mirsam/issues/12).
  Unit ids are `<part>#tbl<n>` and `<part>#cols<n>`; `Rule::applies_to` lets
  a rule say which kind it judges, so no paragraph rule ever sees a
  container. The corpus's broken decks carry a table and a two-column body
  with no direction so the rule is exercised, and their correctly authored
  twins keep `rtl="1"` and `rtlCol="1"`.
- **Chart text containers**, the third kind of container and the first whose
  text is not paragraphs at all
  ([#18](https://github.com/aenawi/mirsam/issues/18)). A chart's category
  labels and series names are cached strings, not `a:p` elements, and the
  direction they are drawn in belongs to `c:catAx/c:txPr`,
  `c:legend/c:txPr` or `c:dLbls/c:txPr` — which most generated charts do not
  have at all, so Arabic labels are drawn with no direction selected. Found
  by a person opening the corpus deck in PowerPoint 2016. `mirsam-ooxml::chart`
  is a second pass over any part whose root is `c:chartSpace`; ids are
  `<part>#catax<n>`, `#legend<n>` and `#dlbls<n>`, and the repair creates the
  whole `c:txPr` in schema position when there is none. What a container
  draws is read from the file rather than assumed: an axis draws the
  categories of the chart that names it in `c:axId`, a legend its series'
  names (or its categories, on a pie), data labels whichever of those their
  `c:showCatName` / `c:showSerName` flags turn on. A value axis draws
  numbers and the chart-level `c:txPr` draws nothing of its own, so neither
  is a unit. The torture deck's chart already carried the defect; its report
  and its repair now show it.
- **`direction-mismatch` gains a warning tier.** An explicit direction
  contrary to the text's own is reported even when the letter order comes
  out identical — pure Arabic marked left-to-right — because the paragraph
  direction still decides which edge is the start and where punctuation
  lands, and no alignment repair can be lowered correctly while it is
  wrong. The error tier, proven by two differing renderings, is unchanged.
- **`mirsam repair <in> <out>`** — writes a repaired copy and audits it,
  reporting the audit of the input beside the audit of the file actually
  written. `--lang` chooses the language tag, `--font` the complex-script
  typeface, `--convert-bullets` opts into replacing typed bullets with native
  lists, `--force` replaces an existing output. Refuses to overwrite its
  input under any flag, refuses an existing output without `--force`, and
  refuses an output whose extension differs from the input's. The exit code
  follows the after-audit, with `--strict` promoting warnings, so CI can run
  `repair` where it ran `audit`. `--format json` carries the options, every
  repair applied, every repair the adapter could not express, and both
  audits.
- **`DocumentWriter` for PPTX.** `PptxDocument::apply` groups repairs by part
  and paragraph, rewrites each part once and stages nothing unless every part
  succeeds; `write` copies every unedited entry raw. The port gained a default
  `supports` method, so a fix an adapter cannot express yet is reported as
  not applied instead of failing the whole run.
- **`RepairOptions`** and `Engine::with_options` — the authoring decisions a
  repair needs (language tag, complex-script typeface, whether to convert
  bullets), configured on the rules that propose the fixes rather than in the
  CLI. `complex-font-missing` becomes fixable only once a typeface is chosen.
- `Fix` and `Alignment` implement `Display`; `Repair` and `RepairOptions`
  serialise.
- **The golden corpus** (`crates/mirsam-cli/tests/golden.rs`). Every deck
  under `tests/fixtures/` has a committed `<deck>.expected.json` recording
  the audit, the repair under `--convert-bullets --font Dubai`, a tag-level
  diff of every package entry the repair changed, and the exit codes. The
  suite regenerates each report and fails on any difference, showing it as a
  diff; a deck without a report fails, and so does a report without a deck.
  `make golden` regenerates the reports and refuses to under `CI`, so a
  change in behaviour can only land inside the commit that explains it.
- `tests/fixtures/quarterly-report.pptx`, the first corpus deck shaped like
  a real one — six slides on PowerPoint's default template, with every
  defect the rule set knows spread across placeholders, a text box, a
  grouped text box, a table and speaker notes, and one paragraph pasted
  from a PDF with pre-shaped presentation forms — and
  `quarterly-report-correct.pptx`, the same deck authored correctly, which
  the tool leaves completely alone. Generated deterministically by
  `scripts/make-corpus.py` (`make corpus`).

- **`mirsam-ooxml::rels`** — the package relationship graph, and the first
  half of the inheritance milestone. It reads every `_rels/*.rels` and
  answers, for any part, which parts it inherits from and in what order:
  slide → layout → master → theme, notes slide → notes master → theme.
  `PptxDocument::relationships()` exposes it. No report changes yet — walking
  the chain to resolve properties is the next item — so the golden corpus is
  untouched.

  What a part *is* comes from the relationship type pointing at it, never
  from its directory: OPC does not require `ppt/slides/`, and a name match
  cannot settle the ambiguity that matters — a slide master relates to its
  layouts as well as its theme, so following the first relationship upward
  walks a master back down into a layout. Types are matched against the full
  standard namespace, so a Microsoft extension of the same name is not
  mistaken for a standard one. Targets resolve to the item names the ZIP
  actually stores, percent-encoding included, because the only useful thing
  to do with a resolved target is read that part. A cycle terminates the walk
  instead of hanging.

- **`mirsam-ooxml::inherit`** — property chain resolution, the second half of
  the inheritance milestone and the answer to #8. A paragraph that states no
  direction or alignment of its own now takes one from the list style on its
  shape, from the matching placeholder on its slide layout, from the same
  placeholder on the master, and last from the master's `titleStyle`,
  `bodyStyle` or `otherStyle` — `notesStyle` on a notes slide. A shape that is
  not a placeholder at all takes `otherStyle`; the placeholder match follows
  `@idx` first, with an absent `@idx` meaning index zero and `@type`
  defaulting to `body` as the schema defaults it.

  Resolution happens in the adapter, so `mirsam-core` still performs no I/O.

- **Nine list levels, selected by `a:pPr/@lvl`.** Every style source states its
  properties once per level, `a:lvl1pPr` through `a:lvl9pPr`, and a paragraph
  now reads the level it is actually at rather than always the first — at each
  hop of the walk, not only at the master. `@lvl` is zero-based, so `lvl="1"`
  reads `lvl2pPr`, and evidence cites the level it read:
  `ppt/slideMasters/slideMaster1.xml bodyStyle/lvl2pPr@rtl`. A level a source
  does not state is not answered by that source's first level — PowerPoint's
  own fallback there is its application default — so the walk carries on
  instead of reporting a value no reader will see.

- **The complex-script font slot resolves, through the theme where it has to.**
  A master naming a typeface in `a:lvlNpPr/a:defRPr/a:cs` now supplies it to
  the paragraphs below, and a real master names a *reference* — `+mn-cs`, into
  the theme's `a:fontScheme` — which `mirsam-ooxml::inherit::FontScheme`
  reads through `RelationshipGraph::theme_of`. A finding names the theme
  rather than the master that pointed at it, because the theme is where the
  typeface a reader will see can be checked in one look.

  A reference the theme answers with `<a:cs typeface=""/>` — which is what the
  stock Office theme states — resolves to nothing rather than to the empty
  string, so `quarterly-report.pptx` keeps all four of its
  `complex-font-missing` warnings. The Latin slot is deliberately not
  inherited: `complex-font-missing` fires only where a Latin font is chosen,
  and inheriting a template's `+mn-lt` would manufacture that precondition on
  every Arabic paragraph in every deck.

  An inherited `lang` is still unresolved, and deliberately: ADR 0007's
  agreement test is stated for direction and alignment, and there is no
  decided answer for an inherited language tag that disagrees with the letters.

### Changed

- **The token-rewrite scaffold is now `mirsam-ooxml::token`**, a module that
  names no element (PLAN M3 3.1). Reading a part into events and writing it
  back, `passthrough`, the raw-byte attribute splice, element lookup and
  ranges, schema-ranked child creation and run-text rewriting all moved there
  from `rewrite`, which keeps its public API and is now what it always was:
  DrawingML's repair vocabulary, performing no XML editing of its own. Where
  the scaffold had `a:t` it now takes the element name as an argument, and
  `remove_controls` / `normalize_presentation_forms` became the vocabulary-free
  `remove_at_offsets` / `map_runs`. A second OOXML format reuses this rather
  than reimplementing it, exactly as it already reuses `package`. Sixteen new
  tests drive the scaffold entirely in WordprocessingML, which is the only way
  to prove no DrawingML name is left in it. No report or byte of output
  changed.
- `rewrite::passthrough` moved to `token::passthrough`.
- **An inherited value that agrees with the text now silences its finding, and
  one that contradicts it keeps it** ([ADR 0007](docs/adr/0007-an-inherited-default-is-not-a-choice.md)).
  Arabic under an `rtl="1"` master is the layout doing its job and is no
  longer reported; Arabic under an English template's untouched `rtl="0"` is a
  default nobody aimed at the text and is reported exactly as an absent one
  is, at the same severity. `alignment-unset` reads the same way: a layout
  that centres or right-aligns is silent, `algn="l"` under right-to-left text
  is not. That retires ADR 0006's cost note — `repair --align` no longer
  proposes pushing a centred title to the right edge.

  In the corpus: the three RTL-mastered decks lost every paragraph-level
  `direction-unset` and `alignment-unset` finding they had;
  `quarterly-report.pptx` kept all seven `direction-unset` warnings with the
  reason changed, and lost the four `alignment-unset` notes on its centred
  titles; `quarterly-report-correct.pptx` is still clean. Every committed
  report regenerated.

- **`Evidence` gains `inherited_from`**, naming the part and property that
  supplied a value the unit did not state —
  `ppt/slideMasters/slideMaster1.xml bodyStyle/lvl1pPr@rtl`. A finding on an
  inherited value is not checkable without it. The field is additive and
  `null` on every finding about a value the unit stated itself, so the JSON
  report gains a key and loses none.

- **`Resolved::Inherited` carries an `Origin`.** `Inherited(T)` is now
  `Inherited(T, Origin)`, where `Origin` is the part and property that
  supplied the value. Source-breaking for anything constructing or matching
  the variant; `Resolved::effective`, `is_unset` and `is_explicit` are
  unchanged, and `origin()` and `is_inherited()` are new.

- **`direction-mismatch` does not judge an inherited direction.** It reports a
  direction the author wrote, or — where none is written — what the renderer
  would auto-detect, as it always has. Feeding it the resolved chain instead
  would turn five of `quarterly-report.pptx`'s warnings into errors on the
  strength of a template default; those paragraphs are reported by
  `direction-unset` at warning severity, naming the master (ADR 0007 §3).

- `pptx::scan_xml` resolves nothing, because a caller holding one part has no
  package and so no chain. `pptx::scan_xml_with` takes a `StyleIndex` for
  callers that do.

- **A theme reference is no longer reported as a typeface.** A run writing
  `<a:cs typeface="+mn-cs"/>` used to put `complex_font: "+mn-cs"` in the
  report, naming a font nobody has. It now resolves through the theme, or —
  where there is no theme to read, as in `pptx::scan_xml` — stays `Unset`.

- `tests/fixtures/torture.pptx` writes its master's complex-script slot both
  ways a real deck does: `+mj-cs` in `titleStyle` and `+mn-cs` in `bodyStyle`,
  the literal `Dubai` in `otherStyle`. A byte change with no report change, so
  the corpus exercises the theme reference rather than only the literal name.

### Fixed

- **`make validate-fixtures` reported a part that was there (#21).**
  `check_container` unquoted a relationship target and then looked for the
  result among the *raw* ZIP item names, so every percent-encoded part looked
  missing — `torture.pptx` had been red since it gained one. A relationship
  resolves against part names, and a part name is the decoded item name;
  both sides are decoded now, as `check_package` in
  `make-torture-fixture.py` already did. `scripts/validate-ooxml.py
  --self-test` guards the repair with three in-memory packages — an encoded
  target that resolves, and an encoded and a plain one that do not — so a
  check that stops reporting anything fails as loudly as the bug did.
  `make validate-fixtures` runs it before the corpus.
- **The audit was silent on right-to-left paragraphs left on the left edge
  by their layout (#8).** Found by a person opening the repaired corpus deck
  beside its correctly authored twin: the only difference on every Arabic
  paragraph was `algn="r"`. See `alignment-unset` and the `direction-mismatch`
  warning tier above.
- **`presentation-forms` reported characters no repair could change.** The
  predicate was a range check over the two Presentation Forms blocks, which
  also matched U+FEFF (a byte-order mark that leaked into a run), the ornate
  parentheses, the pedagogical symbol dots and sixty unassigned codepoints.
  A run with a stray BOM was reported as pre-shaped text and marked fixable;
  once the repair existed it would have applied, changed nothing, and been
  reported again by the after-audit. A character is now a presentation form
  exactly when the repair will change it, so the rule and the repair share
  one definition.
- **`Fix` did not serialise.** It was declared internally tagged, which
  serde cannot do for a newtype variant carrying a string or a list:
  `SetDirection(Rtl)` came out as `{"kind":"set_direction","rtl":null}` and
  `RemoveControls` failed outright. Nothing emitted it before `repair`. It is
  adjacently tagged now, `{"kind": …, "value": …}`, matching `Resolved<T>`.
- A direction-relative alignment repair was lowered against the paragraph's
  own `rtl` attribute alone. A left-aligned paragraph inheriting its
  direction from its body therefore had `Start` lowered to the *left* edge —
  the defect being repaired. The writer now passes the scanner's resolved
  inheritance into the rewriter.
- The rewriter applied text repairs in the order the plan listed them, so a
  bullet conversion ahead of a control removal shifted the control's offset
  and the removal missed. Controls are now always removed first.
- `language-missing` accepted any tag beginning with `ar`, including `arn`
  (Mapudungun). The primary subtag itself must now be `ar`.

- **Text written as character or entity references was invisible.** quick-xml
  reports `&#1585;` as an event of its own, and the PPTX scanner read only
  `Event::Text`, so a run stored that way — a routine encoding for Arabic in
  Office documents — arrived empty and was dropped as a blank paragraph. Such
  paragraphs produced no text unit and therefore no findings: a deck could be
  thoroughly defective and audit as clean. The scanner and the rewriter both
  read `Event::GeneralRef` now.
- `alignment-incoherent` proposed `Alignment::End` for right-to-left text
  aligned left. The end edge in RTL *is* the left edge, so applying the fix
  reproduced the defect it reported. It now proposes `Alignment::Start` — the
  side reading begins on — which stays direction-relative, as intended.
- `audit` on a format mirsam does not read yet returned `3` (document
  unreadable) where `README.md` and `AGENTS.md` both promise `2` (bad
  invocation). Exit codes are now selected by matching the error's *type*
  rather than by searching its rendered message, so the contract no longer
  depends on wording — or on text that happens to appear in a user's document.
- A missing file reported itself as an "unsupported or malformed document",
  which it is not, and repeated its own path. It now has its own error variant.

### Changed

- **Word ligatures are reported, not expanded.** U+FDF0–U+FDFF (ﷺ, ﷼, ﷽ and
  their kin) are content the author chose; expanding ﷺ to the eighteen
  codepoints of its phrase would rewrite what they wrote. They are now a
  *warning* under `presentation-forms` — many fonts lack the glyph, and a
  search for the spelled-out phrase will not match — with no fix attached.
  Before, they were an error with a repair that could not be made.
- `presentation-forms` findings carry the offending codepoints as `U+XXXX`
  in `evidence.offenders`, so a reviewer can verify them without rendering
  the text.
- The PPTX adapter reads through `package::Package` instead of opening the ZIP
  itself, so audit and repair cannot drift apart. `pptx::read_part` and
  `pptx::source_path` are superseded by `Package::read_text` /
  `Package::read_bytes` and `PptxDocument::path`.
- A malformed package is now rejected when it is opened rather than when it is
  scanned. The exit code is unchanged (`3`).

- `scripts/validate-ooxml.py` and `make validate-fixtures` — validate every
  corpus deck against the published ECMA-376 transitional schemas, plus the
  OPC container around them. The schemas are fetched once into
  `target/ooxml-schemas/`; nothing is vendored. Not run in CI, because it
  needs the network; `corpus_packages.rs` covers the same ground there.
- `make fixtures` regenerates the hand-built decks and their reports, the way
  `make corpus` already did for the generated ones.

### Fixed

- **`torture.pptx` made PowerPoint 2016 offer to repair it** (#9, the
  remaining cause). The deck carried a media part named `صورة.png`, and
  PowerPoint 2016 does not resolve a relationship to a part whose name has
  a non-ASCII octet — raw UTF-8 or percent-encoded, in the item name, the
  `.rels` target or both, Arabic or a single Latin letter. It prompts to
  repair, and a picture shape showing the part reports "The picture can't
  be displayed". Found by a person on PowerPoint 2016 across a 23-deck
  bisect: additive, subtractive, then every encoding of that one name.
  Every one of those decks validated against the ECMA-376 schemas, so
  schema validity is now known to be necessary and not sufficient. The
  part is now `ppt/media/my%20image.png`: still percent-encoded, so a
  rewriter that decodes item names is still caught, and proven to open.
  The fixture guard asserts an encoded name is present and that no item
  name leaves ASCII; the generator's own check refuses one. What mirsam
  reports and writes on the corpus is unchanged.
- **The three hand-built corpus decks were not valid OOXML** (#9).
  `torture.pptx` made PowerPoint offer to repair it *before* mirsam touched
  it, which meant the M1 application check — "PowerPoint opens the result
  without a repair prompt" — could not be asked of it either way.
  `clean.pptx` and `broken-arabic.pptx` had the same class of defect. Against
  the ECMA-376 schema: every `p:spTree` was missing the `p:grpSpPr` required
  after `p:nvGrpSpPr`; the theme carried a font scheme but neither of the
  colour and format schemes, and its font collections had no `a:ea`; the bar
  chart had neither of its `c:axId`s nor the axes they name; the notes slide
  had no notes master; `docProps/core.xml` and `docProps/app.xml` were
  untyped and unrelated; `clean.pptx` and `broken-arabic.pptx` had
  relationships pointing at parts that were not in the package. All three are
  regenerated and now validate, and every deck keeps the hazards it carried —
  what mirsam reports and writes on the corpus is byte-for-byte unchanged, so
  no expected report moved.
- `crates/mirsam-ooxml/tests/corpus_packages.rs` asserts the structural half
  of "an application would open this" over the committed decks on every
  `cargo test`, so a regenerated fixture cannot quietly lose it again.

### Notes

- **Application check, run by a person on 2026-09-03 (#6).** PowerPoint
  opened the repaired `quarterly-report.pptx` without a repair prompt.
  `torture.pptx` prompted before any repair, so that half was inconclusive.
  The fixture was the cause and is fixed (#9). **Second pass, 2026-09-04,
  PowerPoint 2016 on Windows 10:** `clean.pptx`, `broken-arabic.pptx` and
  their repaired copies open without a prompt and render right-to-left; the
  repaired `quarterly-report.pptx` renders every Arabic slide right-to-left
  with the table's first column on the right; `torture.pptx` still
  prompted, and the bisect named the media item name above. The torture
  deck's left-to-right title before repair is its seeded
  `direction-mismatch`, and the repaired copy showed it right-to-left. The
  regenerated `torture.pptx` and its repaired copy, with the media part
  renamed, then opened without a prompt, the repaired one with its title
  right-to-left. The M1 application check is verified on every corpus
  deck. The same pass found chart category-axis labels with no text
  properties and no direction, which no rule sees yet. The repaired deck's
  Arabic paragraphs keep the template's left alignment, which the audit does
  not yet report (#8). The tests prove structural correctness; this is the
  record of what was actually seen.

- **Invariant 2 is narrowed, ahead of 2.2:**
  [ADR 0007](docs/adr/0007-an-inherited-default-is-not-a-choice.md). An
  English template's slide master says `rtl="0" algn="l"` in all three text
  styles, and both PowerPoint-authored corpus decks sit on that same master —
  including the correctly authored one, which inherits none of it and writes
  `rtl="1" algn="r"` on every Arabic paragraph itself. So a master's `rtl="0"`
  is a template default nobody aimed at the text, not a design decision. Read
  literally, "a rule that fires on `Inherited` is a bug" would make every
  Arabic finding on `quarterly-report.pptx` vanish the moment M2 can resolve
  the chain — the tool reporting *less* after learning to see more.
  `Inherited` now counts as a choice only where it *agrees* with the text:
  Arabic under `rtl="1"`, or under a layout that centres or right-aligns it,
  stays silent; `rtl="0"` or `algn="l"` under Arabic is reported as an absent
  value is, and names the part that supplied it. Decided by the maintainer on
  three questions worked through the corpus. `AGENTS.md` and `PLAN.md`'s
  standing rule 2 are restated to match. No code changed with the ADR; 2.2 is
  what implements it.

## [0.1.0] — 2026-09-02 · "Steppe Eagle"

Foundation release. Audit only, PowerPoint only — the architecture proven
end-to-end on one format before it is generalised.

### Added

- **`mirsam-core`** — Arabic script, bidi and typography engine with no I/O.
  - `Resolved<T>` property model distinguishing explicit, inherited and unset
    values, so inherited formatting is never reported as missing.
  - UAX#9 resolution via `unicode-bidi`: `resolve`, `auto_direction`,
    `dominant_direction`, `order_differs`.
  - Arabic script detection, presentation-form detection, and bidi-control
    scanning that deliberately preserves ZWJ and ZWNJ.
  - `Rule` trait and engine with eight default rules.
  - `DocumentReader` / `DocumentWriter` ports, split so read-only formats are
    never obliged to implement repair.
- **`mirsam-ooxml`** — PPTX reader over a `quick-xml` token stream.
- **`mirsam-cli`** — `audit`, `explain` and `rules`, with text and JSON output
  and stable exit codes (`0` clean, `1` findings, `2` usage, `3` unreadable).

### Notes

- `direction-mismatch` reports a defect only when the resolved visual order
  actually differs, rather than when an attribute is absent. On the reference
  fixture this yields one error where attribute-based checking reported eight.
- Inheritance resolution through layouts and masters is milestone M2; until it
  lands, an absent property is reported as a warning rather than an error.

[Unreleased]: https://github.com/aenawi/mirsam/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/aenawi/mirsam/releases/tag/v0.1.0
