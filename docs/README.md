---
title: Phodopus Documentation Map
description: Canonical navigation and authority rules for the Phodopus Lua runtime documentation
category: project
audience: mixed
document_type: index
status: accepted
website_publish: true
sidebar_order: 1
---

# Phodopus Documentation Map

This index is the central entry point and navigation map for the canonical documentation of the **Phodopus** pure-Rust stackless Lua runtime.

Phodopus is an independent, sandbox-first Lua runtime designed for uncompromising isolation, instruction Fuel budgeting, hard memory limits, and host-agnostic asynchronous execution. Fuel budgeting, stackless execution, and the absorbed Phase 1/1.5 standard library and `load` work are implemented; hard memory quotas, the module system, and the async bridge are target state. [Evolution Roadmap](architecture/roadmap.md) is the single implementation truth for phase status. Documentation drives implementation; every engineering task is guided by the specifications and architectural contracts contained in this corpus.

---

## Authority & Cross-Repository Context

- **Authority**: This `docs/` corpus is the sole canonical source of architecture, interface contracts, and specifications for the `phodopus` repository.
- **Upstream Lineage**: Phodopus is forked from Catherine West's ([@kyren](https://github.com/kyren)) Piccolo runtime, strictly preserving original copyright, MIT/CC0 dual-licensing, and full Git commit history.
- **Sibling Corpora**:
  - [bitty-docs](https://github.com/bitty-terminal/bitty-docs): Canonical cross-cutting governance, normative security baselines, and organization policies.
  - [bitty-plugins-docs](https://github.com/bitty-terminal/bitty-plugins-docs): Plugin platform architecture, manifest schema, and capability grants.
  - [bitty-terminal-docs](https://github.com/bitty-terminal/bitty-terminal-docs): Terminal platform core, PTY ownership, and layout engine.
- **Docs Self-Containment**: All documents in this directory are self-contained. They define architecture and specifications directly without referencing external transient scratch registers or draft tickets.

---

## Content Trees

The documentation is organized into focused topic trees:

| Tree                                       | Scope and Entry Point                                                                                                                                          | Status |
| :----------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------- | :----- |
| [Architecture](architecture/README.md)     | Pure-Rust VM execution model, `gc-arena` cycle collection, stackless trampolines, and the 6-phase roadmap.                                                     | Active |
| [Specifications](specifications/README.md) | Concrete contracts: Fuel & sandboxing, modular stdlib (`string.format`, Lua patterns, `utf8`), module resolver (`require`), and host async bridge.             | Active |
| [Security](security/README.md)             | Sandbox threat model, isolation guarantees, host callback boundaries, and resource limits.                                                                     | Active |
| [Integration](integration/README.md)       | Host-consumer boundary records: how an external project consumes Phodopus, beginning with the Bitty dependency relationship and `bitty-lua` Host ABI boundary. | Active |
| [Development](development/README.md)       | Documentation spines, engineering lifecycle, toolchain policies (Rust 1.98.1, edition 2024, MSRV 1.85), and CI verification.                                   | Active |

---

## Document Index

| Document                                                                   | Type          | Status   | Summary                                                                                                                                                                       |
| :------------------------------------------------------------------------- | :------------ | :------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Architecture Overview](architecture/overview.md)                          | Architecture  | Accepted | Stackless VM design, zero-cost `Gc` pointers with generative lifetimes, and host trampoline loops.                                                                            |
| [Evolution Roadmap](architecture/roadmap.md)                               | Architecture  | Accepted | Six-phase roadmap from Piccolo baseline fork to production sandbox runtime.                                                                                                   |
| [Garbage Collector Strategy](architecture/gc-strategy.md)                  | Architecture  | Accepted | Dependency pinning, memory accounting evolution, and hard quota strategy for `gc-arena`.                                                                                      |
| [Sandbox & Fuel Specification](specifications/sandbox-and-fuel.md)         | Specification | Partial  | Deterministic instruction budgeting, preemption thresholds, and hard heap allocation ceilings.                                                                                |
| [Modular Standard Library Specification](specifications/modular-stdlib.md) | Specification | Partial  | Capability-gated stdlib: formatting, authentic Lua patterns, UTF-8 code points, and table helpers.                                                                            |
| [Module Resolver Specification](specifications/module-resolver.md)         | Specification | Planned  | Sandboxed `require` searcher chains, preloaded modules, and capability VFS resolvers.                                                                                         |
| [Async Trampoline Specification](specifications/async-trampoline.md)       | Specification | Planned  | Host-agnostic async suspension protocol (`HostOp::Pending`) without Tokio VM coupling.                                                                                        |
| [Threat Model & Trust Boundaries](security/threat-model.md)                | Policy        | Accepted | Defensive boundaries, host callback trust boundaries, and the panic policy.                                                                                                   |
| [Unsafe Code Ledger](security/unsafe-ledger.md)                            | Reference     | Accepted | Exact inventory of every `unsafe` site, its soundness invariant, owning module, exercising test, and the `scripts/check-unsafe-ledger.sh` gate.                               |
| [Bitty Host ABI Boundary](integration/bitty-host-abi.md)                   | Specification | Accepted | Bitty consumes Phodopus as a generic dependency; `bitty-lua` host boundary, async trampoline crossing, text-layout separation, deferral, and mapped integration capabilities. |
| [Documentation Workflow](development/documentation-workflow.md)            | Guide         | Accepted | Section spines, review sign-off gates, and document maintenance workflow.                                                                                                     |
| [Toolchain Policy](development/toolchain-policy.md)                        | Policy        | Accepted | Rust 1.98.1 toolchain, edition 2024, MSRV 1.85, Clippy lint configurations, and justfile gates.                                                                               |
