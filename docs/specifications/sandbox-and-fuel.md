---
title: Sandbox & Fuel Specification
description: Normative specification for instruction Fuel budgeting, hard memory quotas, and execution preemption
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: partial
website_publish: true
sidebar_order: 21
---

# Sandbox & Fuel Specification

> Status: Design **accepted** | Implementation: **partial** (Fuel instruction budgeting implemented; hard heap allocation quotas planned). This document defines the normative resource bounding, instruction Fuel budgeting, and hard memory quota contracts for Phodopus.

---

## 1. Purpose and Scope

### In Scope

- Instruction Fuel accounting rules across VM bytecode opcodes and sequence polling.
- Preemption semantics when Fuel is exhausted (`ExecutorMode::Interrupted`).
- Hard memory allocation ceilings enforced by the allocator.
- Out-of-memory (OOM) error handling without VM panics or memory corruption.
- API surface for setting and replenishing Fuel and memory limits.

### Out of Scope

- Host OS process sandboxing (namespaces, cgroups, pledge/unveil).
- Asynchronous task scheduling policies in external hosts.

---

## 2. Normative Sources

This specification must not weaken:

- **Bitty Security Policy**: Untrusted scripts must not be capable of causing memory exhaustion, unbounded CPU monopolization, or thread lockup.
- **Pure-Rust Invariant**: All allocation failures and execution preemption must resolve cleanly without native `panic!` or undefined behavior.

---

## 3. Terminology

- **Fuel**: An abstract, deterministic unit of execution work roughly corresponding to one bytecode instruction or atomic VM operation.
- **Hard Memory Quota**: An absolute byte ceiling on total heap allocations within a `gc-arena` instance.
- **Interrupted Mode**: A clean VM state where execution pauses because Fuel has reached zero, preserving the entire frame stack for subsequent resumption.

---

## 4. Technical Specification

### 4.1 Fuel Accounting Model

The VM executor maintains an internal integer counter `fuel: i32`.

1. **Instruction Cost**:
   - Standard arithmetic and register move opcodes decrement Fuel by `1`.
   - Table lookup (`OP_GETTABLE`), table store (`OP_SETTABLE`), and closures decrement Fuel by `2`.
   - Function calls (`OP_CALL`) and tail calls decrement Fuel by `3`.
   - Metamethod resolution and sequence polling decrement Fuel by `4`.
2. **Exhaustion Behavior**:
   - When `fuel <= 0`, the executor immediately interrupts execution at the current instruction boundary and returns `Ok(ExecutionResult::Interrupted)`.
   - The executor state remains valid. The caller may replenish Fuel (`exec.fuel_mut().add(budget)`) and call `exec.step()` to continue seamlessly.

```rust
pub struct Fuel {
    remaining: i32,
}

impl Fuel {
    pub fn new(budget: i32) -> Self;
    pub fn remaining(&self) -> i32;
    pub fn add(&mut self, amount: i32);
    pub fn clear(&mut self);
    pub fn consume(&mut self, amount: i32) -> bool;
}
```

### 4.1.1 Variable-Cost Standard Library Callbacks

A callback whose work is not fixed by the VM instruction count must charge that
work against `Execution::fuel` proportionally. Phodopus uses one shared,
deterministic model (`crates/phodopus/src/stdlib/sandbox.rs`):

| Unit               | Fuel cost | Applied to                                                                                                                                                                      |
| ------------------ | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Output byte        | `1`       | Every byte appended to a growing buffer (`format`, `gsub`, `utf8.char`, `string.sub`/`upper`/`lower`/`reverse`/`char`, `string.rep`, `table.concat`, `load` piecewise assembly) |
| Scanned input byte | `1`       | Every byte examined by a scan (`utf8.len`/`codepoint`/`offset`/`codes`, `tonumber` in both the single-argument and explicit-base forms)                                         |
| Format directive   | `4`       | Each `string.format` conversion expanded                                                                                                                                        |
| Pattern attempt    | `16`      | Each candidate start position tried by the pattern engine (`find`, `match`, `gmatch`, `gsub`)                                                                                   |
| Format-string byte | `1`       | Each byte advanced over by `string.pack`/`unpack`/`packsize`                                                                                                                    |

Operations that can exceed the wall slice of a single `Executor::step` are
implemented as resumable `Sequence`s using the existing `SequencePoll`/`Frame::Sequence`
mechanism. Each `poll` processes one batch sized from the remaining Fuel and
returns `SequencePoll::Pending` when the slice is exhausted, preserving its
cursor and partial output; replenishing Fuel resumes exactly where the previous
poll stopped:

- `string.format`, `string.rep`-style output assembly (`FormatSequence`);
- `string.gsub` (`GsubSequence`);
- `string.pack`, `string.unpack`, `string.packsize` (`PackSequence`,
  `UnpackSequence`, `PacksizeSequence`);
- `utf8.len`, `utf8.codepoint`, `utf8.offset` (`LenSequence`,
  `CodepointSequence`, `OffsetSequence`).

Operations that are proven constant-bounded remain one-shot but still charge
their work:

