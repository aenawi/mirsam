# Changelog

All notable changes to this project are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Next: byte-preserving `repair` for PPTX. See [`docs/PLAN.md`](docs/PLAN.md).

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
