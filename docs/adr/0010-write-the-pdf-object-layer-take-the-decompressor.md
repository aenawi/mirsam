# 10. Write the PDF object layer; take the decompressor

Date: 2026-09-07

## Status

Accepted.

## Context

`docs/PLAN.md` §6.1 asks the question directly: *"Decide the parser: own it or
take one. This wants an ADR, not a `Cargo.toml` line."* The constraints it sets
are pure Rust, no C, no network, a permissive licence, and no rasteriser.

The precedent in this repository cuts both ways and says so. `dom.rs` and
`css.rs` were written here rather than taken, and the reasoning was specific:
`markup5ever_rcdom` describes itself as unsupported, and the part of CSS that
decides direction is a few hundred lines. But `html5ever` *was* taken, because
tree construction is a specification with observable, tested behaviour that a
hand-rolled nesting stack gets wrong in ways that move a node's ancestors — and
direction is inherited along ancestors.

So the question is not "does this project write parsers". It is which half of
this particular specification mirsam needs, and what a dependency would carry
in with it.

### What mirsam needs from a PDF

The object graph and the content streams. Concretely: the two spellings of the
cross-reference table, `/Prev` chains, hybrid-reference files, object streams,
the general-purpose stream filters, the page tree with its four inheritable
attributes, and the bytes of the content streams. Above that, §6.2 needs the
text operators and `ToUnicode` CMaps, §6.4 the embedded font programs — and
`mirsam-fonts::sfnt` already parses those.

### What a PDF library carries

Most of a PDF library is the half this project has said it will never do.
Graphics state, path construction, colour spaces, transparency groups, shading
patterns, image decoding, annotation appearance streams, form field rendering —
none of it is reachable from anything mirsam asks. The read-write libraries add
a serialiser, an incremental-update writer and an encryption implementation, all
for a format this project has committed to never writing.

There is a second cost, and it is the one that decided this. A library's object
model becomes the vocabulary of the adapter that uses it, and every subsequent
question — is a missing `/Type` an error, does a dangling reference resolve to
null, is a stream with a wrong `/Length` readable — is answered by that
library's judgement rather than by this project's. Those judgements are exactly
where a document reader is honest or is not. §6.2's whole difficulty is deciding
what the adapter is *entitled to claim* about a file that does not state its own
direction, and inheriting those decisions from a dependency would mean
inheriting them unexamined.

### And what only a library should supply

Inflate. `FlateDecode` is a codec, not a document format: it has one correct
answer, that answer is `zlib`'s, and a hand-written implementation would be a
liability with no upside. The same is true of nothing else in this crate.

## Decision

**`mirsam-pdf` owns the object layer.** The lexer, the object model, the
cross-reference table and stream, object streams, the page tree, the ASCII and
run-length and LZW filters and the PNG and TIFF predictors are written here,
against ISO 32000-1, with the section numbers in the comments.

**`flate2` is taken for `FlateDecode`**, held to the `zlib-rs` backend the
workspace already resolves through `zip`. That is the same rule `ttf-parser`
is pinned under: two inflate implementations in one binary would be two answers
to what a stream decompresses to, and no new dependency enters the tree.

**Three things are refused rather than implemented**, and the refusals are the
design rather than a gap:

- **Decryption.** An encrypted document is refused at `Pdf::open` with the
  algorithm named from the `/Encrypt` dictionary, which §7.6.1 keeps in the
  clear. A tool that opened protected documents by trying the empty password
  would be one nobody could run on a file they were given.
- **Rasterising.** `DCTDecode`, `JPXDecode`, `CCITTFaxDecode` and `JBIG2Decode`
  are named, never decoded. A stream behind one is not text this crate failed
  to read; it is text that is not in the file.
- **Writing.** There is no `DocumentWriter` here and none planned. A broken
  Arabic PDF is rebuilt from its source document.

## Consequences

**The honesty decisions are this project's, and they are visible.** A wrong
`/Length` is verified rather than believed; a dangling reference is null,
because §7.3.9 says so; a reference cycle terminates and answers null rather
than hanging; a stale `startxref` is recovered from by scanning, because a
document reported unreadable while every viewer opens it is the wrong half of
standing rule 4. Each of those is a line in this crate with a test beside it,
and each would otherwise have been a dependency's default.

**The unread list is native rather than bolted on.** ADR 0009 gave
`DocumentReader` a way to say what it could not read, and PDF is the format
that needs it most — a scanned page, an object at an offset that is not there,
a filter this crate does not implement. Because the loader is ours, each of
those is recorded where it happens, named as the document names it. Through a
library those cases are an `Err` or an empty `Vec`, and the difference between
"no text here" and "no text this tool can see" would have had to be
reconstructed from the outside.

**It is more code than a dependency line, and less than a PDF library.** Two
and a half thousand lines, comments included, against a specification of some
hundreds of pages — because the pages that matter here are §7.2 through §7.8,
and everything from §8 on is rendering. That ratio is the decision, and it
would not survive the day mirsam needed a graphics state.

**The bound is stated so it can be tested.** Nesting depth, reference-chain
length, `/Prev` chain length and page-tree depth all have explicit limits, and
a file that exceeds one is malformed rather than a stack overflow. mirsam is
run on files people were sent, which is the threat model a document tool has
whether or not it says so.

**The cost is real: this crate now owns every PDF bug it has.** A file that
opens in a viewer and not here is this repository's to fix, and the fixture
corpus is the answer to that — one file per generator family, written byte by
byte by `scripts/make-pdf-fixture.py` rather than by a library, so a fixture
can never prove only that the parser agrees with itself.

## Related

- Standing rule 4, `AGENTS.md` — "Report only what was verified."
- [ADR 0002](0002-rust-and-token-preserving-xml.md) — the same shape one format
  earlier: parse what is needed, keep the bytes, do not adopt a model.
- [ADR 0009](0009-a-source-the-adapter-could-not-read-is-part-of-the-report.md)
  — the port this crate's `unread` list feeds.
- `docs/PLAN.md` §6.1, which asked for this record.
