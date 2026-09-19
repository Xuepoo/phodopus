---
title: Phodopus Evolution Roadmap
description: Six-phase development roadmap from Piccolo baseline fork to production sandbox runtime
category: architecture
audience: developers
document_type: architecture
status: accepted
website_publish: true
sidebar_order: 12
---

# Phodopus Evolution Roadmap

> Status: **accepted**. This document defines the sequential delivery phases for Phodopus, ordering dependencies from foundational fork stabilization through production deployment.

---

## 1. Roadmap Overview

```text
+-----------------------------------------------------------------+
| Phase 0: Baseline Fork & Workspace Integration      [COMPLETED] |
| - Git history preserved, dual licenses asserted, CarryCtx bound |
+-----------------------------------------------------------------+
                               |
                               v
+-----------------------------------------------------------------+
| Phase 1: Upstream PR Review & Absorption             [UPCOMING] |
| - PR #128 (format), #129 (patterns), #110 (utf8), #121 (trace)  |
+-----------------------------------------------------------------+
                               |
                               v
+-----------------------------------------------------------------+
| Phase 2: Sandboxed Module System (`require`)                    |
| - Pluggable searcher chain, preloaded modules, VFS abstraction   |
+-----------------------------------------------------------------+
                               |
                               v
+-----------------------------------------------------------------+
| Phase 3: Hard Memory Quotas & Sandboxing Ceilings               |
| - RuntimeBuilder::memory_limit, Fuel policies, OOM handling     |
+-----------------------------------------------------------------+
                               |
                               v
+-----------------------------------------------------------------+
| Phase 4: Host-Agnostic Asynchronous Bridge                      |
| - HostOp::Pending protocol, waker-less VM, host trampoline      |
+-----------------------------------------------------------------+
                               |
                               v
+-----------------------------------------------------------------+
| Phase 5: Bitty Core Integration & Lua Host ABI                  |
| - bitty-lua adapter, UI event bindings, plugin host bridge      |
+-----------------------------------------------------------------+
```

---

## 2. Phase Detail & Milestones

### Phase 0: Baseline Fork & Workspace Governance (Completed)

- [x] Clone upstream Piccolo repository, preserving all author history, commits, and tags.
- [x] Establish remote tracking (`origin` -> `bitty-terminal/phodopus`, `upstream` -> `kyren/piccolo`).
- [x] Establish workspace identity: `repo.toml`, `workspace.toml`, `AGENTS.md`.
- [x] Modernize toolchain configuration: `rust-toolchain.toml` (1.98.1), `clippy.toml` (MSRV 1.85).
- [x] Replace single-iteration loops in `meta_ops.rs` with labeled blocks to eliminate Clippy errors.
- [x] Initialize CarryCtx governance (`.carryctx/`), publish initial snapshot ref `refs/heads/carryctx-snapshots`.
- [x] Verify green baseline gates: 100% tests pass on `just check`.

### Phase 1: Upstream Community PR Review & Absorption

Upstream Piccolo has valuable, mature pull requests that solve core runtime gaps:

1. **PR #128: `string.format` Implementation**:
   - Implement standard Lua string formatting (`%s`, `%d`, `%x`, `%f`, `%q`).
   - Prevent unbounded memory expansion during format buffering.
2. **PR #129: Native Lua Pattern Matching**:
   - Incorporate authentic Lua pattern matching (`string.find`, `string.match`, `string.gsub`, `string.gmatch`).
   - Reject generic regex crate mapping in favor of pure Lua pattern syntax (character classes `%a`, `%d`, magic characters `^$()%.[]*+-?`, frontiers `%f`).
3. **PR #110: `utf8` Standard Library**:
   - Add standard Lua 5.3/5.4 `utf8` library (`utf8.char`, `utf8.codes`, `utf8.codepoint`, `utf8.len`, `utf8.offset`).
4. **PR #121: Backtrace & Source Diagnostics**:
   - Enhanced stack frame introspection and line-number error reporting via `debug.traceback`.

### Phase 2: Sandboxed Module System (`require`)

- Define a pluggable searcher chain interface for `package.searchers`.
- Support embedded preloaded modules (`package.preload`) for core built-ins.
- Implement capability-constrained virtual filesystem (VFS) resolvers, preventing arbitrary traversal of the host OS filesystem.

### Phase 3: Hard Memory Quotas & Sandboxing Ceilings

- Integrate hard memory limit support directly into the arena allocator:

  ```rust
  let lua = Lua::builder()
      .memory_limit(16 * 1024 * 1024) // 16 MiB hard quota
      .fuel_limit(100_000)             // 100k instruction budget
      .build();
  ```

- Implement clean, recoverable `OutOfMemory` errors without panics or VM corruption.
- Formalize preemption and cancellation policies.

### Phase 4: Host-Agnostic Asynchronous Bridge

- Replace Piccolo's NOOP waker stub with a formal `HostOp::Pending(handle)` suspension protocol.
- Allow the VM to yield typed suspension descriptors when external asynchronous operations (I/O, timers, IPC) are initiated.
- Provide a host trampoline adapter that maps pending operations to host schedulers (Tokio or async-std) and resumes Lua coroutines upon completion.

### Phase 5: Bitty Core Integration & Lua Host ABI

- Implement `bitty-lua` on top of Phodopus:
  - Terminal UI event bindings (`bitty.ui`, `bitty.panel`).
  - Spatial and semantic command registry bindings (`bitty.command`).
  - Sandboxed plugin file storage (`bitty.fs`).
- Deploy Phodopus as the primary runtime engine for non-AI Lua plugins and workspace scripting.
