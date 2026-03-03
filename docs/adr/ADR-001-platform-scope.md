# ADR-001: Platform Scope

## Status
Accepted

## Decision
`uc` is positioned as a next-generation Cairo compiler/build platform rather than a narrow acceleration add-on.

## Rationale
- Maximizes architecture freedom for 2026+ performance and platform goals.
- Avoids long-term constraints from layering only around legacy orchestration patterns.
- Aligns product direction with long-term ownership and optimization goals.

## Consequences
- Larger implementation scope and delivery effort.
- Must enforce strict measurable gates to control execution risk.