- `string.rep` computes the exact capacity with checked arithmetic and rejects
  anything above `MAX_STRING_REP_BYTES` (16 MiB) **before** allocating; it then
  charges one Fuel per output byte.
- `string.pack`/`unpack`/`packsize` advance a byte cursor and can yield between
  format options, so a long format string is preemptible.
- `string.find`/`string.match`/`gmatch` perform at most one unanchored search
  per call and charge `bytes + attempt bound`; the search engine call itself is
  not preemptible below its `MAX_RECURSION_DEPTH` bound (see Residual Bounds).
- `table.concat` copies the already-resident input values and charges one Fuel
  per output byte; its size is bounded by the sum of its inputs.
- `load` (piecewise function chunks) charges one Fuel per assembled byte, in
  addition to the existing dynamic-compilation charge of one Fuel per 32 source
  bytes in the string-chunk path.

### 4.1.2 Checked Output Ceilings

Independent of the (planned) global heap quota, every growing standard library
buffer is bounded by `MAX_STDLIB_STRING_BYTES` (16 MiB). Before each append,
`string.format`, `string.gsub`, and `utf8.char` compute the checked new length;
overflow or a value above the ceiling raises a clean Lua error
(`"resulting string too large"`) instead of allocating. `string.rep` and the
pack family keep their existing 16 MiB operation ceilings.

### 4.1.3 Residual Bounds (documented, not hidden)

The following work is bounded but not preemptible mid-call, and is therefore
charged up front rather than yielded out of:

- A single `lsonar::engine::find_first_match` call: the engine recurses at most
  `MAX_RECURSION_DEPTH` (500) levels and walks at most the remaining input
  window, so one call is bounded by `O(remaining_bytes)` attempts. `gsub`
  charges `remaining_window + 1` attempts per call and yields between matches;
  `find`/`match`/`gmatch` charge the same bound for their single call.
- Rust standard library formatting of one `string.format` directive
  (`format_value`) is bounded by the parser's `MAX_WIDTH`/`MAX_PRECISION`
  (1000) limits and the input string length.

These bounds are the reason the model does not claim strict preemption _inside_ a
single pattern search; the specification states the real bound instead of
overclaiming.

### 4.2 Hard Memory Ceilings (`MemoryQuota`)

> Status: **planned (not implemented)**. The allocator-side hard quota is the
> subject of the separate memory-quota task. Until it lands, the only VM-wide
> bound is the per-operation checked ceiling described in §4.1.2; the
> observational `Lua::total_memory()` API does not enforce anything.

Memory allocation within `gc-arena` utilizes a custom allocator tracking allocated bytes against a hard limit:

```rust
pub struct MemoryLimit {
    max_bytes: usize,
    current_bytes: usize,
}
```

1. **Quota Enforcement**:
   - Before any allocation or reallocation in the GC heap, the allocator checks if `current_bytes + requested <= max_bytes`.
   - If exceeded, an incremental GC cycle is immediately attempted.
   - If memory remains insufficient after collection, allocation fails and returns `Err(RuntimeError::OutOfMemory)`.
2. **Error Safety**:
   - The VM frames, tables, and threads remain consistent upon OOM; no partially-initialized values leak into the heap.
   - OOM errors can be caught via `pcall` if sufficient recovery memory remains, or propagated cleanly to the host.

---

## 5. Security Review

- **Denial of Service Prevention**: Malicious scripts containing infinite loops (`while true do end`) or memory-bomb constructs (`local t = {}; while true do t = {t} end`) are constrained by the Fuel and Quota bounds.
- **Determinism**: For a given Lua bytecode stream and input data, Fuel consumption is deterministic across platforms and execution environments.

---

## 6. Verification Plan

1. **Infinite Loop Test**: Run a tight loop with a 50,000 Fuel budget; assert execution halts precisely in `Interrupted` mode.
2. **Replenishment Test**: Replenish 20,000 Fuel to an interrupted VM; assert execution resumes and advances.
3. **Variable-Cost Interruption Tests** (`crates/phodopus/tests/fuel_stdlib.rs`): under a small Fuel budget, assert `string.format`, `string.gsub`, `utf8.len`, and `string.pack`/`unpack` are interrupted at least once and produce the same result as an uninterrupted run (state preserved and resumable).
4. **Checked Output Ceiling Tests**: assert a hostile `string.format "%s%s"` over 16 MiB and a hostile `gsub` replacement over 16 MiB both fail via `pcall` with `"resulting string too large"` and no allocation blowup.
5. **Memory Ceiling Test** (planned): configure an 8 MiB quota; allocate large string arrays; assert clean `OutOfMemory` error without native abort or memory corruption.

---

## 7. Acceptance Criteria

- Every standard library callback either consumes proportional Fuel or is proven constant-bounded with a documented justification (§4.1.1, §4.1.3).
- `RuntimeBuilder::fuel_limit(n)` and `RuntimeBuilder::memory_limit(bytes)` configure strict runtime limits. _(Fuel limit is available on `Fuel`; the builder-level APIs remain part of the memory-quota work.)_
- Zero occurrences of native panics when scripts exceed execution bounds.
- Unit and integration tests covering Fuel exhaustion, variable-cost interruption, checked output ceilings, and OOM recovery pass 100% green.
