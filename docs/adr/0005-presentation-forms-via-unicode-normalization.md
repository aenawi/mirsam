# 5. Presentation forms are mapped through `unicode-normalization`, one character at a time

**Status:** accepted · 2026-09-03

## Context

`Fix::NormalizePresentationForms` replaces pre-shaped Arabic Presentation
Forms — U+FEF2 for a final yeh, U+FEFB for a lam-alef ligature — with the
logical-order codepoints they stand for. It was the last of the seven `Fix`
variants without an adapter, refused by the PPTX writer with a message saying
the mapping is NFKC and `mirsam-core` cannot do NFKC without a dependency.

The constraint being guarded is real. `mirsam-core` is the domain: no I/O,
and a dependency tree small enough to audit. `PLAN.md` made adding to it an
ADR rather than a `cargo add`, and this is that ADR.

Three options were on the table.

1. **Add `unicode-normalization` and call `nfkc()` on the run.** One line,
   the obvious reading of "the mapping is NFKC", and wrong. NFKC is a
   whole-string transform. On a run containing one presentation form it also
   composes canonical pairs the author typed (U+0627 U+0653 becomes U+0622),
   expands compatibility characters of other scripts in the same run (U+FB01
   ﬁ, U+00B2 ², U+2460 ①, fullwidth Latin) and expands the word ligatures
   (U+FDFA ﷺ becomes eighteen codepoints; U+FDFC ﷼ becomes four letters).
   Every one of those is a change no finding named, in a tool whose repair
   contract is that it changes only what a finding named.
2. **A generated table.** The two blocks hold 731 assigned codepoints with an
   `<isolated>`, `<initial>`, `<medial>` or `<final>` decomposition; a script
   over `UnicodeData.txt` could emit them into core with no runtime
   dependency, cross-checked in a test against the crate as a dev-dependency.
   Precise, and about 125 KB smaller. Also a table to maintain, a generator
   to maintain, and the crate in the tree anyway — for a saving nobody has
   asked for.
3. **Add `unicode-normalization` and call it per character, on the flagged
   codepoints only.** Measured on a scratch crate (release, LTO, strip,
   actually calling the API):

   | | |
   |---|---|
   | transitive dependencies | 2 (`tinyvec`, `tinyvec_macros`) |
   | binary size | +≈125 KB |
   | compile time | +≈2 s |

   Same maintainers (unicode-rs) as `unicode-bidi`, which core already
   trusts for the harder problem. Core already carries `syn`, `quote` and
   `proc-macro2` at build time through `serde_derive` and `thiserror-impl`,
   so this is not the heaviest thing in its tree.

Not heavy, then. The real question was never the weight of the crate but the
width of the call.

## Decision

Option 3. `mirsam-core` depends on `unicode-normalization`, and uses it in
exactly one way: `script::logical_form(c)` is NFKC of the single character
`c` in isolation, and `None` unless `c` is in the presentation-form blocks,
is not a word ligature, and actually changes. That single-character NFKC is
compatibility decomposition followed by canonical composition, so U+FE83
comes back as U+0623 ALEF WITH HAMZA ABOVE, the codepoint any keyboard
stores, rather than as alef plus a combining hamza. Nothing outside the
character takes part.

Three consequences are decided here rather than left to the code.

- **The rule and the repair share one predicate.** `is_presentation_form(c)`
  is defined as `logical_form(c).is_some()`. Before this, it was a range
  check over U+FB50–U+FDFF and U+FE70–U+FEFF, which also matched U+FEFF (a
  byte-order mark that leaked into a run), the ornate parentheses, the
  pedagogical symbol dots, the bismillah ligature and sixty unassigned
  codepoints — forty-one assigned characters no normalisation can change. A
  run carrying a stray BOM was reported as pre-shaped text and marked
  fixable; the repair would have applied, changed nothing, and the after-audit
  would have reported it again. That is the honesty failure `AGENTS.md`
  forbids, and it is closed by construction: a character is a presentation
  form exactly when the repair will change it.
- **Word ligatures U+FDF0–U+FDFF are reported and never expanded.** ﷺ, ﷼, ﷽
  and their kin are content the author chose. They are reported as a
  *warning* under the same rule — many fonts lack the glyph, and a search for
  the spelled-out phrase will not match it — with no fix attached, so a deck
  that uses them is never told it was repaired when it was not. Severity
  follows ADR 0004: the text renders and reads correctly, so this is not an
  error.
- **The mapping composes within the character, not across it.** If the
  stored text is U+FE8D followed by a combining madda the author placed,
  the result is U+0627 U+0653, not U+0622. Canonically equivalent, and the
  author's sequence; composing across the boundary would be a change nobody
  asked for. Idempotent by construction, which the fixed-point test relies on.

## Consequences

- **Cost:** one more runtime dependency in core, two transitive, ≈125 KB.
  `docs/ARCHITECTURE.md` now says four dependencies where it said three.
- **Cost:** the rule reports fewer characters than it did. That is the
  point, but it is a change in what is reported, and the golden corpus
  records it.
- **Benefit:** `repair` expresses every `Fix` variant; `repairs.skipped` is
  empty on every corpus deck; M1's last work item lands.
- **Benefit:** the dependency question for M4 has a precedent. `rustybuzz`
  and `ttf-parser` are far heavier and will not go in core; the shaping
  milestone gets its own crate, and this ADR is the line it is drawn against.
- **Not decided here:** whether a stray BOM in a run deserves a finding of
  its own. It is no longer misreported as a presentation form; it is not yet
  reported as anything.
