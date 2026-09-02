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

### 1.1 Round-trip harness `[ ]`
Read a PPTX, stream every part through the writer applying **no** fixes, write
it out, assert byte equality with the input.

*Acceptance:* passes on a deck containing `mc:AlternateContent`, an embedded
chart, speaker notes and a non-ASCII filename inside the package.

*This is the foundation.* Everything below is only trustworthy if it holds.

### 1.2 `Fix` application in the token stream `[~]`
For each `Fix` variant, mutate the matching `BytesStart` and re-emit; pass
every other token through unchanged.

- `SetDirection` → `a:pPr/@rtl`, creating `a:pPr` in schema position if absent
- `SetAlignment` → `a:pPr/@algn`
- `SetLanguage` → `a:rPr/@lang` and `a:defRPr/@lang`
- `SetComplexFont` → `a:cs/@typeface`, inserted per `CT_TextCharacterProperties` order
- `RemoveControls` → rewrite `a:t`, applying offsets back-to-front
- `ConvertLiteralBullet` → strip the glyph, add `a:buChar`, set `marR`/`indent`
- `NormalizePresentationForms` → map presentation forms to logical codepoints

*Acceptance:* each variant has a test asserting the diff contains exactly the
intended change and nothing else.

*Trap:* DrawingML child order is schema-significant. Insert by rank against the
`CT_TextParagraphProperties` / `CT_TextCharacterProperties` sequences; a
correct attribute in the wrong position still fails to load.

### 1.3 `repair` command `[~]`
`mirsam repair <in> <out>` with `--lang`, `--font`, `--convert-bullets`,
`--force`; refuses `in == out`; re-audits the output and reports both.

*Acceptance:* repairing the M0 fixture clears every fixable finding, and a
second repair run is a no-op.

### 1.4 Golden corpus `[~]`
Real decks under `tests/fixtures/`, each with a committed expected report.

*Acceptance:* CI fails on any unexplained diff. Include at least one deck the
tool should leave completely alone.

---

## M2 — Inheritance `[ ]`

### 2.1 Package relationship graph `[ ]`
Parse `_rels/*.rels`; resolve slide → layout → master.

### 2.2 Property chain resolution `[~]`
Walk paragraph → placeholder (`p:ph/@type`,`@idx`) → layout → master → theme,
populating `Resolved::Inherited` instead of `Unset`.

*Acceptance:* a deck with direction set only on the master reports zero
`direction-unset` warnings — and the same deck reports them today, so the test
is written first and starts red.

### 2.3 List levels and theme fonts `[~]`
`lvl1pPr`…`lvl9pPr` by `a:pPr/@lvl`; `a:fontScheme` for the `cs` slot.

---

## M3 — Word `[ ]`

### 3.1 Extract the shared package layer `[ ]`
Lift ZIP handling, part enumeration and the token-rewrite scaffold out of
`pptx.rs` into `mirsam-ooxml::package`, leaving PPTX as vocabulary only.

*Do this by extraction, not by anticipation* — the shape is now known from one
working adapter, which is the right moment to generalise.

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
