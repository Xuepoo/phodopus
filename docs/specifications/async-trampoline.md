---
title: Async Trampoline Specification
description: Normative specification for runtime-agnostic asynchronous operations and coroutine resumption trampolines
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: planned
website_publish: true
sidebar_order: 24
---

# Async Trampoline Specification

> Status: Design **accepted** | Implementation: **planned** (scheduled for Phase 4; current runtime uses prototype NOOP waker). This document defines the host-agnostic asynchronous suspension protocol, waker-less VM execution, and coroutine resumption trampolines for Phodopus.

---

## 1. Purpose and Scope

### In Scope

- Suspension of Lua coroutines when initiating asynchronous operations (I/O, timers, network, IPC).
- Host-agnostic suspension descriptors (`HostOpHandle`).
- Resumption protocol via external host schedulers (Tokio, async-std, or custom event loops).
- Cancellation and timeout semantics for pending operations.

### Out of Scope

- Direct dependencies on Tokio or any specific async runtime within the VM core crate.
- Implementation details of specific host I/O primitives.

---

## 2. Background & Problem

Upstream Piccolo provides an `async_sequence` mechanism allowing callbacks to be authored using Rust `async` blocks. However, its internal waker implementation is a NOOP:

```rust
// Upstream Piccolo async_callback.rs limitation:
// Polling an external future that requires real OS wakeups will panic or hang!
```

If an async block awaits a real external future (such as a Tokio timer, TCP socket, or IPC channel), execution either deadlocks or panics because the VM cannot be parked on a foreign waker while retaining GC arena invariants.

---

## 3. Technical Specification

### 3.1 The `HostOp` Suspension Model

Phodopus decouples async execution entirely from the VM core:

```text
[Lua Coroutine]
       |
       | calls async function (e.g. sleep(100ms))
       v
[Rust Callback / Sequence]
       |
       | registers future with Host Bridge -> gets HostOpHandle
       v
Returns SequencePoll::Suspend(HostOpHandle)
       |
       +---> Control returns cleanly to Host Trampoline
             (Lua Coroutine is in "Suspended" state in GC heap)
```

### 3.2 Protocol Sequence

1. **Initiation**: A Rust binding callback registers an external `Future` with the host adapter and receives an opaque handle:

   ```rust
   pub struct HostOpHandle(u64);
   ```

2. **Suspension**: The callback returns `SequencePoll::Suspend(handle)`.
3. **Host Yield**: The VM executor pauses the current `Thread` without unwinding native stack frames and returns `ExecutionResult::Suspended(handle)` to the outer host loop.
4. **External Polling**: The host application (e.g., Tokio event loop) drives the future independently.
5. **Resumption**: When the future resolves with results, the host invokes:

   ```rust
   executor.resume_host_op(ctx, handle, return_values)?;
   ```

   The Lua thread is marked runnable, and execution continues at the next instruction.

### 3.3 Cancellation and Timeout Handling

- If a host operation times out or is cancelled by host policy:
  - The host invokes `executor.cancel_host_op(ctx, handle, "operation timed out")`.
  - The suspended sequence receives a cancellation error, which unwinds through Lua `pcall` handlers normally.
- If the Lua coroutine is garbage-collected or closed before completion:
  - The `HostOpHandle` finalizer notifies the host to drop the underlying `Future`.

---

## 4. Architectural Invariants

- **Zero Core Coupling**: `phodopus` does not declare `tokio` in its dependencies. The suspension protocol operates strictly through numeric/typed handles and generic callbacks.
- **GC Isolation**: During the time a `HostOp` is pending across the network, no GC pointers (`Gc<'gc, T>`) are held by the external future. All transferred values are deserialized into host memory or stashed in stable root handles.

---

## 5. Verification Plan

1. **Async Trampoline Loop Test**: Mock an async timer that completes after 50ms; assert the Lua script suspends, host advances, timer completes, and Lua coroutine resumes with correct values.
2. **Cancellation Test**: Cancel a pending host operation; verify Lua receives a catchable error via `pcall`.
3. **Multi-Thread Suspension**: Concurrently suspend 100 independent Lua coroutines; assert all wake up and complete in non-blocking order.
