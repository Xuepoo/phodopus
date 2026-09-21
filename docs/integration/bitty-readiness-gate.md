---
title: Bitty Readiness Gate
description: Verdict record mapping each required Bitty contract to Phodopus behavior, executable evidence, and an explicit PASS or BLOCKED verdict for the bitty-lua migration decision
category: integration
audience: developers
document_type: specification
status: draft
website_publish: false
sidebar_order: 42
---

# Bitty Readiness Gate

> Status: **draft**. This document records the readiness gate that ends the
> `bitty-lua` deferral named in the [Bitty Host ABI Boundary](bitty-host-abi.md)
> Section 6.5. It is a verdict record, not a migration authorization: the
> migration itself is a separate commander-owned decision (bitty CTX-0590) and
> remains **BLOCKED** until every required item below is PASS. Nothing here
> weakens a normative source, changes an accepted pin, or claims shipped
> behavior beyond what the cited tests prove.

## Purpose and Scope

### In Scope

- One verdict per required Bitty contract: RC-1, RC-2, RC-11, FS-1 through
  FS-9, the restricted standard-library and debug allowlist, structured
  diagnostics, per-plugin module isolation, host-driven cancellation, and
  cross-platform evidence.
- For each item: the Phodopus behavior it maps to, the executable evidence
  that proves it (exact test name plus file), and an explicit
  **PASS** or **BLOCKED** verdict.
- The aggregate migration-block statement: whether the `bitty-lua` runtime
  swap may proceed.

### Out of Scope

- The host-boundary rules themselves; those are owned by the
  [Bitty Host ABI Boundary](bitty-host-abi.md).
- The `bitty-lua` consumer implementation, plugin capability grants, manifest
  grammar, and migration scheduling; those are owned by the Bitty consumer
  repositories.
- Any claim that passing this gate migrates code or changes pins. This gate
  only records evidence; the migration task acts on it.

## Normative Sources This Specification Must Not Weaken

This gate cites the following as normative; if any wording here conflicts
with them, they win:

- **Isolation and Resource RFC** (`bitty-docs`
  `bitty-plugins/specifications/isolation-resource-rfc.md`, accepted
  2026-08-28, closes OQ-014): owns the RC-1, RC-2, RC-11 ceiling definitions
  and the FS-1 through FS-9 failure-semantics definitions. This gate does not
  redefine any ceiling or semantic; it maps each to Phodopus behavior.
- **Lua Runtime RFC** (`bitty-docs`
  `bitty-plugins/specifications/lua-runtime-rfc.md`, accepted 2026-08-27,
  closes OQ-009): owns the sandbox construction, restricted standard-library
  subset, rooted module-resolution rules, and diagnostics contract this gate
  measures against.
- **ADR 0005 — Lua Pins, Upgrade Cadence, Stdlib Allowlist and Unsafe-Surface
  Audit** (`bitty-docs` `docs/decisions/adrs/ADR-0005-lua-pins-and-stdlib.md`,
  accepted 2026-08-29): owns the final restricted stdlib and debug allowlist
  (`debug.traceback` only; `debug.sethook`, `debug.getupvalue`,
  `debug.setupvalue`, `debug.getlocal`, `debug.setlocal`,
  `debug.getregistry`, `debug.upvalueid` denied) and the denied set (`io.*`
  except host handles, `os.execute`/`popen`/spawn, `package.loadlib` and
  native artifacts, filesystem-touching searchers, bytecode `load`/`loadfile`,
  ambient `package.path`/`cpath` mutation, `debug` beyond `traceback`).
- **ADR 0012 — Phodopus Runtime as the Lua Successor Path** (`bitty-docs`
  `docs/decisions/adrs/ADR-0012-phodopus-runtime.md`, accepted 2026-09-20):
  selects Phodopus as the plugin-VM successor direction while keeping the
  accepted `mlua`/vendored Lua 5.4 and `piccolo 0.3.3` pins in force until an
  implementing task migrates `bitty-lua`. Its open points (crate extraction,
  `HostOp::Pending(handle)` review, hard-quota reconciliation with RC-1/RC-2
  and RC-11, migration timing) stay open.
