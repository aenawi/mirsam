# Changelog

All notable changes to this project are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Working towards byte-preserving `repair` for PPTX. See
[`docs/PLAN.md`](docs/PLAN.md).

### Added

- **`mirsam-ooxml::package`** — the shared OOXML package layer, and the
  round-trip guarantee the repair milestone is built on. A rewrite copies every
  entry it was not asked to edit as already-compressed bytes, so no part can be
  silently re-encoded. Refuses to overwrite its own source, writes through a
  temporary and renames into place, and rejects an edit naming a part the
  package does not contain rather than discarding it.
- `tests/fixtures/torture.pptx`, the M1 acceptance deck: `mc:AlternateContent`,
  an embedded chart and its `.xlsx` workbook, speaker notes, a non-ASCII part
  name and four compression settings across 19 entries. Reproducible via
  `scripts/make-torture-fixture.py`.
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

### Fixed

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

### Notes

- **Application check, run by a person on 2026-09-03 (#6).** PowerPoint
  opened the repaired `quarterly-report.pptx` without a repair prompt.
  `torture.pptx` prompts before any repair, so that half is inconclusive
  (#9). The repaired deck's Arabic paragraphs keep the template's left
  alignment, which the audit does not yet report (#8). The tests prove
  structural correctness; this is the record of what was actually seen.

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
