---
title: Sandbox & Fuel Specification
description: Normative specification for instruction Fuel budgeting, hard memory quotas, and execution preemption
category: specifications
audience: developers
document_type: specification
status: accepted
website_publish: true
sidebar_order: 21
---

# Sandbox & Fuel Specification

> Status: **accepted**. This document defines the normative resource bounding, instruction Fuel budgeting, and hard memory quota contracts for Phodopus.

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

### 4.2 Hard Memory Ceilings (`MemoryQuota`)

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
3. **Memory Ceiling Test**: Configure an 8 MiB quota; allocate large string arrays; assert clean `OutOfMemory` error without native abort or memory corruption.

---

## 7. Acceptance Criteria

- `RuntimeBuilder::fuel_limit(n)` and `RuntimeBuilder::memory_limit(bytes)` configure strict runtime limits.
- Zero occurrences of native panics when scripts exceed execution bounds.
- Unit and integration tests covering Fuel exhaustion and OOM recovery pass 100% green.
