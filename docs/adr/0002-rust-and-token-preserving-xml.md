# 2. Rust, and a token-preserving XML strategy

**Status:** accepted · 2026-09-02

## Context

The tool must edit a handful of attributes inside OOXML packages and leave
every other byte untouched. Three ecosystems were candidates: Python (the prior
art's choice), Go, and Rust.

The requirement is unusually strict because OOXML's Markup Compatibility layer
references namespace prefixes **by name, as attribute string values**
(`mc:Ignorable="p14"`). Renaming a prefix yields a file that is still
well-formed XML and is rejected by PowerPoint — a failure no schema check
catches.

Both alternatives were measured on a representative slide, round-tripped with
no intentional modification:

| Ecosystem | Result |
|---|---|
| Rust `quick-xml` | byte-identical; a surgical edit produces a two-attribute diff |
| Python `ElementTree` | prolog rewritten (`standalone` dropped), prefixes renamed for unregistered namespaces |
| Go `encoding/xml` | `xmlns:a` → `_xmlns:a`, `mc:Ignorable` → `_:Ignorable`, default `xmlns` re-declared on every element — invalid |

Go's behaviour is a long-standing limitation of `encoding/xml`, not a
misuse. The main third-party alternative, `unidoc/unioffice`, is commercially
licensed.

Python remained viable, but only by adding `lxml`, `python-pptx`,
`python-docx`, `openpyxl`, `uharfbuzz` and `python-bidi`. The prior art's
central advantage was being **stdlib-only and therefore dependency-free**. Once
that dependency tree is required, the advantage is gone.

## Decision

Rust, with a streaming token rewriter (`quick-xml`) rather than a document
object model.

Secondary confirmation: `unicode-bidi` (UAX#9, as used by Servo) and
`rustybuzz` (a pure-Rust HarfBuzz port) make real bidi resolution and real
shaping verification available inside a single static binary — the capability
this tool is actually for.

## Consequences

- **Cost:** distribution. Agent skills install by `git clone` with no build
  step, so releases must ship five prebuilt targets plus a download shim. This
  is the one thing Python genuinely did better, and it is a real recurring cost.
- **Cost:** a smaller OOXML ecosystem. Mitigated by not needing a document
  object model in the first place.
- **Benefit:** the "surgical edit, preserve the rest" requirement is expressed
  directly rather than fought.
- **Benefit:** no runtime, no interpreter, no package manager on the target
  machine — the actual product requirement.
