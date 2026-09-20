---
title: Bitty Host ABI Boundary
description: Accepted dependency and boundary rules for Bitty consuming Phodopus, the bitty-lua Host ABI, async trampoline crossing, and text-layout separation
category: integration
audience: developers
document_type: specification
status: accepted
website_publish: true
sidebar_order: 41
---

# Bitty Host ABI Boundary

> Status: **accepted** for the dependency relationship and the host-boundary rules (Sections 6.1 through 6.6), which record decided owner direction and must not be weakened. Each point below is individually marked **Accepted** or **Open**; nothing here is presented as decided beyond what an accepted source records. Undecided surfaces are listed under [Open Points](#67-open-points-not-decided). Phodopus has completed Phases 1 and 1.5 of its roadmap but the capabilities this boundary depends on (module resolution, hard quotas, async bridge) remain open; this page records design-level boundaries, and the async and quota mechanisms it names are target state, not shipped behavior (see [Evolution Roadmap](../architecture/roadmap.md)).

## Purpose and Scope

Phodopus is an independent, general-purpose Lua runtime. Bitty consumes it as an ordinary Rust dependency; the Bitty-specific integration layer is `bitty-lua`, which mounts strictly on top. This document is the single canonical record of that relationship on the Phodopus side: what Bitty may rely on, what Bitty must never inject into the runtime core, and which Phodopus capabilities Bitty's eventual adoption depends on. It exists so the boundary does not have to be reconstructed from the README lineage line, the roadmap Phase 5 entry, and the architecture host row.

### In Scope

- The dependency relationship between Bitty and Phodopus, and the placement of `bitty-lua` as the only Bitty-specific consumer layer.
- The host-boundary rules: no Bitty-specific abstractions and no hard asynchronous-runtime dependency in the VM core.
- The asynchronous crossing contract: `HostOp::Pending(handle)` trampoline semantics and host-adapter driving.
- The separation of standard `utf8.*` code-point semantics from terminal text layout.
- The `bitty-lua` implementation deferral and its effect on Bitty's current accepted runtime contract.
- The Phodopus capabilities Bitty depends on for a later switch, each mapped to its owning specification in this corpus.

### Out of Scope

- The generic VM, bytecode compiler, `gc-arena` cycle collector, stackless executor, and Fuel mechanism; these are specified in the [Architecture Overview](../architecture/overview.md) and [Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md).
- The implementation of the modular standard library, Lua patterns, and `utf8.*`; these are specified in the [Modular Standard Library Specification](../specifications/modular-stdlib.md).
- The `require` searcher chain implementation; specified in the [Module Resolver Specification](../specifications/module-resolver.md).
- Bitty-side plugin capability grants, manifest grammar, package lifecycle, and terminal UI contracts; these are owned by the Bitty consumer repositories and referenced, never restated, here.
- Any migration, crate extraction, version pin, or scheduling commitment for a runtime swap.

## Normative Sources This Specification Must Not Weaken

This specification must not weaken any of the following:

- **The accepted runtime specifications of this corpus**: the [Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md), the [Modular Standard Library Specification](../specifications/modular-stdlib.md), the [Module Resolver Specification](../specifications/module-resolver.md), and the [Async Trampoline Specification](../specifications/async-trampoline.md). Where this page and one of those documents overlap, the owning specification wins.
- **The host-agnostic invariant**: the runtime core declares no dependency on a specific asynchronous runtime and contains no host-specific abstraction (see [Architecture Overview](../architecture/overview.md) Section 5 and the [Async Trampoline Specification](../specifications/async-trampoline.md) Section 4).
- **The shared governance decision** that selected Phodopus as Bitty's Lua successor path, referenced under [Consumer records](#consumer-records); it remains the authority for the relationship, and this page does not amend it.
- **The shared Bitty security corpus** (`bitty-docs`), which owns the normative sandbox, capability, and resource-control requirements that any host integration must satisfy.

If any wording here conflicts with a normative source, the normative source wins.

## Terminology

| Term                          | Meaning in this document                                                                                                                                                     |
| :---------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Phodopus core**             | The generic runtime crates (`phodopus-*`): VM, compiler, executor, standard library, module resolver, sandbox, and async bridge. Host-agnostic and Bitty-agnostic.           |
| **`bitty-lua`**               | The Bitty-specific consumer layer that binds terminal, panel, command, and filesystem surfaces onto the generic runtime. The only Bitty-specific integration layer.          |
| **Host ABI boundary**         | The interface and rules governing how a host may consume the runtime: what crosses, in which direction, and what must never be assumed.                                      |
| **`HostOp::Pending(handle)`** | The typed suspension descriptor by which an asynchronous host operation crosses out of the VM (see [Async Trampoline Specification](../specifications/async-trampoline.md)). |
| **Host adapter**              | The host-side component that receives a pending handle, drives the corresponding future, and resumes the suspended coroutine. Never part of the VM core.                     |

| Status       | Meaning in this document                                                                                                   |
| :----------- | :------------------------------------------------------------------------------------------------------------------------- |
| **Accepted** | The point is decided by accepted owner direction or an accepted specification; this page records it and may not weaken it. |
| **Open**     | A question left unresolved until an owning contract decides it.                                                            |

## Technical Body

### 6.1 Dependency Relationship (Accepted)

**Accepted.** Bitty consumes Phodopus as an **independent, general-purpose Rust dependency**. The relationship has three parts:

1. **Phodopus is generic and stays generic.** The runtime core is host-agnostic and Bitty-agnostic. It contains no knowledge of terminals, panels, commands, plugin storage, or any Bitty subsystem.
2. **Bitty is one consumer among others.** Nothing about the runtime's public contracts is shaped by Bitty's requirements; a host-specific need is met by the host layer, not by the core.
3. **`bitty-lua` is the only Bitty-specific consumer layer.** All Bitty host surfaces mount strictly on top of the generic runtime, as an unprivileged consumer. The Phase 5 host surfaces are `bitty.ui`, `bitty.panel`, `bitty.command`, and `bitty.fs` (see [Evolution Roadmap](../architecture/roadmap.md) Phase 5).

The relationship is directional: Bitty depends on Phodopus, never the reverse. A change to the runtime core that would exist only to serve Bitty is a boundary violation, not a feature.

### 6.2 Host ABI Boundary Contract (Accepted)

**Accepted.** Two hard rules bound the interface between Bitty and the runtime core:

1. **No Bitty-specific abstractions in the core.** `bitty.ui`, `bitty.panel`, `bitty.command`, and `bitty.fs` are host-layer concepts. They must not appear in the VM core, its standard library, its module resolver, or its sandbox API.
2. **No hard asynchronous-runtime dependency in the core.** The VM core must build and run without Tokio, async-std, or any other specific scheduler. Runtime selection is a host decision, not a runtime-core requirement.

The accepted [Async Trampoline Specification](../specifications/async-trampoline.md) already requires zero core coupling (Section 4); this section applies that same rule specifically to the Bitty relationship and adds no new mechanism. The exact crate split and the host-seam module boundary between `bitty-lua` and the generic runtime are **Open** (see [Open Points](#67-open-points-not-decided)); crate names in the roadmap and consumer records are illustrative until an owning contract accepts them.

### 6.3 Asynchronous Crossing: The `HostOp::Pending(handle)` Trampoline (Accepted)

**Accepted** as the target boundary contract. Asynchronous host operations will cross the boundary as the typed `HostOp::Pending(handle)` suspension descriptor, and the crossing is directional. The protocol is not yet implemented in the runtime core (Phase 4 target state); the current core retains the Piccolo NOOP waker:

- The **VM core** yields the typed pending handle and parks the Lua coroutine in the GC heap without unwinding native stack frames.
- The **host adapter** — owned by the host, not the core — receives the handle, drives the corresponding future on the host scheduler, and resumes the suspended coroutine with the result or a cancellation error.

This is the accepted [Async Trampoline Specification](../specifications/async-trampoline.md) protocol (Sections 3.1 through 3.3) applied at the Bitty boundary. It composes with, and does not restate, the accepted host-service and `Send` contracts owned by the Bitty consumer corpora. The `HostOp` variant set, handle representation, cancellation semantics, and bounded pending-queue size are **Open**.

### 6.4 Text-Layout Separation (Accepted)

**Accepted.** Standard `utf8.*` operates on Unicode **code points**. Terminal text layout — grapheme clusters, emoji modifiers, and East Asian double-width handling — belongs to the terminal host layer (`bitty.text`), not to Phodopus.

This page does not restate that invariant; it cross-references its authoritative definition in the [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.3, which already asserts it as an architectural invariant. The Bitty-side width surface name and shape are owned by the terminal platform corpus, not by this repository.

### 6.5 Deferral (Accepted)

**Accepted.** Implementation work in `bitty-lua` is **deferred until Phodopus is usable** at the phase the integration requires. This is a scheduling statement, not a rejection:

- Bitty's current accepted `mlua` over vendored Lua 5.4 runtime contract is **unchanged** until a migration is authorized and parity evidence exists.
- No runtime swap is scheduled, claimed, or implied by this page; no pin is changed.
- The deferral ends only when an owning contract records a readiness gate and migration evidence. That readiness gate is **Open**.

### 6.6 Integration Expectations (Accepted set, mapped to owning specifications)

**Accepted as the set of capabilities Bitty depends on for the eventual switch; the shape of each is owned by its specification.** Bitty's adoption of Phodopus presumes the following runtime capabilities are available and reviewed. Each is mapped to the owning specification in this corpus:

| Capability                            | Why Bitty depends on it                                                                              | Owning specification                                                                                                                                    |
| :------------------------------------ | :--------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Module resolution and `require`       | Sandboxed, capability-rooted module loading for configuration and plugins.                           | [Module Resolver Specification](../specifications/module-resolver.md)                                                                                   |
| `string.format`                       | Standard Lua string formatting for configuration and output.                                         | [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.1                                                               |
| Authentic Lua patterns                | `find`, `match`, `gmatch`, and `gsub` with Lua pattern semantics rather than Rust `regex` semantics. | [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.2                                                               |
| `utf8.*`                              | Code-point decoding, iteration, and encoding, kept distinct from terminal typography.                | [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.3                                                               |
| Stack diagnostics (`debug.traceback`) | Actionable error reporting across host/coroutine boundaries.                                         | **Open** — Phase 1 upstream absorption ([Evolution Roadmap](../architecture/roadmap.md) Phase 1); no dedicated specification exists in this corpus yet. |
| Host-agnostic async bridge            | Driving host I/O, timers, and IPC without coupling the core to a scheduler.                          | [Async Trampoline Specification](../specifications/async-trampoline.md)                                                                                 |
| Hard memory quotas and Fuel           | Bounded, attributable resource control for untrusted scripts.                                        | [Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md)                                                                                   |

The stack-diagnostics row is the one capability without an owning specification in this corpus; it is recorded as **Open** rather than implied as specified. All other rows already have accepted owners and must not be weakened by integration work.

### 6.7 Open Points (Not Decided)

None of these is a global open question in this repository's registers; each is left to its owning contract. The accepted relationship and boundary rules in Sections 6.1 through 6.5 do not depend on them.

- The exact crate split, crate names, and the host-seam module boundary between `bitty-lua` and the generic runtime (Sections 6.1 and 6.2).
- The `HostOp` variant set, handle representation, cancellation, ordering, and bounded pending-queue semantics (Section 6.3).
- The terminal-side width surface name and grapheme-helper shape (Section 6.4); owned by the terminal platform corpus.
- The phase-to-milestone mapping and the readiness gate that ends the `bitty-lua` deferral (Section 6.5).
- An owning specification for stack diagnostics (`debug.traceback`) (Section 6.6).

## Security Review

| Concern                            | Required control                                                                                                                                                                     | Source                                                                                            |
| :--------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------ |
| Runtime-core host coupling         | No Bitty abstraction and no hard async-runtime dependency in the core; `bitty-lua` is the only Bitty consumer.                                                                       | This document (Accepted); [Async Trampoline Specification](../specifications/async-trampoline.md) |
| Ambient host access                | `require` reaches host paths only through capability-rooted resolvers; unadmitted paths fail closed.                                                                                 | [Module Resolver Specification](../specifications/module-resolver.md)                             |
| Resource exhaustion                | **Target state**: hard memory quotas and instruction Fuel must bound untrusted scripts with no unbounded execution or allocation. Fuel is implemented; allocator quotas are Phase 3. | [Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md)                             |
| Async re-entrancy and stalls       | Pending handles are bounded and cancellable by host policy; the VM core never blocks on a foreign waker.                                                                             | [Async Trampoline Specification](../specifications/async-trampoline.md)                           |
| Semantic substitution              | Lua patterns are implemented natively; mapping to Rust `regex` is rejected to avoid silent behavior change.                                                                          | [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.2         |
| Unicode/typography conflation      | `utf8.*` stays code-point based; terminal width and graphemes stay terminal-side.                                                                                                    | [Modular Standard Library Specification](../specifications/modular-stdlib.md) Section 4.3         |
| Contract creep across the boundary | This page adds no capability, weakens no accepted control, and promotes no open point to a decision.                                                                                 | This document (Accepted)                                                                          |

This section restates boundaries owned by accepted specifications and introduces no new control. Any integration mechanism that would weaken an accepted security control is out of scope until the owning contract changes.

## Verification Plan

This is a design-boundary document; it adds no executable behavior of its own. Verification is therefore by review and by the gates of any future integration task:

1. **Boundary review**: confirm no Bitty-specific identifier appears in the generic runtime core and no asynchronous-scheduler dependency is declared by the core crates.
2. **Deferral check**: confirm the document states no runtime swap and no pin change; the accepted `mlua`/Lua 5.4 contract remains the recorded position.
3. **Cross-reference integrity**: confirm each capability in Section 6.6 resolves to its owning specification, and that Section 6.4 defers to the accepted invariant rather than restating it divergently.
4. **Consumer-record integrity**: confirm the Bitty-side pointers resolve and are cited as consumer records, not duplicated content.
5. **Documentation gates**: `just check` passes with zero formatting, lint, and link issues.

Implementation evidence for the mapped capabilities belongs to the owning specifications and their future implementation tasks, not to this page.

## Alternatives Considered

| Alternative                                     | Disposition                                                                                                                                                                                                                               |
| :---------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Place this record inside `docs/specifications/` | Rejected. That tree holds generic, host-agnostic runtime contracts; a Bitty-specific consumer record there would blur the boundary this page defines. The `integration/` tree separates host-consumer boundaries from runtime guarantees. |
| Put Bitty-specific APIs in the runtime core     | Rejected. It would make the runtime Bitty-specific and un-auditable as a general-purpose engine; `bitty-lua` stays the only consumer layer.                                                                                               |
| Bind the VM core to a specific async runtime    | Rejected. It would couple the runtime to one scheduler and contradict the accepted async trampoline contract; the typed suspension descriptor keeps the core scheduler-free.                                                              |
| Conflate `utf8.*` with terminal width           | Rejected. It would mix code points with cell width, graphemes, and East Asian Width; the accepted invariant keeps them separate.                                                                                                          |
| Implement `bitty-lua` immediately               | Rejected. It would target an unusable runtime and change an accepted contract without parity evidence; the deferral preserves the current contract.                                                                                       |
| Duplicate the Bitty consumer records here       | Rejected. The owning repositories remain authoritative; this page links them and records only the Phodopus-side boundary.                                                                                                                 |

## Affected Contracts

- **[Async Trampoline Specification](../specifications/async-trampoline.md)** (Accepted): this page applies its `HostOp::Pending(handle)` protocol and zero-core-coupling rule to the Bitty relationship; the specification is not edited.
- **[Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md)** (Accepted): unchanged; named as an integration dependency.
- **[Modular Standard Library Specification](../specifications/modular-stdlib.md)** (Accepted): unchanged; Section 4.3 remains the authoritative text-layout separation, cross-referenced here.
- **[Module Resolver Specification](../specifications/module-resolver.md)** (Accepted): unchanged; named as an integration dependency.
- **[Evolution Roadmap](../architecture/roadmap.md)** (Accepted): Phase 5 records the `bitty-lua` host surfaces; Phase 1 records the stack-diagnostics absorption that Section 6.6 marks Open.
- **`docs/README.md`**: the documentation map gains the Integration content tree and this page in its document index.
- **Bitty consumer records**: owned elsewhere; linked below, never duplicated.

### Consumer records

These are the Bitty-side records of the same direction. They are consumer records: this repository does not restate their content, and they do not override the runtime specifications above.

- `bitty-docs` governance decision: [ADR-0012 — Phodopus Runtime as the Lua Successor Path](https://github.com/bitty-terminal/bitty-docs/blob/main/docs/decisions/adrs/ADR-0012-phodopus-runtime.md). This is the accepted source of the relationship and the deferral; it selects Phodopus as the successor direction without migrating code or changing pins.
- `bitty-plugins-docs` runtime corpus: [Runtime contracts](https://github.com/bitty-terminal/bitty-plugins-docs/blob/main/runtime/README.md) — the accepted plugin-host runtime, Lua runtime, and isolation/resource contracts that the eventual plugin-side integration must satisfy.
- `bitty-terminal-docs` terminal-side candidate: [Phodopus Host ABI (Candidate)](https://github.com/bitty-terminal/bitty-terminal-docs/blob/main/specifications/phodopus-host-abi-candidate.md) — a **draft** candidate (not accepted, not normative) recording terminal-side direction; it authorizes no shipped behavior and is not the authority for this boundary.

## Acceptance Criteria

1. The dependency relationship, the two host-boundary rules, the async crossing, and the text-layout separation are recorded as **Accepted** and are consistent with the accepted specifications they cite.
2. The `bitty-lua` deferral and the unchanged `mlua`/Lua 5.4 position are stated explicitly, with no swap or pin change implied.
3. Every integration capability in Section 6.6 maps to an owning specification, with the one uncovered capability marked **Open** rather than implied.
4. Undecided surfaces are enumerated under Open Points and are not presented as decided.
5. Bitty-side records are cited by absolute URL as consumer records without duplicating their content.
6. No existing accepted specification's claims or status is changed; only cross-references are added.
7. `just check` passes with zero issues.
