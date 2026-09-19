---
title: Phodopus Specifications Index
description: Concrete technical specifications and normative runtime contracts
category: specifications
audience: developers
document_type: index
status: accepted
website_publish: true
sidebar_order: 20
---

# Specifications Index

The Specifications topic tree provides the normative technical contracts governing Phodopus features: sandboxing and Fuel, standard library behavior, module loading, and asynchronous execution.

---

## Documents

| Document | Type | Status | Scope |
| :--- | :--- | :--- | :--- |
| [Sandbox & Fuel Specification](sandbox-and-fuel.md) | Specification | Accepted | Deterministic instruction budgeting, preemption thresholds, and hard heap allocation ceilings. |
| [Modular Standard Library Specification](modular-stdlib.md) | Specification | Accepted | Decoupled standard libraries: string formatting, authentic Lua patterns, UTF-8, tables, and math. |
| [Module Resolver Specification](module-resolver.md) | Specification | Accepted | Sandboxed module resolution (`require`), searcher chains, and virtual filesystem providers. |
| [Async Trampoline Specification](async-trampoline.md) | Specification | Accepted | Host-agnostic async suspension protocol (`HostOp::Pending`) and coroutine resumption trampolines. |