- **Security Overview, Threat Model, P0 Acceptance Criteria** (`bitty-docs`
  `docs/security/overview.md`, `docs/security/threat-model.md`,
  `docs/security/p0-acceptance-criteria.md`, all normative): own the P0
  controls (P0-AC-011 restricted stdlib, P0-AC-012 capability host API,
  P0-AC-013 isolation and containment, P0-AC-014 attributable budgets,
  P0-AC-015 hot-path exclusion, P0-AC-018 native rejection). Phodopus
  supplies only the runtime half of each; host-side enforcement stays with
  the consumer.
- **Plugin Host Runtime RFC** (`bitty-docs`
  `bitty-plugins/specifications/plugin-host-runtime-rfc.md`, accepted and
  shipped per ADR 0010): owns the `bitty-lua` seam contract one VM per
  `(PluginId, generation)`, the RC-1 (`10^7` instructions or 50 ms wall,
  8 ms warning) and RC-2 (32 MiB) reuse on activation and callbacks, the
  synchronous non-blocking `Send` contract (`piccolo` VM is `!Send`,
  confined to one executor thread), and the typed `E_TIMEOUT` (`budget`
  class) proposal.
- **Phodopus Host ABI (Candidate)** (`bitty-terminal-docs`
  `specifications/phodopus-host-abi-candidate.md`, draft candidate, not
  normative): the terminal-side direction record (P-1 boundary, P-2 typed
  pending-handle trampoline, P-3 `utf8.*` versus typography split, P-4
  roadmap and deferral). Cited here only for terminology synchronization;
  it authorizes nothing.

Terminology in this gate follows those sources verbatim: **PASS** means the
Phodopus runtime half has executable evidence cited below; **BLOCKED** means
the item cannot pass on runtime evidence alone (host work, a missing
surface, or an unlanded migration) and the migration stays gated.

## Technical Body

### Gate verdict table

