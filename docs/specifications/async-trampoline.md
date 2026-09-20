---
title: Async Trampoline Specification
description: Normative specification for runtime-agnostic asynchronous operations and coroutine resumption trampolines
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: implemented
website_publish: true
sidebar_order: 24
---

# Async Trampoline Specification

> Status: Design **accepted** | Implementation: **implemented** (Phase 4; `HostOpHandle` bridge with `SequencePoll::Suspend`, `ExecutorMode::HostSuspended`, and `resume_host_op` / `cancel_host_op`). The prototype NOOP waker remains only for `AsyncSequence::pending` / in-VM yields; host-driven suspension goes through the typed bridge. This document defines the host-agnostic asynchronous suspension protocol, waker-less VM execution, and coroutine resumption trampolines for Phodopus.

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
3. **Host Yield**: The VM executor pauses the current `Thread` without unwinding native stack frames: the parked sequence moves into a `Frame::HostSuspended` marker, `Executor::step` returns `Ok(true)` (no further progress), `Executor::mode()` reports `ExecutorMode::HostSuspended`, and `Executor::pending_host_op` surfaces the handle to the outer host loop.
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

1. **Async Trampoline Loop Test**: Mock an async timer (deterministic tick counter, no real sleep in CI); assert the Lua script suspends, host advances, timer completes, and Lua coroutine resumes with correct values. Implemented in `crates/phodopus/tests/hostop.rs::mock_async_timer_resumes_with_correct_values`.
2. **Cancellation Test**: Cancel a pending host operation; verify Lua receives a catchable error via `pcall`. Implemented in `crates/phodopus/tests/hostop.rs::host_cancellation_surfaces_as_catchable_pcall_error`.
3. **Multi-Thread Suspension**: Concurrently suspend 100 independent Lua coroutines; assert all wake up and complete in non-blocking order. Implemented in `crates/phodopus/tests/hostop.rs::burst_of_100_suspended_coroutines_all_complete`.
4. **Fuel and Quota Accounting**: A parked step reports no progress and burns no fuel (the sequence is not re-polled); resume re-enters the executor-loop quota chokepoint. Implemented in `crates/phodopus/tests/hostop.rs::suspend_resume_preserves_fuel_and_quota_accounting`.
5. **Finalizer Notification**: A collected thread's parked op is reported by `HostOpRegistry::abandoned_handles` so the host drops the future; `HostOpGuard` notifies synchronously on drop. Implemented in `crates/phodopus/tests/hostop.rs::hostop_handle_finalizer_notifies_host_on_collect`.
6. **Sequence Compatibility**: `SequencePoll::Pending` re-poll semantics are unchanged alongside `Suspend`. Implemented in `crates/phodopus/tests/hostop.rs::suspend_goes_through_sequence_machinery_and_pending_still_works`.

## 6. Implementation Notes

- Suspension goes through the existing `Sequence` machinery, not a parallel path: the executor's `Frame::Sequence` poll arm handles `SequencePoll::Suspend` by moving the parked sequence into a `Frame::HostSuspended` marker (which owns the sequence and the handle). `Thread::mode()` maps the marker to `ThreadMode::Suspended`; `Executor::mode()` refines it to `ExecutorMode::HostSuspended` so the host trampoline can distinguish "waiting on an external future" from a plain Lua yield.
- A parked `step` returns `Ok(true)` immediately (the thread is not `Normal`, so the loop breaks before the per-iteration fuel/quota charges): parking is fuel-free, and only the suspending sequence step plus the resuming steps consume fuel. Resume/cancel replace the marker with a runnable `Frame::Sequence` (plus host values or a pending `HostOpCancelled` error), so the next step re-enters the quota chokepoint with accounting intact.
- `AsyncSequence::suspend(handle)` is the `async`-block spelling of the same protocol: it records a `SequenceOp::Suspend` that `poll_fut` translates to `SequencePoll::Suspend`, mirroring `pending()` / `SequencePoll::Pending`.
- The `HostOpRegistry` singleton tracks live `handle -> thread (weak)` entries per `Lua` instance; resume/cancel remove the entry they resolve. GC never runs Rust `Drop` inside the arena, so collection notification is an explicit host-driven sweep (`abandoned_handles` after `Lua::gc_collect`), complemented by the synchronous `HostOpGuard` RAII hook for bindings that hold one.
- Public surface (all in crate root): `HostOpHandle`, `HostOpRegistry`, `HostOpResult`, `HostOpValue`, `HostOpGuard`, `HostOpCancelled`, `HostOpError`, `SequencePoll::Suspend`, `ExecutorMode::HostSuspended`, `Executor::{pending_host_op, resume_host_op, cancel_host_op}`, `AsyncSequence::suspend`.
