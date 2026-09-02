# 3. Hexagonal ports and adapters

**Status:** accepted · 2026-09-02

## Context

Five document formats, one body of Arabic correctness knowledge. The obvious
structure — one tool per format — duplicates the rules five times, and they
drift. The rules are the valuable part and the formats are incidental, so the
structure should say so.

## Decision

A pure domain crate (`mirsam-core`) holding Unicode, bidi and the rule engine,
with **no I/O of any kind**. Formats implement `DocumentReader` /
`DocumentWriter` and lower their native structure into `TextUnit`s. The CLI is
a driving adapter holding no logic.

Repairs are expressed as a format-agnostic `Fix` enum: the domain decides what
must change, the adapter decides how to say it.

`DocumentReader` and `DocumentWriter` are separate traits because PDF can be
audited but must never be patched in place.

## Consequences

- A new rule touches one file and benefits every format at once.
- A new format cannot silently change correctness semantics.
- `mirsam-core` is testable with no fixtures — rules are pure functions.
- **Cost:** an adapter must lower faithfully, including resolving inheritance.
  That work is real and is where adapter bugs will live.
- **Cost:** one indirection between "the rule fires" and "the file changes".
  Accepted: it is what stops five adapters becoming five rule sets.
