# AGENTS.md — mirsam

Instructions for AI coding agents (Claude, Codex, Grok, OpenCode, Pi, …) using
or extending this tool.

## What this is

**mirsam** is an independent Rust CLI that audits and repairs Arabic
right-to-left, bidirectional and typography defects in documents. Single static
binary: no Python, no Node, no Office, no runtime of any kind on the target
machine.

Inspired by Sultan Alsafran's MIT-licensed `arabic-presentations` skill. No
shared code. See [`CREDITS.md`](CREDITS.md).

## Using it

```bash
mirsam audit deck.pptx --format json     # full diagnostic model
mirsam audit deck.pptx --strict          # warnings block too
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

### Repairing

`repair <in> <out>` never modifies `<in>` and refuses `<out> == <in>` under
every flag. It changes only what a finding named and copies every other part
across as its original compressed bytes. Its exit code is the audit of
`<out>`, re-read from disk.

The JSON report carries `options`, `repairs.applied`, `repairs.skipped` (a
fix the adapter cannot express yet — listed, never claimed), and `before` and
`after`, each an audit in the same shape `audit --format json` emits.

Two repairs need a decision the text cannot supply, so they are off until
asked: `--font <TYPEFACE>` for `complex-font-missing`, and `--convert-bullets`
for `literal-bullet`. A `literal-bullet` finding in `after` with
`convert_bullets: false` in `options` is not a failed repair; it is one you
did not request.

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
register it in `Engine::with_options`. Nothing else changes. If its repair
needs a choice the text cannot make, the choice is a `RepairOptions` field,
not a CLI flag with logic behind it.

**Adding a format** — new crate implementing `DocumentReader`; add
`DocumentWriter` only if the format can be faithfully edited in place.

**Any change to what is reported or written** shows up in the golden corpus:
`cargo test` compares every deck under `tests/fixtures/` with its committed
`<deck>.expected.json` and fails on any difference. When the difference is
intended, run `make golden`, read the diff, and commit the regenerated
reports with the change that explains them. Never regenerate to make a
failure you do not understand go away. A new deck is one file dropped into
that directory plus `make golden`.

### Non-negotiable invariants

1. **`mirsam-core` performs no I/O.** If it needs to open a file, the design
   has gone wrong.
2. **A rule that fires on `Resolved::Inherited` is a bug.** Inherited
   formatting is the author's layout choice.
3. **Repairs are byte-preserving.** Everything a `Fix` does not address passes
   through untouched. The round-trip test guards this; it must stay green.
4. **Never insert bidi control characters.** Direction belongs to the
   container. Never strip ZWJ or ZWNJ — they are meaningful in Arabic and
   Persian orthography.
5. **Never reverse strings or emit presentation forms.** Storage is always
   logical-order Unicode.
6. **Findings carry evidence.** A diagnostic a reviewer cannot verify without
   opening the application is not finished.

## Before pushing

```bash
make verify     # version check, fmt, clippy -D warnings, tests, build
```

The `pre-push` hook runs this; enable it with `make hooks-install`.
