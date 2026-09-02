# 4. Prove defects; do not assert them

**Status:** accepted · 2026-09-02

## Context

The established approach to Arabic document linting is to check attributes: is
`rtl="1"` present, is `algn` set, is there a language tag. The prior art does
this, well and honestly.

It has two failure modes, both observed in practice:

1. **False positives.** A paragraph inheriting alignment from its layout is
   reported as defective. The repair then *forces* an alignment, silently
   changing the author's design. Users learn to ignore the tool, which is worse
   than not having it.
2. **False negatives.** Text can be mis-tagged and still render correctly,
   which is reported as a failure; and correctly tagged text can still be
   wrong, which is not reported at all.

The underlying issue: an attribute is a proxy for the thing that matters, which
is *what the reader will see*.

## Decision

Resolve the Unicode bidirectional algorithm over the actual string, under the
declared direction and under the semantically correct direction, and report a
defect only when the two visual orders differ.

Severity follows from what was proven:

| Finding | Severity |
|---|---|
| Resolved order is wrong | error |
| Order is right, but only via renderer auto-detection | warning |
| Metadata absent; rendering unaffected | warning |
| Stylistic or unverifiable | note |

Every diagnostic carries `Evidence`: the logical text and both resolved orders.

Property state is modelled as `Resolved<T>` — `Explicit` / `Inherited` /
`Unset` — so a rule can distinguish "absent" from "inherited and correct". A
rule firing on `Inherited` is a bug.

## Consequences

- Fewer findings, each defensible. On the reference fixture: one error, where
  the attribute-based approach reported eight.
- Findings are checkable without opening PowerPoint, which is what makes the
  tool useful to an agent rather than merely to a human with Office installed.
- **Cost:** rules are more expensive to write — each must construct the
  counterfactual rendering, not read an attribute.
- **Cost:** the tool will stay silent on text that renders correctly today by
  accident. `direction-unset` covers the fragility as a warning; that is the
  honest severity.
