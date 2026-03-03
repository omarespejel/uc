# ADR-002: First Proof Gate and Criteria

## Status
Accepted

## Decision
The first hard gate for `uc` is build-path proof:
- warm rebuild p95 >= 40% faster than Scarb baseline,
- zero artifact hash mismatches,
- diagnostics parity >= 99.5%.

## Rationale
- Fastest way to validate value proposition and de-risk further investment.

## Consequences
- Comparator and benchmark harness are mandatory early work.
- Broad command expansion is contingent on proof success.
