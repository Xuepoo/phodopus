# phodopus agent guide

## Scope and authority

- This file governs only the independent `phodopus` Git repository under
  <https://github.com/bitty-terminal/phodopus>.
- The umbrella workspace directory is not a Git repository; other Bitty
  repositories own their own Git, CarryCtx, CI, releases, and agent guidance.
- Lineage & Attribution: `Phodopus` is the pure-Rust stackless Lua successor
  runtime forked from [`kyren/piccolo`](https://github.com/kyren/piccolo).
  All upstream commit history, copyright attributions, and MIT/CC0 licenses
  are strictly preserved. Upstream remote is tracked at
  `https://github.com/kyren/piccolo.git`.
- Canonical planning and design records originate in the research corpus
  (`research/summary/060.md`) and will bridge to `bitty-plugins-docs` and
  `bitty-terminal-docs` as integrations mature.
- Shared governance: decisions, security corpus, reviews, and project standards
  are defined in [`bitty-docs`](https://github.com/bitty-terminal/bitty-docs).

## Current phase

- Phase 0: Baseline fork initialized, repo identity and workspace integration
  established, quality gates passing cleanly.
- Approaching Phase 1: Review and absorb mature upstream Piccolo PRs
  (#128 `string.format`, #129 Lua pattern matching, #110 `utf8`, #121 `traceback`
  stack diagnostics).
- Do not introduce Bitty-specific abstractions or hardcoded async runtime
  assumptions into the VM core.

## Architectural boundaries

- **Strict Decoupling**: `Phodopus` is an independent, general-purpose,
  sandbox-first Lua runtime crate usable by any Rust application.
- **No Direct Tokio in VM Core**: Core VM execution remains synchronous and
  stackless. Asynchronous operations yield typed host suspension descriptors
  (`HostOp::Pending(handle)`), allowing external host schedulers (such as Tokio
  in Bitty) to drive futures and resume coroutines via a trampoline.
- **Sandboxing & Fuel**: Execution must remain deterministic, preemptible by
  instruction Fuel, and boundable by hard memory quotas.
- **Native Lua Patterns**: String pattern matching implements authentic Lua
  patterns (via PR #129 adaptation) rather than generic Rust regex syntax.

## Quality gates

- Always run quality gates via the repository `justfile`:
  - `just check` (runs `fmt-check`, `typecheck`, `clippy`, `test`)
  - `just fmt` (formats Rust code with `cargo fmt`)
- Toolchain: pinned to Rust 1.98.1 (`rust-toolchain.toml`), MSRV 1.85
  (`clippy.toml`).
- All tests must pass before proposing or merging any changes.

## Engineering discipline

- **No Hardcoded Values**: Never hardcode host/environment values: absolute
  paths, user home directories, hostnames, ports, credentials.
- **Language**: English-only for all code, comments, Markdown documentation,
  commit messages, and issue descriptions.
- **Git Hygiene**: Keep Git history clean, logical, and traceable to specific
  tasks or upstream PR references.
