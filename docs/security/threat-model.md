---
title: Threat Model & Trust Boundaries
description: Normative specification of sandbox defense capabilities, host callback trust boundaries, and resource isolation
category: security
audience: developers
document_type: policy
status: accepted
website_publish: true
sidebar_order: 31
---

# Threat Model & Trust Boundaries

> Status: **accepted**. This document defines the security architecture, threat model, defensive perimeter, and trust boundaries for Phodopus as an untrusted script execution engine.

---

## 1. System Vision & Security Mission

Phodopus is engineered to execute **untrusted or semi-trusted Lua 5.4 scripts** embedded within host applications (such as terminal emulators, plugin runtimes, game engines, or edge services) without endangering host stability, host memory integrity, or unauthorized ambient authority.

The primary security objective is: **A malicious or bugged script must never crash the host, access host resources without explicit authorization, or consume unbounded CPU/memory.**

---

## 2. Defensive Perimeter (What Phodopus Defends Against)

Phodopus provides mathematical and architectural guarantees against the following threat vectors:

### 2.1 Memory Safety & Spatial/Temporal Isolation

- **C-Style Memory Corruption**: Zero C dependencies. No `buffer overflow`, `use-after-free`, `double free`, or dangling pointer vulnerabilities in the Lua runtime.
- **Garbage Collector Escape**: `gc-arena` generative lifetimes (`'gc`) enforce at compile time that GC-managed values cannot outlive the arena or leak between distinct Lua states.
- **Native Stack Overflow**: The VM is completely stackless. Deep Lua recursion, coroutine ping-pong, and mutual tail calls allocate on the heap; they never exhaust the native OS/Rust thread stack.

### 2.2 Resource Exhaustion & Denial of Service (DoS)

- **Infinite CPU Loops**: Fuel budgeting decrements an instruction counter per opcode. When Fuel is exhausted, the VM unconditionally yields control back to the host scheduler (`ExecutorMode::Interrupted`).
- **Unbounded Memory Growth**: Memory allocations are metered at the allocator level (`MetricsAlloc` and `MemoryBudget`), preventing scripts from silently allocating gigabytes of RAM.
- **Regex ReDoS**: String pattern matching uses linear-bounded backtracking algorithms rather than exponential NFA regex engines.

### 2.3 Ambient Authority & Isolation

- **No Implicit Filesystem or Network Access**: Phodopus does not load `io.open`, `os.execute`, or `package.loadlib` by default.
- **No Ambient Host Stdout Leaks**: Standard `print` routing can be bound to abstract `OutputSink` traits rather than host process `stdout`.

---

## 3. Explicit Non-Goals (What Phodopus Does NOT Defend Against)

To prevent misplaced security assumptions, the following areas are outside Phodopus's threat perimeter:

1. **Buggy or Over-Privileged Host Callbacks**: If the host exposes a callback `host.read_file(path)` without validating paths, Phodopus cannot prevent the Lua script from reading sensitive files. **Host callbacks are trusted components in the threat model.**
2. **Microarchitectural Side-Channel Attacks**: Phodopus does not protect against hardware-level timing or cache side-channel attacks (e.g., Spectre/Meltdown).
3. **Rust Compiler / LLVM Exploits**: The security model relies on the soundness of the standard Rust compiler toolchain.

---

## 4. Trust Boundaries

```text
┌─────────────────────────────────────────────────────────────┐
│                       Host Application                      │
│                                                             │
│  [Untrusted Script] ──> [Phodopus VM Sandbox]               │
│                                │                            │
│                                │ (Typed Callback / Yield)   │
│                                ▼                            │
│                       [Host Callback API] ◄── TRUST BOUNDARY│
│                                │                            │
│                                ▼                            │
│                     [Host System / OS APIs]                 │
└─────────────────────────────────────────────────────────────┘
```

### 4.1 Host Callback Trust Boundary

Any Rust function exposed to Lua via `Callback::from_fn` crosses the trust boundary into host privileges:

- **Input Validation Requirement**: Host callbacks must validate all arguments received from Lua (string lengths, integer ranges, path normalization).
- **No Panic Leaks**: Host callbacks should return `Err(ExternError)` rather than panicking.
- **Resource Discipline**: Long-running operations inside host callbacks must yield asynchronously (`HostOp::Pending`) or consume appropriate Fuel.

### 4.2 Unsafe Rust Boundary

Unsafe Rust in Phodopus is strictly quarantined to:

1. `gc-arena` runtime primitives (tracing, pointer erasure).
2. VM opcode dispatch and table hash slot access.

Zero `unsafe` blocks are permitted in standard library implementations, user-facing callbacks, or module resolution logic.

---

## 5. Panic Safety & Sandbox Invariants

1. **Panics Do Not Escape**: A panic originating from Lua bytecode compilation or execution is caught at the `Lua::try_enter` boundary and converted to an error result.
2. **State Consistency**: If a panic occurs, the arena remains in a well-defined state or aborts the specific execution context without corrupting surrounding host memory.
3. **No Catch-All Ignorance**: Internal invariant violations (`unreachable!()`) indicate runtime compiler bugs and are treated as P0 security defects.
