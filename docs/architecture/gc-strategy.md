---
title: Garbage Collector Dependency Strategy
description: Architectural strategy for gc-arena pinning, memory accounting evolution, and hard sandbox quota design
category: architecture
audience: developers
document_type: architecture
status: accepted
website_publish: true
sidebar_order: 13
---

# Garbage Collector Dependency Strategy

> Status: **accepted**. This document defines the lifecycle, dependency pinning policy, memory accounting architecture, and upstream tracking strategy for `gc-arena` within Phodopus.

---

## 1. System Vision & Context

Phodopus relies fundamentally on [`gc-arena`](https://github.com/kyren/gc-arena) for garbage collection and memory safety. Unlike conventional C Lua (which relies on manual garbage collector sweeps, marks, and raw pointers) or naive Rust Lua implementations (which lean on reference counting `Rc`/`Arc` cycles and incur reference count thrashing or leaks), `gc-arena` provides:

1. **Zero-cost, copyable `Gc<'gc, T>` pointers**: Sized identically to raw machine pointers (`usize`), implementing `Copy` without runtime reference-count manipulation.
2. **Generative Lifetime Invariants**: Unique invariant lifetimes (`'gc`) branded via closures prevent GC pointers from leaking beyond their owning arena boundary or cross-contaminating distinct Lua instances.
3. **Deterministic incremental collection**: Cycle-detecting mark-and-sweep phases driven cooperatively alongside VM execution.

Because garbage collection is deeply woven into value representations (`Value<'gc>`), closures (`Closure<'gc>`), tables (`Table<'gc>`), and thread stacks (`Thread<'gc>`), choices regarding `gc-arena` directly dictate Phodopus's memory bounds, allocator compatibility, and sandboxing guarantees.

---

## 2. Current Baseline Pinning (gc-arena 0.5.3)

Phodopus currently pins `gc-arena` to commit `5a7534b883b703f23cfb8c3cfdf033460aa77ea9`:

```toml
gc-arena = {
    git = "https://github.com/kyren/gc-arena",
    rev = "5a7534b883b703f23cfb8c3cfdf033460aa77ea9",
    features = ["allocator-api2", "hashbrown"]
}
```

This revision corresponds to upstream release **0.5.3**. The critical capabilities of this generation are:

- **`MetricsAlloc<'gc, A>`**: An allocator adapter implementing `allocator_api2::alloc::Allocator`. It transparently reports allocation, reallocation, and deallocation byte counts directly to the arena's internal `Metrics` tracker.
- **External Allocation Accounting**: Types holding memory outside the arena (such as `allocator_api2::vec::Vec<T, MetricsAlloc<'gc>>` in thread stacks and table buckets) accurately register their heap footprint into `arena.metrics().total_allocation()`.
- **Heap Observability**: Exposed via `Lua::total_memory()` and `Lua::gc_metrics()`, enabling fine-grained memory tracking per VM instance.

---

## 3. Upstream Evolution & Divergence in gc-arena 0.7.0

Upstream `gc-arena` has developed toward version **0.7.0**, introducing sweeping design changes:

1. **Removal of `MetricsAlloc` and `allocator-api2`**:
   Upstream 0.7 completely eliminated external allocator hooks and the `allocator-api2` feature to streamline the core collector and shed unstable Rust allocator API shims.
2. **Internal-Only Allocation Tracking**:
   In 0.7, the GC metrics track only heap objects allocated directly inside the GC arena itself (`GcBox`). Memory allocated by auxiliary collections (`Vec`, string buffers, hash tables) is either unaccounted for or requires manual byte instrumentation.
3. **Alternative Pacing Model**:
   0.7 introduced a revised collection pacing state machine tuned primarily for low-latency throughput rather than hard quota enforcement.

### The Architectural Tension

Phodopus is explicitly designed as a **sandbox-first embedded runtime**. A core promise of Phodopus is **Hard Memory Quotas**: host applications must be able to specify a rigid limit (e.g. `16 MiB`), and any script attempt to allocate beyond that limit must trigger immediate GC, fail safely, or interrupt execution before causing host process out-of-memory (OOM).

If `MetricsAlloc` is removed without a dedicated replacement:

- Thread stacks, open upvalues, and table storage become invisible to the memory budget.
- Malicious or bugged scripts can exhaust system RAM by growing large tables or strings without tripping the VM's internal GC allocation threshold.

Therefore, **Phodopus must not blindly upgrade to `gc-arena 0.7.0` via automated dependency bumps**.

---

## 4. Phodopus Memory Accounting & Evolution Roadmap

To reconcile sandbox guarantees with upstream progress, Phodopus adopts a four-stage strategy:

```text
┌──────────────────────────────────────────────────────────┐
│ Stage 1: Lock gc-arena 0.5.3 (Current)                   │
│ - Pin allocator-api2 = "0.2"                             │
│ - Configure Dependabot to ignore semver-major bumps      │
│ - Retain MetricsAlloc for full thread & table accounting │
└────────────────────────────┬─────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────┐
│ Stage 2: MemoryLimit quota enforcement (Landed)          │
│ - Shared MemoryLimit keyed to gc-arena Metrics           │
│ - Executor-loop chokepoint (<=1 VM iteration overshoot)  │
│ - Fallible table/string growth + GC-on-exceed            │
│ - Internal Gc-box allocation not checked individually    │
└────────────────────────────┬─────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────┐
│ Stage 3: Research gc-arena 0.7 Pacing & Memory Model     │
│ - Prototype external byte accounting under 0.7           │
│ - Evaluate whether custom Allocator wrapping is viable   │
│ - Measure GC pause latency vs strict quota overhead      │
└────────────────────────────┬─────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────┐
│ Stage 4: Maintain Compatibility Fork or Upstream PR      │
│ - If 0.7 cannot support external quotas: maintain scoped │
│   bitty-terminal/gc-arena fork with quota hooks          │
│ - Ensure dual crates.io publishing compliance            │
└──────────────────────────────────────────────────────────┘
```

### Stage 1: Active Lock on 0.5.3 (Immediate)

- Keep `gc-arena = 0.5.3` (rev `5a7534b883b703f23cfb8c3cfdf033460aa77ea9`).
- Maintain `allocator-api2 = "0.2"` in `[workspace.dependencies]`.
- Enforce the Dependabot rule in `.github/dependabot.yml` ignoring `allocator-api2` and `gc-arena` semver-major updates.

### Stage 2: Memory Accounting Abstraction (Phase 3, landed)

Phodopus implements a shared `MemoryLimit { max_bytes, current_bytes, exceeded, quota_yielded }` (see
`crates/phodopus/src/memory.rs`) rather than the illustrative `MemoryBudget` trait sketched in an
earlier draft. `MemoryLimit` is integrated with `gc-arena`'s `Metrics`: the current byte count is
the arena's tracked `Metrics::total_allocation()`, and the ceiling is shared between the `Lua`
handle, the arena root, and every allocation-boundary check.

Quota enforcement has two complementary layers:

1. **A single executor-loop chokepoint.** The executor step loop checks the tracked total against
   the ceiling after _every_ executor iteration (`crates/phodopus/src/thread/executor.rs`). This is
   the structural fix for the fact that `gc-arena 0.5.3` does not route internal `Gc`-box
   allocation through an application allocator: it covers every retained-growth path that has no
   per-site check, most importantly a deep Lua call chain. On the first excess it yields to the
   host boundary for a possible collection; if the excess survives to the next observation it
   injects a typed `OutOfMemory` as a normal `Frame::Error`, which `pcall` can catch. The maximum
   overshoot is therefore one executor iteration (`VM_GRANULARITY = 64` instructions); the observed
   worst case is ≈1.4× quota at a 1 MiB ceiling (one geometric vector reallocation), versus the
   pre-chokepoint ≈88×.
2. **Per-allocation pre-checks** for precise early refusal, each _before_ the allocation:

   - every Lua table constructor (`{...}`) charges its initial array/map capacity through
     `Table::try_new` before either part is allocated;
   - every subsequent table array/map growth calls `Context::check_memory` with the amortized growth
     request _before_ reserving, and switches from the infallible `Vec`/hashbrown growth to the
     fallible `try_reserve` path so a refused request returns a typed `OutOfMemory` instead of
     aborting;
   - `..` and `table.concat` charge the projected result size before allocating the result buffer;
   - every `Closure` opcode checks the `Gc`-boxed closure and its upvalue vector through
     `Closure::try_from_parts` before the box is allocated, which refuses a retained closure chain;
   - large standard-library string buffers (for example `string.rep`, `string.format`, and
     `string.gsub`) are pre-checked before allocation.

The split around collection is deliberate. `gc-arena` forbids collection while the arena is mutably
borrowed, so the executor chokepoint can only _refuse_, never reclaim. Collection runs at the host
boundary between executor steps, once the arena borrow is released; growth a collection can reclaim
(unrooted closure-per-iteration allocation) is therefore collected and execution continues, while
only retained growth that survives a collection attempt is refused. When the chokepoint refuses,
the unwinding path releases the spare capacity of the unwound thread buffers so the recovered
instance is genuinely usable under the same quota.

What remains unchecked _individually_ is allocation _inside_ an already charged operation (upvalue
`Gc` boxes, interned-string nodes, `Gc` box headers); this is now bounded by the chokepoint's
per-iteration check rather than by that charge alone. Moving the check into the internal allocator
itself remains part of the Stage 4 compatibility-fork or upstream-PR work.

### Stage 3 & 4: Upstream Tracking or Compatibility Fork

If upstream `gc-arena` 0.7+ permanently rejects external allocation tracking hooks:

1. Phodopus will publish and maintain an organization-scoped fork (`bitty-terminal/gc-arena` or `phodopus-gc-arena`).
2. The fork will preserve zero-cost `Gc` pointers while adding explicit allocator budget callbacks necessary for deterministic sandbox execution.

---

## 5. Crates.io Publishing Policy

When publishing `phodopus` to `crates.io`:

- Cargo rejects crates whose dependencies rely exclusively on `git = "..."` without a crates.io-compatible source.
- Once ready for publishing, the dependency will specify both `version = "0.5.3"` and `git`:

```toml
gc-arena = { version = "0.5.3", git = "https://github.com/kyren/gc-arena", rev = "5a7534b883b703f23cfb8c3cfdf033460aa77ea9", features = ["allocator-api2", "hashbrown"] }
```

This guarantees local build reproducibility while satisfying crates.io package verification rules.
