---
title: Toolchain Policy
description: Compiler toolchain pins, MSRV, Clippy lint policy, and quality gates for Phodopus
category: development
audience: developers
document_type: policy
status: accepted
website_publish: true
sidebar_order: 32
---

# Toolchain Policy

> Status: **accepted**. This document defines the normative compiler versions, MSRV standards, linting thresholds, and verification gates for the Phodopus repository.

---

## 1. Toolchain Pins

| Tool               | Pinned Version | Declaration Location                                  |
| :----------------- | :------------- | :---------------------------------------------------- |
| **Rust Toolchain** | `1.98.1`       | `rust-toolchain.toml`                                 |
| **MSRV**           | `1.85.0`       | `Cargo.toml` (`rust-version`), `clippy.toml` (`msrv`) |
| **Rust Edition**   | `2024`         | `Cargo.toml` (`[workspace.package]`)                  |
| **Actionlint**     | `1.7.12`       | GitHub Actions CI / local binary                      |

- **No Unpinned CI**: CI workflows strictly install the pinned `1.98.1` channel with minimal profile components (`rustfmt`, `clippy`).
- **Edition 2024**: The workspace targets Rust edition 2024, declared once in `Cargo.toml` under `[workspace.package]` and inherited by every member crate (`edition.workspace = true`).
- **MSRV 1.85**: The minimum supported Rust version is declared as `rust-version = "1.85"` in `[workspace.package]` and inherited by every member crate; `clippy.toml` mirrors it as `msrv = "1.85"`.
- **Nightly Policy**: The Phodopus core crate does not use unstable nightly compiler flags. All features build cleanly on stable Rust.

---

## 2. Quality Gates (`justfile`)

All local and automated validations are driven through `just`:

```bash
# Run all core quality gates
just check

# Run individual verification steps
just fmt-check      # Verify rustfmt formatting
just typecheck      # cargo check across all targets
just clippy         # Run clippy analysis
just test           # Run unit, integration, and doc-tests
just actionlint     # Validate GitHub Actions workflow syntax
```

---

## 3. Dependency Governance

- **Zero C Dependencies**: The core VM and stdlib must remain pure Rust. No C libraries, native compilers (gcc/clang), or CMake builds may be introduced into the dependency tree.
- **Minimal Core Footprint**: Any new dependency must be evaluated against cold-start time and binary size. Crates with heavy procedural macros or complex runtime runtimes (e.g. Tokio) are barred from the core VM.
- **Cargo Deny & Audit**: Supply-chain dependencies are continuously checked for vulnerabilities, unmaintained crates, and license compliance (MIT / Apache-2.0 / CC0 compatible).
