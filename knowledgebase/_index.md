# dist-api Knowledge Base

> Living documentation for design explorations and decisions that are not yet
> (or not only) code. Engine internals and conformance status live in PLAN.md
> and tests/hasura/COVERAGE.md; this base holds ideas, research, and ADRs.

## Domains

### [[embedded-sdk/_index|Embedded SDK & Native Hooks]]
Embedding the engine into host-language applications (Go, Node.js) with
native-function hooks (`pre_insert` / `post_insert` / post-commit) instead of
Hasura-style webhooks. 6 design notes, 1 research report, 5 decisions.
**Status: idea, deferred until core conformance is done.**

## Templates

- [[_templates/feature-dossier|Feature Dossier Template]]
- [[_templates/decision|Decision (ADR) Template]]