| ID               | Required contract (normative source)                                                                                                                                                                                                                                                                                    | Phodopus behavior mapping                                                                                                                                                                                                                                                                                                                                             | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Verdict                                                                                                                                                                                                                 |
| :--------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| RC-1             | Callback CPU and instruction budget: `10^7` VM instructions or 50 ms wall, 8 ms warning; fail-closed suspend (Isolation Resource RFC RC-1; Plugin Host Runtime RFC A.6 reuses it for activation and callbacks)                                                                                                          | Instruction Fuel accounting per opcode plus proportional variable-cost charging for every stdlib callback (`crates/phodopus/src/stdlib/sandbox.rs` cost model; `docs/specifications/sandbox-and-fuel.md` Section 4.1.1); wall-clock deadlines stay host-owned                                                                                                         | `crates/phodopus/tests/fuel_stdlib.rs::format_is_interrupted_and_resumable` (line 67); `::gsub_is_interrupted_and_resumable` (line 113); `::utf8_len_is_interrupted_and_resumable` (line 157); `::pack_and_unpack_are_interrupted_and_resumable` (line 198); `crates/phodopus/tests/memory_quota.rs::fuel_limit_is_total_and_replenishable` (line 547); executable gate `crates/phodopus/tests/readiness_gate.rs::rc1_fuel_budget_is_enforced_and_recoverable` | **PASS (runtime half)**                                                                                                                                                                                                 |
| RC-2             | Memory per plugin VM: 32 MiB accounted allocations, hard-gated suspend (Isolation Resource RFC RC-2; Plugin Host Runtime RFC A.6)                                                                                                                                                                                       | Hard heap quota via `RuntimeBuilder::memory_limit` (executor-loop chokepoint plus per-site pre-allocation checks; `docs/specifications/sandbox-and-fuel.md` Section 4.2); peak bounded by the ceiling plus one quota-capped single-iteration allocation (measured at most 2x quota at 32 KiB and above through `Lua::execute`)                                        | `crates/phodopus/tests/memory_quota.rs::eight_mib_quota_refuses_large_string_allocation` (line 69); `::quota_refuses_deep_recursion_with_bounded_overshoot` (line 692); `::quota_caps_unpack_sequence_bomb_at_small_quota` (line 792); `::quota_caps_pack_sequence_batch_at_small_quota` (line 831); executable gate `crates/phodopus/tests/readiness_gate.rs::rc2_memory_quota_refuses_and_recovers`                                                          | **PASS (runtime half)**                                                                                                                                                                                                 |
| RC-11            | Plugin persistent store quota: 256 KiB persisted per plugin, at most 8 KiB per value, depth at most 8, at most 1024 nodes; refusal above quota (`E_STORE_QUOTA`), no eviction, no partial write (Isolation Resource RFC RC-11)                                                                                          | No store surface exists in Phodopus: persistence is a `bitty-lua`/host service (`bitty.store`), not a runtime primitive                                                                                                                                                                                                                                               | None; no test possible in this repository                                                                                                                                                                                                                                                                                                                                                                                                                      | **BLOCKED** — host-owned surface; remains with the `bitty-lua` consumer. Follow-up: implement `bitty.store` against RC-11 in the consumer with `E_STORE_QUOTA` typed refusal.                                           |
| FS-1             | Transactional denial: a refused check leaves no partial state and returns a typed error (Isolation Resource RFC FS-1)                                                                                                                                                                                                   | Quota refusals fail before allocation (`Context::check_memory` checked arithmetic); denied `require` names fail pre-resolution (`validate_module_path`); failed loaders leave no sentinel in `package.loaded`                                                                                                                                                         | `crates/phodopus/tests/memory_quota.rs::try_set_field_returns_typed_oom_under_quota` (line 400); `::table_set_under_quota_is_panic_free` (line 422); `crates/phodopus/tests/require.rs::failed_loader_does_not_leave_sentinel_cached` (line 141); executable gate `crates/phodopus/tests/readiness_gate.rs::fs1_denial_leaves_no_partial_state`                                                                                                                | **PASS (runtime half)**                                                                                                                                                                                                 |
| FS-2             | Degradation ladder: refuse, terminate callback, suspend generation, disable plugin; three escalations in a sliding 60-second window suspend; reactivation needs explicit user action (Isolation Resource RFC FS-2)                                                                                                      | No ladder exists in Phodopus: the runtime reports typed errors (`OutOfMemory`, `FuelExhausted`, `HostOpCancelled`) but owns no plugin generation, suspension registry, disable state, or escalation counters                                                                                                                                                          | None; no test possible in this repository                                                                                                                                                                                                                                                                                                                                                                                                                      | **BLOCKED** — host-owned lifecycle; remains with the `bitty-lua`/plugin-host consumer. Follow-up: implement the escalation ladder with attributed strike records in the consumer.                                       |
| FS-3             | Containment: a fault affects only the owning VM or lifecycle; the host survives and siblings are unaffected (Isolation Resource RFC FS-3; P0-AC-013 parity)                                                                                                                                                             | One `Lua` instance per plugin is the unit of containment: a quota or fuel refusal is a typed error inside that instance, never a process abort; `catch_unwind` proves refusal does not unwind the host thread                                                                                                                                                         | `crates/phodopus/tests/memory_quota.rs::constructor_chain_refusal_never_aborts_under_pcall` (line 237); `::quota_refuses_deep_recursion_with_bounded_overshoot` (line 692, `catch_unwind` proves host survival); executable gate `crates/phodopus/tests/readiness_gate.rs::fs3_fault_is_contained_to_owning_vm`                                                                                                                                                | **PASS (runtime half)**                                                                                                                                                                                                 |
| FS-4             | Attribution: every enforcement action emits a structured record (owner id, generation, budget dimension, observed value, limit, action) (Isolation Resource RFC FS-4)                                                                                                                                                   | Runtime errors carry machine-readable fields (`OutOfMemory { requested, limit, current }`, `FuelExhausted { limit }`) but no owner id or generation: attribution needs the host's `(PluginId, generation)`                                                                                                                                                            | Partial: `crates/phodopus/tests/memory_quota.rs::eight_mib_quota_refuses_large_string_allocation` (line 69 asserts `oom.limit`); executable gate `crates/phodopus/tests/readiness_gate.rs::fs4_enforcement_carries_machine_readable_fields`                                                                                                                                                                                                                    | **BLOCKED** — structured owner-attributed records are host-owned; the runtime supplies only the dimension fields. Follow-up: emit `(PluginId, generation, dimension, observed, limit, action)` records in the consumer. |
| FS-5             | Reclaim: after suspension or disable, the owner's memory, tasks, timers, queues, and descriptors are released and verified against the pre-activation baseline (Isolation Resource RFC FS-5)                                                                                                                            | `pcall` recovery keeps the `Lua` instance usable after a refusal; GC-boundary collection reclaims garbage before the runtime gives up                                                                                                                                                                                                                                 | `crates/phodopus/tests/memory_quota.rs::pcall_recovers_from_out_of_memory` (line 464); `::unrooted_closure_allocations_are_gc_bounded` (line 336); executable gate `crates/phodopus/tests/readiness_gate.rs::fs5_recovery_keeps_instance_usable`                                                                                                                                                                                                               | **PASS (runtime half)**                                                                                                                                                                                                 |
| FS-6             | Reload ordering: generation N resources are disposed before generation N+1 activates; a failed reload restores N or disables cleanly (Isolation Resource RFC FS-6)                                                                                                                                                      | No generations exist in Phodopus: VM lifecycle (create, execute, suspend, dispose) is host-driven; the runtime owns no reload transaction                                                                                                                                                                                                                             | None; no test possible in this repository                                                                                                                                                                                                                                                                                                                                                                                                                      | **BLOCKED** — host-owned lifecycle; remains with the `bitty-lua`/plugin-host consumer. Follow-up: implement generation disposal-before-activation with transactional restore in the consumer.                           |
| FS-7             | Fail-closed instrumentation: if a budget cannot be enforced, components requiring it refuse to load rather than run unbounded (Isolation Resource RFC FS-7)                                                                                                                                                             | No fail-closed load gate exists keyed on enforcement health: a host can build a `Lua` without any quota or fuel limit and run unbounded (`execution_without_fuel_limit_is_unbounded` proves the unguarded path works)                                                                                                                                                 | Counter-evidence: `crates/phodopus/tests/memory_quota.rs::execution_without_fuel_limit_is_unbounded` (line 579) demonstrates unbounded execution is possible when the host sets no budget                                                                                                                                                                                                                                                                      | **BLOCKED** — the runtime cannot force the host to set budgets; the consumer must refuse to load plugin VMs without RC-1/RC-2 configured. Follow-up: add a host-side load gate in `bitty-lua`.                          |
| FS-8             | Safe-mode independence: `bitty --safe` starts with minimal built-in configuration and zero third-party plugins (Isolation Resource RFC FS-8; P0-AC-019 parity)                                                                                                                                                          | No safe-mode surface exists in Phodopus: `--safe` is a Bitty application path, not a runtime primitive                                                                                                                                                                                                                                                                | None; no test possible in this repository                                                                                                                                                                                                                                                                                                                                                                                                                      | **BLOCKED** — application-owned path; remains with the Bitty consumer. Follow-up: verify `--safe` against the hostile-fixture set in the consumer.                                                                      |
| FS-9             | No silent weakening: no flag, variable, switch, or temporary API bypasses a ceiling or denial (Isolation Resource RFC FS-9)                                                                                                                                                                                             | No bypass surface exists in the runtime: ceilings are constructor-set (`RuntimeBuilder`), per-site checks use checked arithmetic (`Context::check_memory`), and `unsafe` is forbidden in the stdlib and compiler trees (`scripts/check-unsafe-ledger.sh` gate)                                                                                                        | `crates/phodopus/tests/memory_quota.rs::context_check_memory_uses_checked_arithmetic` (line 615); `just unsafe-ledger` gate (manifest in `docs/security/unsafe-ledger.md` Section 4); executable gate `crates/phodopus/tests/readiness_gate.rs::fs9_no_bypass_surface_exists`                                                                                                                                                                                  | **PASS (runtime half)**                                                                                                                                                                                                 |
| Stdlib allowlist | Restricted standard library and debug allowlist (Lua Runtime RFC accepted subset; ADR 0005 final allowlist): `debug.traceback` only; `io.*`, `os.execute`/`popen`/spawn, `package.loadlib`, native artifacts, filesystem-touching searchers, bytecode `load`/`loadfile`, ambient `package.path`/`cpath` mutation denied | `Lua::core` loads base, coroutine, math, string, table, utf8, `debug.traceback`-only `debug`, and a preload-only `require`; `io`/`os` globals are absent; `load` is text-only (binary mode and bytecode signatures refused, 16 MiB chunk ceiling); `package.path` defaults to empty with no `cpath` and no native loader                                              | `crates/phodopus/tests/require.rs::default_runtime_is_preload_only_and_pathless` (line 40); `crates/phodopus/tests/backtrace.rs::test_backtrace` (line 6) and `::test_pretty_print_backtrace` (line 147) exercise `debug.traceback`; executable gates `crates/phodopus/tests/readiness_gate.rs::stdlib_allowlist_denies_ambient_authority` and `::diagnostics_traceback_and_structured_errors`                                                                 | **PASS**                                                                                                                                                                                                                |
| Diagnostics      | Structured diagnostics: severity, stable error class (`syntax`, `resolution`, `validation`, `runtime`, `budget`), source location, bounded message; errors collected, fail-closed to the previous good state; budget violations abort with the `budget` class (Lua Runtime RFC diagnostics contract)                    | Typed error taxonomy: `Error`/`ExternError` (`Lua` versus `Runtime` variants), `OutOfMemory { requested, limit, current }`, `FuelExhausted { limit }`, `HostOpCancelled { handle, message }`, traversal `access violation` errors naming the module path, missing-module errors listing searcher candidates; backtraces with chunk names, line numbers, and arguments | `crates/phodopus/tests/backtrace.rs::test_backtrace` (line 6, asserts 5 frames with chunk, function, line, args); `crates/phodopus/tests/require.rs::traversal_is_rejected_before_any_searcher_runs` (line 241); `::missing_module_lists_searcher_candidates` (line 332); executable gate `crates/phodopus/tests/readiness_gate.rs::diagnostics_traceback_and_structured_errors`                                                                               | **PASS (runtime half)** — host marshalling to stable `E_*` bridge codes (`BridgeError` in the Plugin Host Runtime RFC) stays consumer-owned.                                                                            |
| Module isolation | Rooted module resolution per VM: each VM resolves `require` only inside its own rooted tree; no cross-tree fallback; traversal out of root is a resolution error; `package.path`/`cpath` mutation ignored; results cached per VM (Lua Runtime RFC search rules)                                                         | Capability-rooted VFS: pre-resolution `validate_module_path` rejects traversal before any searcher runs; searcher chain is preload, embedded, VFS, then custom host searcher; `package.loaded` sentinel-`true` circular semantics; failure messages list per-searcher candidates                                                                                      | `crates/phodopus/tests/require.rs::traversal_is_rejected_before_any_searcher_runs` (line 241); `::traversal_variants_are_all_rejected` (line 293); `::vfs_resolves_dotted_names_and_init_files` (line 161); `::circular_require_terminates_via_sentinel` (line 206); `::require_resolution_is_interruptible_by_fuel` (line 107); executable gate `crates/phodopus/tests/readiness_gate.rs::module_isolation_denies_escape`                                     | **PASS (runtime half)** — per-`(PluginId, generation)` VM ownership and cache clearing on generation disposal stay consumer-owned.                                                                                      |
| Cancellation     | Host-driven cancellation and timeouts: a bounded host operation that times out suspends the plugin slice fail-closed with a `budget`-class error (`E_TIMEOUT` proposal; Plugin Host Runtime RFC A.6); suspended sequences deliver cancellation through `pcall`                                                          | `cancel_host_op` injects a pending `HostOpCancelled` error into the parked sequence so `Sequence::error` runs on the next step and the payload unwinds through Lua `pcall`; unknown handles fail cleanly; collected threads report abandoned handles for host-driven future drops                                                                                     | `crates/phodopus/tests/hostop.rs::host_cancellation_surfaces_as_catchable_pcall_error` (line 258); `::hostop_handle_finalizer_notifies_host_on_collect` (line 466); `::suspend_resume_preserves_fuel_and_quota_accounting` (line 360, unknown-handle refusal); executable gate `crates/phodopus/tests/readiness_gate.rs::cancellation_surfaces_catchable_error`                                                                                                | **PASS (runtime half)** — wall-clock timeout policy and the `E_TIMEOUT` bridge code stay consumer-owned.                                                                                                                |
| Cross-platform   | Tier 1 platform coverage and MSRV: CI builds and tests on Linux, macOS, and Windows; MSRV 1.85 verification (ADR 0005 consequences; Isolation Resource RFC hostile-config and platform tiers)                                                                                                                           | Deterministic Fuel accounting (same bytecode and input consume the same Fuel on every platform; `docs/specifications/sandbox-and-fuel.md` Section 5) plus CI evidence                                                                                                                                                                                                 | `.github/workflows/ci.yml`: `Test (${{ matrix.os }})` job with `os: [ubuntu-latest, macos-latest, windows-latest]` running `cargo test --workspace --all-targets`; `MSRV verification (1.85.0)` job running `cargo +1.85.0 check --workspace --all-targets`; executable gate `crates/phodopus/tests/readiness_gate.rs::cross_platform_contracts_are_deterministic`                                                                                             | **PASS (evidence-linked)** — full per-platform pass records live in CI runs, not in this repository.                                                                                                                    |

