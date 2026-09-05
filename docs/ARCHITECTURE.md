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
    Explicit(T),           // stated on this unit
    Inherited(T, Origin),  // supplied by the named layout, master or cascade
    Unset,                 // nothing anywhere; the renderer picks
}
```

This three-state model is the single most important decision in the codebase.
Attribute-only linters collapse `Inherited` into `Unset` and therefore report
every placeholder that inherits centred alignment from its layout as a defect.
That false positive is what drives users to disable the tool.

Adapters are responsible for resolving the inheritance chain. Rules are
responsible for distinguishing the three states.

Resolving a value is not the same as establishing that anyone chose it, which
is the distinction [ADR 0007](adr/0007-an-inherited-default-is-not-a-choice.md)
draws: **an inherited value is evidence of a choice only where it agrees with
the text.** Arabic under an `rtl="1"` master is the layout doing its job and is
never reported — the case the three-state model exists to protect. Arabic under
an English template's untouched `rtl="0"` is the absence of a decision wearing
the same clothes as one, and is reported exactly as an absent value is. The
condition is narrow and mechanical: does the inherited value match
`bidi::dominant_direction`, the same comparison the rules already make on
`Explicit`.

`Origin` is what makes such a finding arguable. It names the part and property
that supplied the value — `ppt/slideMasters/slideMaster1.xml
bodyStyle/lvl1pPr@rtl` — and surfaces as `evidence.inherited_from`, so a
reviewer can check "the master says left-to-right" without opening PowerPoint.
The repair still writes to the unit the finding names; editing the master would
change every paragraph in the deck.

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
- **L** — every adapter satisfies one conformance suite
  (`crates/mirsam-ooxml/tests/conformance.rs`), so the engine can hold any
  `DocumentReader` without special-casing. Each case states a situation once in
  the shared model's vocabulary and runs it against every adapter; no case that
  asserts what the tool reports names an element, an attribute or a format.
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
    text.rs        TextUnit (paragraph or container), Properties, Resolved<T>
    bidi.rs        UAX#9 resolution, dominant vs auto direction
    script.rs      Arabic script detection, presentation forms
    controls.rs    explicit bidi controls (never ZWJ/ZWNJ)
    joining.rs     Joining_Type and the contextual form each letter is
                   required to take — the expectation, from the text alone
    shape.rs       what a font actually does with it: rustybuzz over caller-
                   supplied bytes, reported letter by letter and never judged
    diagnostic.rs  Severity, Diagnostic, Evidence, Report
    fix.rs         format-agnostic repairs
    ports.rs       DocumentReader / DocumentWriter
    rules/         the rule set
  mirsam-ooxml/    adapter — PPTX and DOCX; XLSX will share the two modules
                   below that name no element: the package and the scaffold
    package.rs     ZIP access and the byte-preserving rewrite (raw entry copy)
    token.rs       the same guarantee inside a part: read to events, splice an
                   attribute in its raw bytes, insert a child at the rank a
                   caller's schema sequence decides, rewrite run text. Which
                   element is which is the caller's to say
    rels.rs        the OPC relationship graph: which part a part inherits
                   from — slide → layout → master → theme, with each part's
                   role read from the relationships pointing at it
    inherit.rs     the properties along that chain: placeholder list styles
                   and a master's named text styles, resolved into
                   Resolved::Inherited with the part that supplied each value
    rewrite.rs     DrawingML's repair vocabulary: which element and which
                   attribute each Fix lands on, and where a created element
                   goes. Every edit is a token.rs call with a name in it
    pptx.rs        DrawingML vocabulary: paragraphs, properties, bullets;
                   DocumentReader and DocumentWriter
    chart.rs       chart text containers: the cached strings an axis, a
                   legend or a set of data labels draws, which are not
                   paragraphs and which no DrawingML pass can see
    docx.rs        WordprocessingML vocabulary: paragraphs and the properties
                   the rules judge. DocumentReader only — Word's writer is
                   not built, and the split ports are what let it wait
    style.rs       Word's own chain, which is not a walk between parts:
                   docDefaults, the styles a paragraph and its runs name, and
                   the w:basedOn above each — all in one word/styles.xml.
                   Resolved the same way inherit.rs resolves PowerPoint's,
                   and sharing its theme reader, because a fontScheme is
                   DrawingML wherever it is stored
  mirsam-cli/      driving adapter — argument parsing and rendering only
    tests/golden.rs  the golden corpus: every .pptx and .docx under
                   tests/fixtures/ against its committed report of what the
                   binary finds, repairs and writes — or, for a format it
                   reads but cannot write, of the refusal it gives instead
```

`mirsam-core` has five dependencies and no I/O. That constraint is the
architecture; if it ever needs to open a file, something has gone wrong.

The fifth is `rustybuzz`, and it is worth saying why it does not breach that.
Shaping is the domain: "does this Arabic join" is the question the whole tool
exists to answer, and it is answered by an algorithm over text and a font, not
by a renderer and not by a filesystem. `shape::Font::parse` takes bytes.
*Which* typeface a paragraph resolves to, and where that file lives on which
machine, are questions about the world, and they stay in an adapter.
