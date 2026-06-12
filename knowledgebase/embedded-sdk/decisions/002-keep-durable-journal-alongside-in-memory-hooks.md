---
type: decision
status: accepted
date: 2026-06-12
features:
  - "[[hooks-and-events]]"
---

# In-memory hooks complement the durable journal — never replace it

## Context

The original impulse was "rewrite event triggers as in-memory pre/post hooks
and drop the Postgres journal" to avoid journal write amplification. But the
journal write happens in the same transaction as the mutation — it is the
only thing that makes delivery at-least-once across crashes.

## Decision

Three mechanisms with explicit, documented contracts, chosen per use case:

1. Durable event triggers (journal, at-least-once, fires on any table
   change) — for anything that must not be lost.
2. Sync hooks: `pre_insert` (validate/enrich/reject) and in-transaction
   `post_insert` (sees defaults/ids, can abort; engine wraps the operation
   in an explicit transaction only when such a hook is registered).
3. Post-commit in-memory events — documented **at-most-once**; for cache
   invalidation, metrics, notifications.

Removing the journal is rejected: "in-memory instead of journal" is a silent
guarantee downgrade that surfaces in production.

## Alternatives

| Option | Why Not |
|--------|---------|
| In-memory only, no journal | Process crash between commit and callback loses the event forever; at-most-once unfit for money-class side effects |
| Journal only, no sync hooks | Can't validate/enrich/abort mutations; async-only DX misses the main embedded value |
| Call hooks from mutation code without journal but claim durability | Lie in the contract; first production crash exposes it |

## Consequences

Users pick guarantees explicitly; the journal's write cost (~2–3x on
mutations with triggers) is paid only by tables that opt into durable
triggers. In-process fast-path delivery (push after commit, journal as
crash/nack fallback) gets low latency without weakening at-least-once.
Hooks see only GraphQL-path writes — raw SQL bypasses them (journal triggers
don't); documented per mechanism.