### What T5 through T8 actually implemented

The PASS verdicts above rest only on what T5 through T8 shipped; nothing
more is claimed:

- **T5 (CTX-0013)**: proportional Fuel accounting for variable-cost stdlib
  callbacks (`crates/phodopus/src/stdlib/sandbox.rs` cost model; resumable
  `Sequence` batches; 16 MiB checked output ceilings). Evidence:
  `crates/phodopus/tests/fuel_stdlib.rs` (10 tests).
- **T6 (CTX-0017)**: sandboxed `require` with the pluggable searcher chain,
  preloaded core modules, empty default `package.path`, no native loader,
  and capability-constrained VFS roots. Evidence:
  `crates/phodopus/tests/require.rs` (21 tests).
- **T7 (CTX-0018)**: hard allocator memory quota with `OutOfMemory`
  recovery (`RuntimeBuilder::memory_limit`, executor-loop chokepoint plus
  per-site pre-allocation checks; peak bounded by the ceiling plus one
  quota-capped single-iteration allocation, measured at most 2x quota at
  32 KiB and above). Evidence:
  `crates/phodopus/tests/memory_quota.rs` (30 tests).
- **T8 (CTX-0019)**: typed `HostOp` host-async suspension bridge
  (`SequencePoll::Suspend`, `ExecutorMode::HostSuspended`,
  `resume_host_op`/`cancel_host_op`, `HostOpRegistry`,
  `abandoned_handles`, `HostOpGuard`). Evidence:
  `crates/phodopus/tests/hostop.rs` (8 tests).

