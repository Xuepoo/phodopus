---
title: Unsafe Code Ledger
description: Normative inventory of every unsafe Rust site in Phodopus, its soundness invariant, owning module, and exercising test
category: security
audience: developers
document_type: reference
status: accepted
website_publish: true
sidebar_order: 32
---

# Unsafe Code Ledger

> Status: **accepted**. This document is the authoritative inventory of every `unsafe` site in the
> Phodopus workspace. It exists because Phodopus is **not** a zero-`unsafe` crate: the correct
> assurance claim is _audited, justified, and gated_ `unsafe`, not its absence.

---

## 1. Purpose and Scope

Phodopus permits `unsafe` Rust only where a GC or low-level representation invariant cannot be
expressed in safe Rust. Every site is:

1. **Enumerated** here with its file, its invariant, its owning module, and the test that exercises
   it.
2. **Justified in place** with a `SAFETY:` comment at the call site.
3. **Gated** by `scripts/check-unsafe-ledger.sh`, which fails when the source set diverges from the
   machine manifest in [Section 4](#4-machine-manifest), when a site lacks a `SAFETY:` comment, or
   when `unsafe` appears in the stdlib or compiler trees. The module resolver has landed in
   `crates/phodopus/src/stdlib/` and is already inside that forbidden tree
   (`scripts/check-unsafe-ledger.sh:107-120`).

In scope: every `unsafe` token in a Rust source file under `crates/`. Out of scope: safe API
surface, dependency internals (`gc-arena`, `hashbrown`), and build tooling.

---

## 2. Terms

- **Site**: one code occurrence of the `unsafe` keyword, counted after ignoring full-line comments.
  A site may be an `unsafe { }` block, an `unsafe fn` declaration/definition, or an `unsafe impl`.
- **Invariant**: the property the compiler cannot verify that the surrounding code must uphold for
  the site to be sound.
- **Owning module**: the crate module that owns the layout or representation contract.
- **Status `active`**: the site exists in the current tree. **Status `external`**: the site belongs
  to another task's scope and is documented here but not owned by this ledger's review.

---

## 3. Ledger

Line numbers reflect the audited revision and are checked mechanically by count, not by line, so
incidental edits above a site do not fail the gate. Each entry lists the owning module and the test
that exercises the behavior.

### 3.1 `crates/phodopus/src/any.rs` (3 sites) — `any` module

| Line | Site                                         | Invariant                                                                                                                                                     | Exercising test                                                                    | Status |
| :--- | :------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------ | :--------------------------------------------------------------------------------- | :----- |
| 133  | `Gc::cast::<AnyInner<M>>(val)`               | `Value<M, Root<'gc, R>>` is `#[repr(C)]` with `AnyInner<M>` as its first field, so the header cast is valid.                                                  | `any::tests::test_any_value`                                                       | active |
| 174  | `Gc::cast::<Value<M, Root<'gc, R>>>(self.0)` | The runtime `type_id` equals `TypeId::of::<R>()`; `Gc` is invariant in `'gc`; the `Rootable` proxy is used for the `TypeId` so projection cannot be confused. | `any::tests::test_any_value`                                                       | active |
| 189  | `Write::assume(root)`                        | `Gc::write` was called on the containing `Gc` immediately before, so the write barrier is active.                                                             | `userdata::tests` via `UserData::downcast_write`; `user_methods::add_write` (util) | active |

The module-level safety argument for non-`'static` downcasting is documented in full at the top of
`any.rs`; the three sites above depend on it.

### 3.2 `crates/phodopus/src/async_callback.rs` (5 sites) — `async_callback` module, **owned by CTX-0018**

| Line | Site                                                                   | Invariant                                                                                                                                         | Exercising test           | Status   |
| :--- | :--------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------ | :------------------------ | :------- |
| 348  | `self.get_unchecked_mut()`                                             | Structural pinning: no field is moved out.                                                                                                        | `tests/async_sequence.rs` | external |
| 365  | `Pin::new_unchecked(fut).poll(...)`                                    | `fut` is structurally pinned and never moved or re-exposed.                                                                                       | `tests/async_sequence.rs` | external |
| 483  | `mem::transmute::<*mut Shared, *mut Shared<'static, 'static>>`         | Pointer lifetime erasure is reversed before `with` exits via drop guard; only accessed through `visit`, which requires a `for<'gc, 'a>` callback. | `tests/async_sequence.rs` | external |
| 508  | `mem::transmute::<*mut Shared<'static, 'static>, *mut Shared<'_, '_>>` | `visit` only runs inside the `with` callback; pointer is nulled outside.                                                                          | `tests/async_sequence.rs` | external |
| 525  | `Waker::from_raw(NOOP_RAW_WAKER)`                                      | The raw waker's vtable is trivial and its four operations are no-ops.                                                                             | `tests/async_sequence.rs` | external |

### 3.3 `crates/phodopus/src/callback.rs` (7 sites) — `callback` module

| Line | Site                                                | Invariant                                                                                                                                                     | Exercising test                                | Status |
| :--- | :-------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------ | :--------------------------------------------- | :----- |
| 94   | `call: unsafe fn(...)` (field type)                 | The function pointer is only invoked by `Callback::call` with the pointer it was created with; the trampoline casts back to the concrete `HeaderCallback<C>`. | `tests/callback.rs`                            | active |
| 112  | `unsafe impl Collect for HeaderCallback`            | The header holds only a function pointer (no GC data); `C` handles its own tracing.                                                                           | `tests/callback.rs`                            | active |
| 133  | trampoline `ptr as *const HeaderCallback<C>`        | `HeaderCallback<C>` is `#[repr(C)]` with `header` first; `ptr` originated from that exact allocation.                                                         | `tests/callback.rs`                            | active |
| 145  | `Gc::cast::<CallbackInner>(hc)`                     | `CallbackInner` is the `#[repr(C)]` first field of `HeaderCallback`.                                                                                          | `tests/callback.rs`                            | active |
| 226  | `(self.0.call)(...)`                                | Dispatch through the VTable with the originating pointer; lifetimes are reconstructed exactly as erased.                                                      | `tests/callback.rs`                            | active |
| 341  | `unsafe impl Collect for BoxSequence`               | The manual impl forwards tracing through a structurally-pinned `Box`; `gc-arena` has no `Pin<T>` impl.                                                        | `tests/callback.rs`, `tests/async_sequence.rs` | active |
| 366  | `Box::from_raw_in(ptr as *mut dyn Sequence, alloc)` | Pointer and allocator from `into_raw_with_allocator` are reunited immediately; unsizing preserves the concrete type in the fat pointer.                       | `tests/callback.rs`, `tests/async_sequence.rs` | active |

This is the representation-erasure boundary for user callbacks. The `unsafe` is confined to the
runtime's VTable machinery; the user-facing `Callback::from_fn` / `from_fn_with` API is safe and
introduces no `unsafe` into callback bodies.

### 3.4 `crates/phodopus/src/error.rs` (2 sites) — `error` module, **owned by CTX-0019**

| Line | Site                                  | Invariant                                                                              | Exercising test  | Status   |
| :--- | :------------------------------------ | :------------------------------------------------------------------------------------- | :--------------- | :------- |
| 117  | `unsafe impl Send for ExternLuaError` | The stored raw pointers are never dereferenced; they are informational addresses only. | `tests/error.rs` | external |
| 118  | `unsafe impl Sync for ExternLuaError` | Same as `Send`: pointer values are inert.                                              | `tests/error.rs` | external |

### 3.5 `crates/phodopus/src/string.rs` (10 sites) — `string` module

| Line | Site                                              | Invariant                                                                                                                                          | Exercising test                                                            | Status |
| :--- | :------------------------------------------------ | :------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------- | :----- |
| 57   | `Box::from_raw(ptr as *mut [u8])` in `Drop`       | `Buffer::Indirect` is only built from `Box::into_raw`; restored exactly once with the original length.                                             | `string::tests::test_string_header`                                        | active |
| 76   | `Gc::cast::<StringInner>(Gc::new(mc, owned))`     | `Owned` is `#[repr(C)]` with `StringInner` first.                                                                                                  | `string::tests::test_string_header`                                        | active |
| 107  | `Gc::cast::<StringInner>(string)`                 | `InlineString<N>` is `#[repr(C)]` with `StringInner` first.                                                                                        | `string::tests::test_string_inline_capacity_boundaries`                    | active |
| 148  | `slice::from_raw_parts` in `as_bytes`             | Inline data lives at `Layout::new::<StringInner>().extend(layout(len))` offset within the allocation; `len` matches the constructed inline length. | `string::tests::test_string_inline_capacity_boundaries`                    | active |
| 239  | `unsafe impl Collect for InternedDynStringsInner` | Exclusive arena access during tracing; only weak pointers are upgraded/erased; no new `Gc` adopted.                                                | `string::tests::test_string_header` (interner paths via integration tests) | active |
| 245  | `unlock_unchecked().borrow_mut()` (trace)         | Arena tracing holds exclusive access; write barrier not required while tracing.                                                                    | `string::tests::test_string_header`                                        | active |
| 248  | `dyn_strings.erase(bucket)` (trace)               | Erasing the currently yielded bucket is permitted; the `RawTable` outlives the iterator.                                                           | `string::tests::test_string_header`                                        | active |
| 274  | `unlock_unchecked().borrow_mut()` (intern)        | Interning runs inside arena mutation (exclusive access); write barrier invoked before mutation.                                                    | `string::tests::test_string_header`                                        | active |
| 278  | `dyn_strings.erase(bucket)` (intern)              | As above; `RawTable` outlives the iterator.                                                                                                        | `string::tests::test_string_header`                                        | active |
| 339  | `unlock_unchecked().borrow_mut()` (static intern) | Interning runs inside arena mutation; write barrier invoked before mutation.                                                                       | `string::tests::test_string_header`                                        | active |

### 3.6 `crates/phodopus/src/table/raw.rs` (3 sites) — `table::raw` module

| Line | Site                                       | Invariant                                                                                              | Exercising test                                       | Status |
| :--- | :----------------------------------------- | :----------------------------------------------------------------------------------------------------- | :---------------------------------------------------- | :----- |
| 157  | `bucket.as_mut()`                          | The bucket was yielded by a mutably borrowed `RawTable`; no other reference can coexist.               | `tests/table.rs::remove_during_iteration_then_refill` | active |
| 360  | bucket scan in `next`                      | `raw_table` is borrowed for the whole method; no mutation during iteration; bucket indices stay valid. | `tests/table.rs::test_table_iter`                     | active |
| 389  | bucket scan after `bucket_index` in `next` | Same borrow; `bucket` was found in the same `raw_table`.                                               | `tests/table.rs::test_table_iter`                     | active |

### 3.7 `crates/phodopus/src/hostop.rs` (2 sites) — `hostop` module, **owned by CTX-0019**

| Line | Site                                   | Invariant                                                                                                           | Exercising test                                                     | Status |
| :--- | :------------------------------------- | :------------------------------------------------------------------------------------------------------------------ | :------------------------------------------------------------------ | :----- |
| 86   | `unsafe impl Collect for HostOpHandle` | `HostOpHandle` is a plain `u64` with no GC pointers; `needs_trace()` is `false`, so the manual impl traces nothing. | `tests/hostop.rs` (all bridge tests)                                | active |
| 175  | `unsafe impl Collect for HostOpGuard`  | The guard holds only a `u64` handle and a `'static` Rust hook (no GC pointers); `needs_trace()` is `false`.         | `tests/hostop.rs::hostop_handle_finalizer_notifies_host_on_collect` | active |

The manual impls exist because the types deliberately opt out of derived tracing: neither type owns
GC-managed data by construction (the GC-isolation contract — only the numeric handle crosses into
host memory). A derived `Collect` would be equally sound here; the manual impls document the
no-trace invariant explicitly.

### 3.8 `crates/phodopus/tests/hostop.rs` (2 sites) — host-async bridge test scaffolding, **owned by CTX-0019**

| Line | Site                                             | Invariant                                                                                                                                                                 | Exercising test                                       | Status |
| :--- | :----------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | :---------------------------------------------------- | :----- |
| 121  | `&mut *this.host.get()` (`*mut MockHost` deref)  | The pointer targets the test's stack-owned `MockHost`, which outlives the `Lua` instance; the `Rc<Cell<*mut>>` plumbing is test-only (real hosts own their future table). | `tests/hostop.rs` (manual-`Sequence` bridge tests)    | active |
| 642  | `&mut *host_handle.get()` (`*mut AsyncMockHost`) | Same contract as the line-121 site: the pointer targets the test's stack-owned `AsyncMockHost`, which outlives the `Lua` instance; test-only scaffolding.                 | `tests/hostop.rs::async_sequence_suspend_*` (2 tests) | active |

### 3.9 `crates/phodopus/src/table/table.rs` (0 sites)

The `raw.rs` submodule owns the raw-slot access; `table.rs` contains no code `unsafe`. The word
`unsafe` appears only in prose in a doc comment (line 145) and is ignored by the gate.

### 3.10 `crates/phodopus-util/src/freeze.rs` (11 sites) — `freeze` module

| Line | Site                                          | Invariant                                                                             | Exercising test                                                      | Status |
| :--- | :-------------------------------------------- | :------------------------------------------------------------------------------------ | :------------------------------------------------------------------- | :----- |
| 174  | `DropGuard::new(self)`                        | RAII guard pairs `ScopeGuard::set` with `Drop`-driven `unset` before `scope` returns. | `freeze::tests::test_freeze_works`                                   | active |
| 190  | `unsafe fn set` for `FreezeGuard`             | Transmutes `'f` to `'static`; reversed by `unset`; `assert!` rejects double-set.      | `freeze::tests::nested_scope_on_same_handle_panics`                  | active |
| 195  | `mem::transmute` `'f` -> `'static`            | Erases the only non-`'static` lifetime; restored before the borrowed value expires.   | `freeze::tests::test_freeze_works`, `handle_is_reusable_after_scope` | active |
| 212  | `mem::transmute` `'static` -> `'f` in `unset` | Reinstates the original lifetime; `try_borrow_mut` proves no live reference.          | `freeze::tests::test_freeze_expires`                                 | active |
| 287  | `DropGuard::new(&mut self.0)`                 | Same RAII pairing as line 174 for `FrozenScope`.                                      | `freeze::tests::scope_works`                                         | active |
| 306  | `unsafe fn set` trait declaration             | Contract: callers must `unset` before the value's lifetime ends.                      | n/a — trait contract                                                 | active |
| 326  | `unsafe fn set` for `()`                      | Unit guard holds no value; no-op.                                                     | `freeze::tests::scope_works`                                         | active |
| 334  | `unsafe fn set` for `(A, B)`                  | Forwards `set` to components; tuple `unset` reverses in order.                        | `freeze::tests::scope_works`                                         | active |
| 337  | both component `set` calls                    | Same preconditions as the tuple contract; tuple is not shared.                        | `freeze::tests::scope_works`                                         | active |
| 355  | `unsafe fn new` for `DropGuard`               | Caller accepts the `ScopeGuard::set` contract; `Drop` unsets before the borrow ends.  | `freeze::tests::test_freeze_works`                                   | active |
| 358  | forwarded `s.set()`                           | The caller's `set` preconditions are forwarded unchanged.                             | `freeze::tests::test_freeze_works`                                   | active |

`phodopus-util` is a host utility crate (`user_methods`, `freeze`); `freeze.rs` is the only unsafe
user in it.

---

## 4. Machine Manifest

`scripts/check-unsafe-ledger.sh` parses the block below. Each line is `<path> <count>` where
`count` is the number of code `unsafe` occurrences (full-line comments excluded) in that file.

<!-- unsafe-ledger:manifest:start -->

crates/phodopus/src/any.rs 3
crates/phodopus/src/async_callback.rs 5
crates/phodopus/src/callback.rs 7
crates/phodopus/src/error.rs 2
crates/phodopus/src/hostop.rs 2
crates/phodopus/src/string.rs 10
crates/phodopus/src/table/raw.rs 3
crates/phodopus/src/table/table.rs 0
crates/phodopus-util/src/freeze.rs 11
crates/phodopus/tests/hostop.rs 2
<!-- unsafe-ledger:manifest:end -->

The gate additionally forbids any `unsafe` in `crates/phodopus/src/stdlib/` and
`crates/phodopus/src/compiler/`, and requires a preceding `SAFETY:` comment within 12 lines of each
site.

### 4.1 Wiring Status

The gate is self-runnable as `scripts/check-unsafe-ledger.sh`. Wiring it into CI is owned by
CTX-0014 (`.github/workflows/` is out of this task's scope). The intended CI step, to be added by
CTX-0014, is:

```bash
scripts/check-unsafe-ledger.sh
```

run after checkout in the quality-gate job. Until then, run it manually and in review.

### 4.2 Excluded Sites and Ownership

`async_callback.rs` and `error.rs` are documented here for completeness but are owned by CTX-0018
and CTX-0019 respectively; this ledger does not edit them. Their counts are part of the manifest so
the gate catches unreviewed changes in either file.

---

## 5. Verification Plan

1. `scripts/check-unsafe-ledger.sh` must exit 0.
2. `CARGO_TARGET_DIR=<isolated> just check` must exit 0.
3. `rg -n 'catch_unwind' crates/phodopus/src crates/phodopus-util/src` must return no matches,
   consistent with the host-responsibility panic policy in `threat-model.md`. The only repository
   occurrence is in `crates/phodopus/tests/panic_containment.rs`, which stands in for the host.
4. Reviewers rerun (1)–(3) and compare the manifest against the source.

---

## 6. References

- [Threat Model & Trust Boundaries](threat-model.md) — panic policy and trust boundaries.
- [`gc-arena`](https://github.com/kyren/gc-arena) — the GC primitives underlying several invariants.
- `scripts/check-unsafe-ledger.sh` — the enforcing gate.
