---
type: design
domain: embedded-sdk
created: 2026-06-12
---

# FFI Boundary Design (cgo variant)

> Tiny versioned C ABI; sync hooks via one batched trampoline, async events
> via blocking pull; memory ownership is unidirectional.

Two mechanisms because sync hooks and async events have different natures.
Crate layout: shared `dist-embed` crate (pure Rust embedding API) → thin
facades: `dist-ffi` (C ABI, for Go) and `dist-node` (napi-rs).

## C ABI surface — tiny and versioned

```c
dist_abi_version()
dist_start(config_json) -> handle      // engine, tokio, HTTP listener
dist_stop(handle)                      // drains in-flight hooks
dist_register_hook(handle, table, op, phase, hook_id)
dist_next_events(handle, max, timeout_ms) -> events_json | NULL   // batched!
dist_ack(handle, ids_json, ok)
dist_buf_free(buf)
```

Only byte buffers cross the boundary (JSON now; format can change without
breaking the ABI). No structs.

## Sync hooks: single exported trampoline + handle registry

The SQLite/DuckDB pattern (verified canonical, see [[research-findings]]):

- One `//export go_hook_dispatch(hook_id, payload) -> result` trampoline for
  the whole SDK; registry via `cgo.Handle` (or a grocksdb-style indexed
  COW-list if sync.Map contention shows up — benchmark first).
- **Batch rows per call** — one crossing per operation, not per row. An
  invariant, not an optimization (see [[performance]] on scheduler hazards).
- Rust side dispatches hook calls via `spawn_blocking`/dedicated pool — never
  on tokio workers.
- Result JSON: `{rows: [...]} | {error: {...}} | {ok: true}`.

## Async post-commit events: blocking pull, not callbacks

Kafka-consumer shape: N bounded goroutines call `dist_next_events` (batched),
dispatch to handlers/channels, ack. Gives natural backpressure, keeps error
handling in the Go stack, never blocks tokio workers. Channels cannot cross
FFI — they are the Go-side ergonomic wrapper over this pull loop.

## Three rules that make it stable

1. **Unidirectional memory ownership.** Rust allocates → only
   `dist_buf_free` frees; Go copies the buffer into its own structs
   immediately. Neither side ever holds the other's pointers past one call.
   (cgo pointer rules verified: Go pointers containing Go pointers panic;
   strings/slices/channels are unpinnable.)
2. **Panic barrier in both directions.** `catch_unwind` on every
   `extern "C"`; `recover` in the trampoline. Panics become `{error}`
   results, never UB / process crashes.
3. **User code never sees FFI.** Public API is `PreInsert(...)` and
   `for ev := range eng.Events()`; the webhook transport implements the same
   interface.

## Node.js SDK (napi-rs)

- Bind `dist-embed` directly via napi-rs (not raw FFI over the C ABI);
  prebuilt platform packages npm-style (esbuild/swc/Prisma distribution
  model).
- JS is single-threaded: sync hooks go through N-API `ThreadsafeFunction`
  (call queued onto the event loop; hook-pool thread awaits the result,
  Promise-returning handlers supported).
- **Pull doesn't work in Node** — nothing to block. Post-commit events need
  push mode (TSFN with bounded in-flight for backpressure). Hence the
  DeliveryTransport must support pull AND push from day one.
- Mitigations for the single-thread ceiling: event-loop-lag metric as
  first-class SDK telemetry; option to run hooks in a dedicated
  `worker_thread`.

## See Also

- [[hooks-and-events]] — the semantics this boundary carries
- [[performance]] — why batching is an invariant
- [[_index|Domain Index]]
