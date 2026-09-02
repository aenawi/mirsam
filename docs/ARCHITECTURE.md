# Architecture

## The shape of the problem

Five document formats disagree about almost everything. They agree on one
thing: each ultimately presents a **run of text with a base direction, a
language and a font**. Every Arabic correctness rule worth writing is a
statement about that shape, not about DrawingML or CSS.

So the architecture inverts the obvious layout. Instead of five format tools
that each grow their own rules, there is one rule engine and five adapters that
lower their format onto a shared model.

```
                    ┌─────────────────────────────────┐
   pptx ─┐          │          mirsam-core            │
   docx ─┤          │                                 │
   xlsx ─┼─ adapter │   TextUnit  ──►  Rule engine    │
   html ─┤   lowers │      ▲               │          │
   pdf  ─┘   into   │      │               ▼          │
                    │   Properties     Diagnostic     │
                    │   (Resolved<T>)   + Evidence    │
                    │                       │         │
                    │                       ▼         │
                    │                     Fix         │
                    └───────────────────────┼─────────┘
                                            │
                              adapter renders Fix into
                              its own native vocabulary
```

The domain decides **what** must change. The adapter decides **how** to say it.
Neither knows the other's vocabulary.

## Ports and adapters

| Port | Direction | Implemented by |
|---|---|---|
| `DocumentReader` | driven | every format adapter |
| `DocumentWriter` | driven | only formats that can be faithfully edited |
| CLI | driving | `mirsam-cli` |

`DocumentWriter` is deliberately separate from `DocumentReader`. PDF can be
audited but must never be patched in place — a broken Arabic PDF is rebuilt
from source, not layered over. Splitting the ports means the PDF adapter is
never obliged to implement a meaningless `apply`, and the type system enforces
it. This is Interface Segregation doing real work rather than decoration.

Driving adapters are cheap to add because the CLI holds no logic: a language
server, an HTTP service or a library caller would reuse the same engine.

## The central type: `Resolved<T>`

```rust
enum Resolved<T> {
    Explicit(T),   // stated on this unit
    Inherited(T),  // supplied by a layout, master, style or cascade
    Unset,         // nothing anywhere; the renderer picks
}
```

This three-state model is the single most important decision in the codebase.
Attribute-only linters collapse `Inherited` into `Unset` and therefore report
every placeholder that inherits centred alignment from its layout as a defect.
That false positive is what drives users to disable the tool.

Adapters are responsible for resolving the inheritance chain. Rules are
responsible for distinguishing the three states. A rule that fires on
`Inherited` is a bug.

## Proving rather than asserting

`direction-mismatch`, the flagship rule, does not ask *"is `rtl` set?"*. It
resolves the text under the declared direction and under its semantically
correct direction, and fires only when the two visual orders differ.

The consequences are worth stating plainly:

- **No false positives by construction.** If the rendering is identical, there
  is nothing to report, whatever the attributes say.
- **Findings carry evidence.** A reviewer can check the claim without opening
  PowerPoint, because the diagnostic contains both resolved orders.
- **Severity becomes meaningful.** Text that renders correctly today only by
  auto-detection is *fragile* (warning), not *broken* (error).

See [`adr/0004-prove-defects-dont-assert-them.md`](adr/0004-prove-defects-dont-assert-them.md).

## Why token-preserving XML

The repair path rewrites only what a `Fix` addresses and passes every other
byte through untouched. This is not fastidiousness. OOXML's Markup
Compatibility layer references namespace prefixes **by name, as attribute
string values** — `mc:Ignorable="p14"`. A DOM round-trip that renames prefixes
produces a file that is still well-formed XML and is rejected by PowerPoint.

Measured on a representative slide:

| Approach | Result |
|---|---|
| Rust `quick-xml` token stream | byte-identical but for the intended edit |
| Python `ElementTree` | prolog rewritten, prefixes renamed on unregistered namespaces |
| Go `encoding/xml` | `xmlns:a` → `_xmlns:a`, `mc:Ignorable` → `_:Ignorable`; invalid |

See [`adr/0002-rust-and-token-preserving-xml.md`](adr/0002-rust-and-token-preserving-xml.md).

## Principles, concretely

**SOLID** — each is load-bearing here, not recited:

- **S** — `mirsam-core` changes when Arabic correctness changes; an adapter
  changes when a file format changes. These are genuinely different clocks.
- **O** — a new check is a `Rule` impl plus one line in the registry. A new
  format is a new crate. Neither modifies existing code.
- **L** — every adapter satisfies one conformance suite, so the engine can hold
  any `DocumentReader` without special-casing.
- **I** — `DocumentReader` / `DocumentWriter` split, motivated by PDF.
- **D** — the domain defines the traits; adapters depend on the domain; the CLI
  wires them. Dependencies point inward, always.

**KISS** — what was deliberately *not* built: no plugin system, no dynamic rule
loading, no config DSL, no document object model, no async. Rules are pure
functions over one struct. The whole engine is a `Vec<Box<dyn Rule>>` and a
loop, and it should stay that way until something forces otherwise.

## Crate layout

```
crates/
  mirsam-core/     domain — Unicode, bidi, rules, ports. No I/O whatsoever.
    text.rs        TextUnit, Properties, Resolved<T>
    bidi.rs        UAX#9 resolution, dominant vs auto direction
    script.rs      Arabic script detection, presentation forms
    controls.rs    explicit bidi controls (never ZWJ/ZWNJ)
    diagnostic.rs  Severity, Diagnostic, Evidence, Report
    fix.rs         format-agnostic repairs
    ports.rs       DocumentReader / DocumentWriter
    rules/         the rule set
  mirsam-ooxml/    adapter — PPTX today; DOCX and XLSX share the package layer
    package.rs     ZIP access and the byte-preserving rewrite (raw entry copy)
    rewrite.rs     token-stream repair: change what a Fix names, nothing else
    pptx.rs        DrawingML vocabulary: paragraphs, properties, bullets;
                   DocumentReader and DocumentWriter
  mirsam-cli/      driving adapter — argument parsing and rendering only
```

`mirsam-core` has three dependencies and no I/O. That constraint is the
architecture; if it ever needs to open a file, something has gone wrong.
