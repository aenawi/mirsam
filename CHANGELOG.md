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

- **`mirsam-ooxml::rewrite`** — token-stream repair for six of the seven `Fix`
  variants. Attributes are spliced in their raw bytes rather than rebuilt, so
  neighbouring attributes keep their exact quoting; inserted children are
  placed by DrawingML schema rank. `NormalizePresentationForms` is refused with
  an explicit message pending NFKC support in `mirsam-core`.

### Fixed

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

- The PPTX adapter reads through `package::Package` instead of opening the ZIP
  itself, so audit and repair cannot drift apart. `pptx::read_part` and
  `pptx::source_path` are superseded by `Package::read_text` /
  `Package::read_bytes` and `PptxDocument::path`.
- A malformed package is now rejected when it is opened rather than when it is
  scanned. The exit code is unchanged (`3`).

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
