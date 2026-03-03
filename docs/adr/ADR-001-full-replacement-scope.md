# ADR-001: Full Replacement Scope

## Status
Accepted

## Decision
`uc` will replace Scarb for our workflows rather than remaining a narrow compatibility acceleration layer.

## Rationale
- Maximizes architecture freedom for 2026+ performance and platform goals.
- Avoids long-term constraints from layering only around legacy orchestration patterns.
- Aligns product direction with long-term ownership and optimization goals.

## Consequences
- Larger implementation scope and delivery effort.
- Must enforce strict measurable gates to control execution risk.
