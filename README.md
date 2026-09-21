# Phodopus

[![CI](https://github.com/bitty-terminal/phodopus/actions/workflows/ci.yml/badge.svg)](https://github.com/bitty-terminal/phodopus/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: CC0-1.0](https://img.shields.io/badge/License-CC0_1.0-lightgrey.svg)](LICENSE-CC0)

**Phodopus** is a pure-Rust, stackless Lua runtime designed for uncompromising sandboxing, deterministic execution, and predictable resource bounds.

Originally forked from Catherine West's ([@kyren](https://github.com/kyren)) pioneering work on [Piccolo](https://github.com/kyren/piccolo), Phodopus preserves all original commit history and licensing while advancing the runtime toward a modular embedded engine. Development is pre-adoption: Phases 1, 1.5, and 2 of the roadmap are complete; the Phase 3 hard heap quota and the Phase 4 typed host-async suspension bridge are implemented in the runtime core (see [Target Architecture](#target-architecture-status-by-phase)), while the remaining roadmap scope — broader Fuel-policy work, the host-side scheduler adapter, and the Phase 5 Bitty host ABI — remains open (see [Roadmap & Evolution](#roadmap--evolution)). It is not yet a production sandbox and is not yet Bitty's active runtime. Project SemVer begins afresh at `0.1.0-alpha.1` towards `0.1.0`, anchored on the upstream Piccolo `0.3.3` lineage baseline.

---

## Why "Phodopus"?

_Phodopus_ is the biological genus of small, energetic dwarf hamsters. It harmonizes with Bitty's hamster-themed ecosystem (**Bitty** terminal -> **Bittie** mascot -> **Wheel** agent harness -> **Phodopus** runtime engine) while remaining a fully independent, general-purpose Rust crate with no external host coupling.

---

## Current Capabilities (Today)

1. **Pure Rust Implementation**: Zero C dependencies and no `longjmp`; `gc-arena` provides generative lifetime-branded GC pointer safety. `unsafe` is confined to specific VM and `gc-arena` primitives, not eliminated; see the [Unsafe Code Ledger](docs/security/unsafe-ledger.md) for the audited surface.
2. **Stackless & Preemptible VM**: Execution state is heap-allocated in the GC arena. Coroutines, callbacks, and tail calls trampoline through non-blocking `Sequence` steps without consuming native Rust stack frames.
3. **Deterministic Fuel Metering**: Fine-grained instruction budgeting ("Fuel") allows pausing or terminating runaway execution loops.
4. **Microsecond Cold Starts & Tiny Footprint**: Starts in ~35 µs with an initial heap footprint of only ~11 KB.
5. **Lua 5.4 Language & Stdlib Subsets**: Core Lua 5.4 syntax, arithmetic/bitwise operators, closures, coroutines, and metatables; stdlib modules `base`, `coroutine`, `math`, `string` (`format`, Lua patterns, `pack`/`unpack`), `table`, `utf8`, and `debug.traceback`; sandboxed text-only `load` with `_G`; and a sandboxed `require` with a pluggable searcher chain, preloaded core modules, an empty default `package.path`, and capability-constrained VFS roots. The `io` and `os` modules are intentionally absent from the sandboxed core; `print` is the only I/O-adjacent global and dynamic loading is opt-in.

---

## Target Architecture (Status by Phase)

1. **Hard Memory Quotas** (Phase 3, shipped in core): `RuntimeBuilder::memory_limit` installs a hard heap quota, enforced by per-site pre-allocation checks plus a single executor-loop chokepoint, with the peak bounded per the [sandbox specification](docs/specifications/sandbox-and-fuel.md) (measured ≤ 2× quota at 32 KiB and above; quotas below the ~25 KiB runtime baseline refuse at baseline). Broader Fuel-policy work remains open.
2. **Host-Agnostic Async Suspension** (Phase 4, shipped in core): typed host suspension descriptors (`HostOpHandle`, surfaced through `SequencePoll::Suspend`) decoupled from specific async runtimes (Tokio/smol/async-std); the bridge lives in `crates/phodopus/src/hostop.rs` per the [async trampoline specification](docs/specifications/async-trampoline.md) (status: implemented). The prototype Piccolo no-op waker stub remains only for `AsyncSequence::pending` / in-VM yields; host-driven suspension goes through the typed bridge. The host-side scheduler adapter remains open.
3. **Bitty Host ABI** (Phase 5, not yet implemented): The `bitty-lua` consumer layer mounting terminal, panel, command, and filesystem surfaces on the generic runtime.

---

## Workspace Structure

The repository is structured as a standard multi-crate virtual workspace under `crates/`:

- [`crates/phodopus`](crates/phodopus): Core runtime crate containing the virtual machine, compiler, fuel accounting, and standard library.
- [`crates/phodopus-util`](crates/phodopus-util): Ergonomic integration helpers (`freeze`, Serde support, userdata binders).
- [`docs/`](docs): Canonical documentation corpus (architecture, specifications, and development guides).

---

## Roadmap & Evolution

- [x] **Phase 0: Baseline & Lineage Preservation**: Fork Piccolo with complete Git history, MIT/CC0 attribution, upstream remote tracking, and verified green quality gates.
- [x] **Phase 1: Upstream PR Absorption**: Review and integrate mature community contributions:
  - PR #128: `string.format` implementation
  - PR #129: Authentic Lua pattern matching (`find`, `match`, `gsub`)
  - PR #110: `utf8` standard library
  - PR #121: Backtraces and error location reporting
- [x] **Phase 1.5: Sandboxed Dynamic Loading**: Text-only `load`, custom `_ENV` binding, piecewise iterator chunks with a 16 MiB assembly ceiling, and global `_G`.
- [x] **Phase 2: Sandboxed Module System**: Pluggable searcher chain for `require`, embedded preloaded modules, and capability-constrained VFS resolvers.
- [ ] **Phase 3: Hard Quotas & Resource Accounting**: Explicit maximum heap memory limits and Fuel allocation policies directly on the `RuntimeBuilder`.
- [ ] **Phase 4: Generic Async Bridge**: Clean suspension protocol for host-driven futures and coroutine wakeups without core runtime coupling.
- [ ] **Phase 5: Bitty Host ABI**: Mount high-level terminal and plugin interfaces (`bitty-lua`) strictly on top as an unprivileged consumer.

---

## Garbage Collection: `gc-arena`

The garbage collection model is powered by [`gc-arena`](https://github.com/kyren/gc-arena). Phodopus features an incremental, cycle-detecting garbage collector with zero-cost `Gc` pointers that are machine-pointer sized and implement `Copy`.

It achieves safety by combining:

1. An unsafe `Collect` trait for tracing garbage-collected types, safely implemented via derive macros.
2. Branding `Gc` pointers with unique, invariant "generative" lifetimes, ensuring pointers remain isolated to a single root arena.

---

## Stackless VM Architecture

In Phodopus, execution is organized in a "stackless" (trampoline) style. Lua callbacks can either produce an immediate result (value, coroutine yield, error) or return a `Sequence`. A `Sequence` behaves like a multi-step state machine that the parent `Executor` drives across mutation cycles:

```text
[Host / Rust] -> [Lua Coroutine] -> [Rust Sequence] -> [Yielding Lua Code]
```

Control continuously returns outside the GC arena to the driving host loop. The host can pause, cancel, or switch tasks at any boundary without unwinding native stack frames.

---

## Development & Quality Gates

Phodopus adheres to Bitty's strict engineering quality gates:

```bash
# Run all quality checks (formatting, compilation, clippy, tests)
just check

# Format source code
just fmt

# Run unit and integration tests
just test
```

---

## Lineage & Attribution

Phodopus is built on the foundations of **Piccolo** (previously known as _Luster_ and _Deimos_), conceived and authored by **Catherine West** ([@kyren](https://github.com/kyren)) and community contributors.

We honor the immense craftsmanship that went into Piccolo's stackless architecture and `gc-arena`. All original copyright notices, licenses, and commit histories remain in place.

---

## License

Phodopus is dual-licensed under:

- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
- Creative Commons CC0 1.0 Universal Public Domain Dedication ([LICENSE-CC0](LICENSE-CC0) or <https://creativecommons.org/publicdomain/zero/1.0/>)

at your option.
