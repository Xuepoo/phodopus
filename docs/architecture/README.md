---
title: Phodopus Architecture Index
description: Architectural foundation, stackless VM model, and system design
category: architecture
audience: developers
document_type: index
status: accepted
website_publish: true
sidebar_order: 10
---

# Architecture Index

The Architecture topic tree defines the structural foundations of Phodopus: its pure-Rust VM, the `gc-arena` generative lifetime cycle collector, the non-blocking stackless trampoline, and the multi-phase evolution plan.

---

## Documents

| Document | Type | Status | Description |
| :--- | :--- | :--- | :--- |
| [Architecture Overview](overview.md) | Architecture | Accepted | Core VM execution model, stackless sequence execution, and GC arena safety guarantees. |
| [Evolution Roadmap](roadmap.md) | Architecture | Accepted | Multi-phase development roadmap from baseline fork to production sandbox engine. |
| [Garbage Collector Strategy](gc-strategy.md) | Architecture | Accepted | Dependency pinning, memory accounting evolution, and hard quota strategy for `gc-arena`. |