Supporting evidence (not T5 through T8, cited where used): stack
diagnostics in `crates/phodopus/tests/backtrace.rs` (2 tests) and the
host-responsibility panic boundary in
`crates/phodopus/tests/panic_containment.rs` (2 tests).

### Migration-block statement

**The `bitty-lua` migration (bitty CTX-0590) is BLOCKED.** Six required
items are BLOCKED (RC-11, FS-2, FS-4, FS-6, FS-7, FS-8), and the accepted
pins are unchanged: Bitty still depends on `piccolo = 0.3.3`
(`bitty/crates/bitty-lua/Cargo.toml:17`), and ADR 0012 keeps the accepted
`mlua`/vendored Lua 5.4 and `piccolo 0.3.3` contract in force until an
implementing task migrates `bitty-lua`. The runtime halves that are PASS
must not be read as migration approval.

## Security Review

| Concern                  | Required control                                                                                                                                              | Source                                                                                    |
| :----------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------ | :---------------------------------------------------------------------------------------- |
| Ceiling redefinition     | This gate maps ceilings, never resets them; changing a value needs an RFC revision                                                                            | Isolation Resource RFC ceilings rule                                                      |
| Silent PASS inflation    | Every PASS cites an exact test name plus file and line; BLOCKED items name the owning consumer and follow-up                                                  | This document                                                                             |
| Host-authority confusion | Host-owned items (store, ladder, attribution records, generations, load gates, safe mode, bridge codes) are marked BLOCKED, never implied as runtime behavior | Lua Runtime RFC; Plugin Host Runtime RFC; Isolation Resource RFC FS-2/FS-4/FS-6/FS-7/FS-8 |
| Unsafe-surface drift     | `unsafe` stays forbidden in the stdlib and compiler trees; the ledger gate enforces the inventory                                                             | `docs/security/unsafe-ledger.md`; `scripts/check-unsafe-ledger.sh`                        |
| Panic-boundary confusion | Panic containment stays host responsibility; quota and fuel refusals are typed errors, never panics                                                           | `docs/security/threat-model.md` Section 5                                                 |

