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

Phodopus is **not** a zero-`unsafe` crate. The correct assurance claim is _audited, justified, and
gated_ `unsafe`. Every `unsafe` site is inventoried in the [Unsafe Code Ledger](unsafe-ledger.md)
with its soundness invariant, owning module, and exercising test, and is enforced by
`scripts/check-unsafe-ledger.sh`.

Audited `unsafe` is confined to the following categories:

1. `gc-arena` runtime primitives: pointer erasure and header casts (`any.rs`, `string.rs`,
   `callback.rs`), unchecked lock access during arena tracing, and manual `Collect` implementations.
2. VM table hash-slot access (`table/raw.rs`) and the `Send`/`Sync` marker on the inert pointer
   payload of `ExternLuaError` (`error.rs`).
3. Host utility lifetime-erasure (`phodopus-util/src/freeze.rs`) and the async sequence trampoline
   (`async_callback.rs`).

`unsafe` is **forbidden** in standard library and compiler trees; the ledger gate fails if any
appears there. Module resolution is not yet implemented, and the planned resolver will be added to
the gate's forbidden set when it lands. The user-facing callback API (`Callback::from_fn`,
`from_fn_with`) is safe: representation erasure is confined to the runtime's VTable machinery, and
host callback bodies cannot be forced to contain `unsafe`.

---

## 5. Panic Safety & Sandbox Invariants

**Panic containment is a host responsibility.** Phodopus installs no `catch_unwind` boundary at
`Lua::enter`, `Lua::try_enter`, `Lua::finish`, or `Lua::execute`; `try_enter` only maps the
runtime's typed `Error<'gc>` to a GC-free `ExternError` (`lua.rs:289-294`). This is a deliberate,
normative decision:

1. **Arena soundness.** The VM runs inside `gc_arena::Arena::mutate`, which is not panic-safe: a
   panic unwinding through an in-progress mutation can leave the arena's collection phase or a
   `RefLock` borrow in an inconsistent state. Catching such a panic and continuing to use the same
   `Lua` instance would be unsound, so the runtime must not claim to do so.
2. **Unwinding is observable, not swallowed.** A Rust panic raised by the VM or by a host callback
   unwinds across the runtime API. The host contains it at its own task, thread, or process
   boundary (for example, a plugin worker that owns the `Lua` instance).
3. **Internal panics are invariant guards.** The `panic!`/`unreachable!` sites in the executor,
   thread, compiler, and meta-operation modules document states that cannot be reached by valid
   Lua bytecode; they fire only on a compiler or VM bug. They are treated as P0 security defects,
   never as recoverable errors.

Host-visible behavior is pinned by `crates/phodopus/tests/panic_containment.rs`:

- A Lua `error(...)` is delivered as a typed `Err`, never a panic.
- A panicking host callback propagates an unwind to the host's `catch_unwind`, proving the host is
  the component that contains it.

There is intentionally no `catch_unwind` in any runtime source (`rg -n 'catch_unwind'
crates/phodopus/src crates/phodopus-util/src` returns no matches); the only occurrence in the
repository is in `tests/panic_containment.rs`, which supplies the host-side `catch_unwind` used to
demonstrate that the runtime does not swallow the unwind.
