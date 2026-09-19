---
title: Phodopus Architecture Overview
description: Pure-Rust stackless Lua execution model, gc-arena integration, and trampoline architecture
category: architecture
audience: developers
document_type: architecture
status: accepted
website_publish: true
sidebar_order: 11
---

# Architecture Overview

> Status: **accepted**. This document establishes the foundational execution and memory architecture of the Phodopus pure-Rust Lua runtime.

---

## 1. System Vision & Purpose

Phodopus provides an uncompromisingly safe, deterministic, and sandboxed Lua runtime implemented in pure Rust. Traditional Lua C-bindings (e.g. `mlua` binding to LuaJIT or PUC-Rio Lua) introduce significant challenges in high-assurance or multi-tenant desktop environments:

1. **Native Stack Dependency**: C Lua uses the native C call stack for coroutine yields and function frames, causing stack overflows under deep recursion and requiring complex `longjmp` / unwinding interop.
2. **Ambient Authority & Unbounded Resources**: C standard libraries inherently grant access to filesystem (`io`), operating system (`os`), and external processes without fine-grained hardware instruction or memory ceilings.
3. **Threading & Async Impedance**: Suspending a native C Lua call stack across an external asynchronous event loop (e.g. Tokio) requires dedicated OS threads or thread pool offloading.

Phodopus solves these challenges at the VM foundation by combining:

- A **stackless bytecode interpreter** running on an explicit frame heap.
- **Generative lifetime branding** via `gc-arena` for zero-cost, memory-safe garbage collection.
- Deterministic **Fuel preemption** and **hard memory limits**.
- An **external host trampoline** enabling zero-cost coroutine suspension across async boundaries.

```text
+---------------------------------------------------------------+
|                      Host Application                         |
|  (e.g., Bitty Terminal / Tokio / Embedder Orchestration)     |
+---------------------------------------------------------------+
                               |
               mutate() / step() / resume()
                               v
+---------------------------------------------------------------+
|                       Phodopus Core                           |
|  +-----------------------+     +----------------------------+ |
|  |     gc-arena Heap     |     |   Stackless VM Executor    | |
|  | - Table / String      |     | - Frame Stack (Heap)       | |
|  | - Closure / Upvalues  | <-> | - Fuel Decrement Counter   | |
|  | - Thread Coroutines   |     | - Sequence State Machines  | |
|  +-----------------------+     +----------------------------+ |
+---------------------------------------------------------------+
                               |
             HostOp::Pending(handle) Suspension
                               v
+---------------------------------------------------------------+
|                      Host Async Bridge                        |
|  (Drives background operations and resumes paused coroutines) |
+---------------------------------------------------------------+
```

---

## 2. Memory Model & Generative Lifetimes (`gc-arena`)

Memory safety within Phodopus without the overhead of reference counting (`Arc` / `Rc`) is achieved through `gc-arena`.

### 2.1 Generative Lifetime Branding

Every Lua value is parameterized by an invariant lifetime `'gc`:

```rust
pub enum Value<'gc> {
    Nil,
    Boolean(bool),
    Integer(i64),
    Number(f64),
    String(String<'gc>),
    Table(Table<'gc>),
    Function(Function<'gc>),
    Thread(Thread<'gc>),
    UserData(UserData<'gc>),
}
```

The lifetime `'gc` is "branded" dynamically by the `Lua::enter` / `Arena::mutate` closure using Rust's higher-ranked trait bounds (HRTB: `for<'gc> FnOnce(Context<'gc>)`):

1. **Isolation Guarantee**: Values belonging to one Lua arena cannot be leaked into or referenced by another arena; the compiler enforces this statically at zero runtime cost.
2. **Machine-Sized Pointers**: Inside the arena, `Gc<'gc, T>` pointers are raw pointer-sized and implement `Copy`.
3. **Cycle Collection**: The garbage collector executes an incremental, tri-color mark-and-sweep cycle that naturally collects cyclic references between tables, coroutines, and closures.

---

## 3. Stackless Trampoline Architecture

The Phodopus VM is "stackless": it does not consume the Rust native call stack to represent Lua call depth.

### 3.1 Frame Heap & Sequences

All function calls, upvalue closures, and coroutine switches are allocated as lightweight frame objects inside the GC heap. When a Lua function calls a Rust callback, the callback can complete immediately or return a `Sequence`:

```rust
pub trait Sequence<'gc>: Collect {
    fn poll(
        &mut self,
        ctx: Context<'gc>,
        exec: &mut Executor<'gc>,
        stack: &mut Stack<'gc>,
    ) -> Result<SequencePoll<'gc>, Error<'gc>>;
}
```

A `Sequence` is a multi-step state machine. The parent `Executor` drives the sequence across mutation steps:

```text
[Host Loop]
    |
    v
Arena::mutate() ---> Executor::step()
                         |
                         +---> Execute Bytecode OpCodes (Fuel--)
                         |
                         +---> Sequence::poll()
                                  |
                                  v
                            Yield / Return / Call Subroutine
```

### 3.2 Benefits of the Stackless Model

- **Immunity to Native Overflow**: A recursive Lua script hitting 100,000 stack depth merely grows the heap frame buffer until a memory ceiling is hit; it cannot cause an unrecoverable native OS stack overflow.
- **Microsecond Cold Starts**: A fresh `Lua` instance initializes in approximately **35 µs** with an initial base heap consumption of only **~11.2 KB**.
- **Cooperative Multitasking**: Thousands of independent Lua threads can be stepped concurrently within a single OS thread.

---

## 4. Deterministic Preemption via Fuel

Phodopus tracks execution cost using an instruction budget called **Fuel**. Every bytecode dispatch, table lookup, and sequence poll decrements the active Fuel counter:

1. When Fuel reaches zero, the VM cleanly yields control back to the driving host with `ExecutorMode::Interrupted`.
2. The host application decides whether to replenish Fuel, pause execution, or terminate the script.
3. This provides guaranteed resilience against infinite loops (`while true do end`) and CPU denial of service without requiring asynchronous POSIX signals or OS thread termination.

---

## 5. Security & Isolation Invariants

Phodopus maintains strict security invariants:

- **No Ambient Authority**: Standard filesystem, process execution, and network APIs are completely decoupled and absent from the base engine.
- **Sound Generative Branding**: `unsafe` blocks are restricted to low-level raw table manipulation and downcasting inside `gc-arena`, fully audited and verified.
- **Host Agnosticism**: The core runtime contains no dependency on Tokio, async-std, or any external platform runtime.