This section introduces no new control.

## Verification Plan

1. **Harness gate**: `cargo test -p phodopus --test readiness_gate`
   passes; every gate test fails if its mapped behavior regresses (each
   asserts the same property as its cited T5 through T8 source test).
2. **Source-suite parity**: the cited source suites pass unchanged:
   `fuel_stdlib` (10 tests), `require` (21 tests), `memory_quota`
   (30 tests), `hostop` (8 tests).
3. **Documentation gates**: `just check` passes with zero formatting, lint,
   and link issues; this page resolves every relative link.
4. **Consumer-record integrity**: normative citations use fixed paths and
   sections; no citation invents a definition.
5. **Review**: independent reviewer APPROVE (CTX-0021) before merge.

## Alternatives Considered

| Alternative                                     | Disposition                                                                                                                    |
| :---------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------- |
| Mark host-owned items PASS on design intent     | Rejected. A gate that passes on intent instead of executable evidence would authorize a migration the runtime cannot support.  |
| Redefine RC or FS values for Phodopus           | Rejected. Ceilings and semantics are owned by the Isolation Resource RFC; this gate maps them.                                 |
| Fold the verdict table into `bitty-host-abi.md` | Rejected. That page records the accepted boundary; this page records per-item evidence and verdicts that change as work lands. |
| Migrate `bitty-lua` in this task                | Rejected. Migration is bitty CTX-0590, owned by the Bitty commander after this gate passes.                                    |

