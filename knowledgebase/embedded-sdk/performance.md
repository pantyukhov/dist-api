---
type: design
domain: embedded-sdk
created: 2026-06-12
---

# Performance Analysis

> cgo is ~40ns/call but the real hazard is scheduler P-holding; the durable
> journal tops out at ~5–20k events/s per PG instance; Node's ceiling is
> single-threaded handler CPU.

## Query hot path

Unchanged in every embedded variant where Rust keeps the listener — bounded
by Postgres, tens of thousands RPS. Go GC pauses affect only hooks, not the
engine (better isolation than a pure-Go engine would have).

## cgo numbers (verified, date-stamped)

- Per-call overhead: ~171ns (2015, Go 1.5) → ~60ns (2018, Go 1.10) → **~40ns
  (Go 1.21+, current)**. Roughly 100x a native Go call but tiny in absolute
  terms.
- **The real hazard is scheduler interaction, not nanoseconds**: a goroutine
  in a cgo call holds its P up to ~20µs before sysmon treats it as blocked
  (actual retake can reach 10–17ms due to sysmon's adaptive sleep).
  CockroachDB: 100 concurrent cgo callers → ~6.5x throughput collapse
  (8,200 → 1,245 ops/s); moving batching to the Go side ≈ +100% performance.
  The Go team formally declined a "nonblocking cgo" path (2016) — the cost is
  structural; **amortize via batching, don't hope for a faster FFI**.
- Consequences: batched pull API is an invariant; the number of goroutines
  blocked in FFI must be small and configurable; reverse-direction cost
  (Rust threads → Go, needm/dropm) is unmeasured anywhere — microbenchmark
  with a pre-attached thread pool before fixing the hook-pool size.

## Hook-call overhead in context

Boundary crossing + JSON for a ~1KB batch: ~1–5µs, against 0.5–2ms for the
mutation's Postgres roundtrip → **<1% of operation latency**. Inside the
handler it is plain Go — no "slowed-down Go" exists. GC allocation pressure
at 20k events/s × 1KB ≈ 20MB/s — negligible.

## Event journal ceiling (durable triggers)

Write amplification per event: journal INSERT (in mutation txn) + status
UPDATE + cleanup → mutations with triggers cost ~2–3x in writes. Practical
ceiling **~5–20k events/s per PG instance**. Mandatory mitigations:
`FOR UPDATE SKIP LOCKED`, time-partitioned journal (drop partitions instead
of DELETE — kills vacuum pain), batched acks (`UPDATE..WHERE id = ANY($1)`),
aggressive archival. Backlog-depth metric + alert: slow handlers + correct
backpressure = silently growing delivery lag. Beyond the ceiling: shard the
journal or switch capture to logical replication (CDC) — a different scale of
project.

## In-transaction hooks

Limited not by CPU but by transaction hold time: the txn and a pool
connection stay busy for the whole handler (including any `errgroup.Wait` on
network calls). Monitor handler latency; size the pool accordingly.

## Node.js verdict

Achievable "normal" performance with one structural ceiling: handler JS-CPU
time is single-threaded.

| Scenario | Node vs Go |
|---|---|
| I/O-bound hooks (typical) | parity — `await` releases the loop |
| Post-commit stream 5–20k/s, light handlers | parity (batched push) |
| CPU-bound handlers | Go wins ~Ncores×; Node mitigates with worker_threads |
| Sync-hook p99 with a busy app event loop | Go steadier; Node needs a dedicated worker |

TSFN crossing ~5–20µs; V8 JSON parse is fast; GC pauses hit hook latency
only. Document honestly: sustained CPU-heavy hook pipelines → recommend the
Go SDK.

## See Also

- [[wasm-compiler-core]] — wasm call costs (cheaper hazard profile than cgo)
- [[research-findings]] — sources for every number above
- [[_index|Domain Index]]
