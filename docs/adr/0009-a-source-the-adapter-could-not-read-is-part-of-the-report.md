# 9. A source the adapter could not read is part of the report

Date: 2026-09-05

## Status

Accepted.

## Context

`docs/PLAN.md` says of M5 and M6: *"Adapters only; no core changes expected.
If one is needed, record an ADR explaining what the core got wrong."* The HTML
adapter needed one. This is that record.

A `.pptx` and a `.docx` are packages. Everything that decides a paragraph's
direction — the layout, the master, `styles.xml`, the theme — is inside the
file the tool opened, so when the adapter has finished reading, it has read
everything there was.

HTML is not like that. A page states direction in three places, and only two
of them are in the file:

```html
<p dir="rtl">…</p>                                   <!-- in the file -->
<style>body { direction: rtl }</style>               <!-- in the file -->
<link rel="stylesheet" href="https://cdn/site.css">  <!-- somewhere else -->
```

mirsam performs no network I/O, by design: an audit whose answer depended on
a server would not be reproducible, and a tool that quietly fetched URLs out
of documents would be a tool nobody could run on an untrusted file. So the
third case is a stylesheet the adapter does not read, and the rules in it are
rules nobody applied.

That matters because of what it does to a finding. A page whose direction is
set only by that sheet comes back with `direction` `Unset`, and the engine —
correctly, given what it was handed — reports `direction-unset` on every
Arabic paragraph in it. The finding is not wrong about the units it was given.
It is wrong about the document, and the report as it stood had no way to say
so.

Standing rule 4 already covers exactly this shape of problem: *"Report only
what was verified. `NOT RUN` is an honest result; inferred compatibility is
not."* `font-coverage` and `shaping-broken` obey it through `fonts:
{checked: false}`, which is why nobody may read their silence as a pass. The
same sentence needed saying one level up — not about a *check* that did not
run, but about a *source* that was not read.

## Decision

`DocumentReader` gains one method, with a default:

```rust
fn unread_sources(&self) -> Vec<String> { Vec::new() }
```

It names, as the document names them, the sources the last `scan` could not
read. Both OOXML adapters take the default and answer with nothing, which for
a package is the whole truth. The HTML adapter answers with the `href` of
every stylesheet it did not fetch.

`audit --format json` carries the answer in every report, in the shape it
carries `fonts`:

```json
"sources": { "unread": ["https://cdn.example.test/site.css"] }
```

and the human report prints a line only when the list is non-empty.

## Consequences

**The core learned a word, and it is not a format's word.** "A source the
adapter could not read" is as true of a Word document with a linked template
or a spreadsheet with an external workbook reference as it is of a web page.
Had the method been called `unread_stylesheets`, or had it returned URLs, the
core would have learned CSS — which is the failure standing rule 5 forbids.
It returns opaque strings the engine never parses, exactly as `UnitId` does.

**A default of "nothing" is a claim, and it is a true one.** An adapter that
reads a self-contained file has read everything, and saying so costs it no
code. An adapter that grows an external reference later has to override the
method, and a reviewer can see at a glance which adapters make the claim.

**It is a port change, so it is the abstraction moving rather than a format
leaking into it.** The alternative was for the CLI to know that HTML is the
format with unread sources and to downcast to `HtmlDocument` to ask — which
would put a format's name in the driving adapter, and would have left a
library caller with no way to ask the same question.

**The report shape changed for every format**, and the golden corpus records
the change on real documents. A `"sources"` block reading `{"unread": []}` on
a deck is the point: a consumer branches on one shape, and never has to know
which formats can have unread sources.

## Related

- Standing rule 4, `AGENTS.md` — "Report only what was verified."
- [ADR 0003](0003-hexagonal-ports-and-adapters.md) — the ports this extends.
- [ADR 0007](0007-an-inherited-default-is-not-a-choice.md) — the other place
  the tool refuses to let an absence stand for a decision.