## Affected Contracts

- **[Bitty Host ABI Boundary](bitty-host-abi.md)** (Accepted): unchanged;
  its Section 6.5 deferral now points at this gate as the record that ends
  it. This page does not edit that page.
- **[Sandbox & Fuel Specification](../specifications/sandbox-and-fuel.md)**
  (Accepted): unchanged; cited for the Fuel model and quota bound.
- **[Module Resolver Specification](../specifications/module-resolver.md)**
  (Accepted): unchanged; cited for the searcher chain and traversal rule.
- **[Async Trampoline Specification](../specifications/async-trampoline.md)**
  (Accepted): unchanged; cited for the suspension and cancellation
  protocol.
- **[Evolution Roadmap](../architecture/roadmap.md)** (Accepted): unchanged;
  remains the single implementation truth for phase status.
- **`docs/README.md`**: the documentation map gains this page in its
  document index.
- **`docs/integration/README.md`**: the integration index gains this page
  in its document table.

## Acceptance Criteria

1. Every gate item (RC-1, RC-2, RC-11, FS-1 through FS-9, stdlib allowlist,
   diagnostics, module isolation, cancellation, cross-platform) carries an
   explicit PASS or BLOCKED verdict with evidence or rationale.
2. Every PASS cites at least one executable test by exact name, file, and
   line; every BLOCKED names the owning consumer and the follow-up.
3. The migration-block statement is explicit: the `bitty-lua` swap stays
   gated while any required item is BLOCKED.
4. The executable harness (`crates/phodopus/tests/readiness_gate.rs`) passes
   and fails on regression of any PASS-mapped behavior.
5. `just check` passes with zero issues; independent reviewer APPROVE
   (CTX-0021) is recorded before merge.
