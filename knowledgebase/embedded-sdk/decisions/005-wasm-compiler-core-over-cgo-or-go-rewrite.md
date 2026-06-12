---
type: decision
status: proposed
date: 2026-06-12
features:
  - "[[wasm-compiler-core]]"
  - "[[precedents]]"
---

# Core language: wasm compiler-core over full-engine-cgo or a Go rewrite

## Context

The embedded idea forces a core-language bet. Rewrite cost is low (the
engine reproduces in days with AI assistance; the real asset — tests-py
harness + spec knowledge — is HTTP-level and transfers to any language), so
this is a pure architecture decision, not a sunk-cost one. The tension:
a Rust core behind cgo taxes the flagship Go SDK's audience (the Go
ecosystem demonstrably avoids cgo — see [[precedents]]); a Go core kills
in-process Node forever ([[003-never-embed-go-runtime-in-node]]).

## Decision (proposed, revisit at implementation time)

Split the engine: the conformance-heavy core (GraphQL parsing, metadata,
permissions, sqlgen) compiles to **one wasm blob**; each language gets a
thin **native execution layer** (HTTP, pg pool, transactions, hooks as plain
native functions). Feasible specifically because of the M4 design — one SQL
statement per operation, JSON assembled in Postgres — which makes the host
layer "run statement, return JSON blob". Precedent: ncruces/go-sqlite3
(SQLite-in-wasm under wazero, pure-Go module, ~1.2–2x slower than cgo,
accepted by the market).

## Alternatives

| Option | Why Not |
|--------|---------|
| Full Rust engine embedded via cgo | cgo adoption tax on exactly the target audience; foreign threads, panic barriers, prebuilt matrix; Prisma's public retreat is the scar |
| Rewrite the engine in Go | Perfect Go SDK, but Node = sidecar forever, every future language too; loses Rust hot-path headroom; abandons multi-language embedded entirely |
| Sidecar only (no embedding) | Loses the differentiator (native hooks, single binary); remains as the universal fallback regardless |
| Embedded JS/wasm plugins inside the Rust server (extism/goja model) | Inverts the ownership: users want the engine in *their* app with *their* language's ecosystem, not scripts inside our server |

## Consequences

Gains: every language gets native hooks with zero FFI hazards; pure-language
modules (`go get` / `npm install` work); Rust core work reused everywhere;
hot path = host language + Postgres with a plan cache (wasm only on cache
miss, ~0.1–1ms per compile under wazero).

Costs: execution layer (txns, error mapping, subscriptions polling) written
per language; conformance must run against each SDK (harness is
implementation-agnostic, which makes this tractable); plan format becomes a
versioned contract; wasm instances are single-threaded (instance pool +
parameterized plans required).

Status **proposed**: confirm at implementation time with (a) a spike of the
plan-format contract, (b) a wazero compile-latency benchmark on real
metadata, (c) the per-SDK conformance run wired into CI.
