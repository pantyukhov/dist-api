---
type: decision
status: accepted
date: 2026-06-12
features:
  - "[[precedents]]"
---

# Never embed a Go runtime inside a Node process

## Context

If the engine core were written in Go, the Node.js SDK question becomes "can
Go run inside Node?". Go compiles to `c-shared`, but carries its full
runtime: GC, scheduler, signal handlers, own threads — inside a foreign
process this means signal conflicts, two GCs, non-unloadability.

## Decision

Written down so it is never attempted: a Go core serves Node **only via a
subprocess/sidecar** (the esbuild model — its author deliberately ships a
child-process binary speaking stdio instead of embedding; esbuild's wasm
build is ~10x slower). In-process Node support requires a core with no
runtime — Rust (napi-rs) or wasm.

## Alternatives

| Option | Why Not |
|--------|---------|
| Go c-shared loaded into Node | No production precedent at scale; signal/thread/GC conflicts; the entire native-Node-module ecosystem (swc, Prisma, Parcel) is C/C++/Rust via N-API, Go absent as a class |
| TinyGo → wasm | Severe stdlib/GC limitations; would constrain the engine core |

## Consequences

The core-language choice directly decides Node's ceiling: Go core → Node is
sidecar forever; Rust/wasm core → in-process Node possible. Feeds
[[005-wasm-compiler-core-over-cgo-or-go-rewrite]].
