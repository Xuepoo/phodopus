---
title: Phodopus Integration Index
description: Host-consumer boundary records for external projects that embed Phodopus
category: integration
audience: developers
document_type: index
status: accepted
website_publish: true
sidebar_order: 40
---

# Integration Index

The Integration topic tree records how external host projects consume Phodopus as a dependency. It sits deliberately outside the `architecture/` and `specifications/` trees: those define the generic, host-agnostic runtime and its contracts, while this tree defines the boundary an embedding host must respect and the capabilities it depends on. Documents here never add host-specific behavior to the runtime core.

---

## Documents

| Document                                        | Type          | Status   | Scope                                                                                                                                                                                                                      |
| :---------------------------------------------- | :------------ | :------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Bitty Host ABI Boundary](bitty-host-abi.md)    | Specification | Accepted | Bitty's dependency relationship, the `bitty-lua` Host ABI boundary, async trampoline crossing, text-layout separation, integration deferral, and the capability expectations mapped to this corpus.                        |
| [Bitty Readiness Gate](bitty-readiness-gate.md) | Specification | Draft    | Per-item PASS/BLOCKED verdicts mapping RC-1/RC-2/RC-11, FS-1..FS-9, stdlib allowlists, diagnostics, module isolation, cancellation, and cross-platform evidence to T5-T8 outputs; the `bitty-lua` migration stays BLOCKED. |
