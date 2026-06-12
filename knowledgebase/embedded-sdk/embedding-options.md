---
type: design
domain: embedded-sdk
created: 2026-06-12
---

# Embedding the Rust Engine into a Go Process

> Why "Go's http server in front, engine behind cgo" hurts, and which shapes
> of embedding survive scrutiny.

## Naive option: Go's net/http in front, engine behind cgo per request

Downsides, in order of pain:

1. **Two schedulers.** Tokio lives inside the Rust core (pool, async
   execution, subscriptions); Go has its own scheduler. They compete for
   cores, and every cgo call blocks an OS thread: the goroutine pins a thread
   for the whole duration of the Rust call. Under load Go spawns more threads
   (its reaction to cgo-blocked threads) and the cheap-goroutine advantage
   evaporates.
2. **Postgres pool ownership.** If the pool stays in Rust, the full tokio
   runtime comes with it. If Go owns I/O (pgx), Rust degrades to a SQL
   compiler and the execution layer must be rewritten in Go — a different
   project (later this became [[wasm-compiler-core]]).
3. **Subscriptions/websockets.** Long-lived streaming state in Rust with
   event delivery back into Go across FFI is the worst part — no natural way
   to stream across cgo, only callbacks with manual lifecycle management.
4. **Build & ops.** `CGO_ENABLED=1` breaks easy cross-compilation; static
   linking of a Rust staticlib + musl is its own adventure; CI gets heavy.
   Profiling splits (pprof can't see Rust frames, perf can't see Go well).
   A Rust panic crossing FFI without `catch_unwind` is UB.
5. **Memory.** Everything crosses by copy with manual freeing on the Go side;
   ownership mistakes are use-after-free, not panics.

## Better cgo shape (if embedding the full engine)

Let the Rust engine keep its own HTTP listener (axum inside the cdylib, on
its own tokio). Then **only events/hooks cross the cgo boundary** — the hot
query path never touches Go, and downsides 1–2 mostly disappear. An optional
`dist_execute(request) -> response` can serve a single-port setup but brings
back thread-per-request blocking; keep it an option, not the default.

## Sidecar alternative (always keep it)

Rust server as-is + unix socket/gRPC from Go gives runtime isolation, normal
stack traces, independent deploys; cost is one local hop (~tens of µs,
invisible next to a Postgres roundtrip). The Go SDK must expose the same
handler API over both transports so users can switch without code changes
(see [[decisions/001-webhooks-first-transport-agnostic-sdk]]).
Embedded-via-cgo is only justified when a single binary is a hard
requirement.

## See Also

- [[ffi-boundary]] — how the boundary is built if we do embed
- [[precedents]] — why the Go ecosystem resists cgo
- [[_index|Domain Index]]
