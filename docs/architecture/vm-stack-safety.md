---
title: VM Stack Safety & Frame Invariants
description: Thread stack architecture, parameter alignment, upstream Issue #145 analysis, and zero-cost stack frame isolation guarantees
category: architecture
audience: developers
document_type: architecture
status: accepted
website_publish: true
sidebar_order: 14
---

# VM Stack Safety & Frame Invariants

> Status: **accepted**. This document establishes the stack layout, frame boundary mechanics, parameter alignment protocols, and security invariants for execution in Phodopus.

---

## 1. System Vision & Context

In a register-based virtual machine, execution frames are mapped onto a linear register stack. Unlike native C-stack Lua runtimes (which rely on the machine stack and compiler calling conventions), Phodopus maintains an explicit, flat `Vec<Value<'gc>, MetricsAlloc<'gc>>` stack per thread managed alongside a lightweight frame descriptor stack (`Vec<Frame<'gc>>`).

When functions execute, return, or tail-call each other, registers allocated to an activation record are repeatedly recycled to avoid continuous heap reallocations. Without rigorous stack hygiene, newly activated function frames risk inheriting stale, uninitialized, or dirty data previously written into those physical slots by earlier function calls.

### 1.1 Upstream Issue #145 Context

Upstream Piccolo Issue #145 (*"Unprovided function parameters inherit stale stack values instead of nil"*) highlighted a critical safety and semantic vulnerability:

1. **Semantic Violation**: In standard Lua (PUC-Rio Lua 5.4), unprovided function parameters and uninitialized local variables must evaluate strictly to `nil`. The universal Lua idiom:

   ```lua
   param = param or default_value
   ```

   relies unconditionally on `param` being `nil` (or falsy) when omitted by the caller. If an unprovided parameter inherits a truthy value (such as a string, integer, or table) left on the stack by a preceding function, the default value is bypassed, leading to subtle runtime bugs.

2. **Security & Sandbox Breach**: In a sandboxed plugin or multi-tenant desktop runtime (such as Bitty Terminal), scripts written by untrusted authors execute within shared or constrained Lua environments. If unprovided parameters or uninitialized registers reveal dirty stack slots from prior calls, untrusted scripts could read private table references, internal object handles, sensitive strings, or forge execution states across call boundaries.

Phodopus formalizes and proves stack frame safety through a unified `push_call` architecture, explicit frame boundary tracking, and zero-cost parameter nil-fill.

---

## 2. Component Architecture & Structural Diagrams

Thread state in Phodopus is encapsulated in `ThreadState<'gc>`:

```rust
pub(crate) struct ThreadState<'gc> {
    pub(crate) frames: vec::Vec<Frame<'gc>, MetricsAlloc<'gc>>,
    pub(crate) stack: vec::Vec<Value<'gc>, MetricsAlloc<'gc>>,
    pub(crate) open_upvalues: vec::Vec<UpValue<'gc>, MetricsAlloc<'gc>>,
}
```

Execution frames are represented by the `Frame<'gc>` enumeration:

```rust
pub(crate) enum Frame<'gc> {
    Lua {
        bottom: usize,
        closure: Closure<'gc>,
        base: usize,
        is_variable: bool,
        pc: usize,
        stack_size: usize,
        expected_return: Option<LuaReturn>,
    },
    Callback {
        bottom: usize,
        callback: Callback<'gc>,
    },
    Sequence {
        bottom: usize,
        sequence: Sequence<'gc>,
        pending_error: Option<Error<'gc>>,
    },
    Result {
        bottom: usize,
    },
    Error(Error<'gc>),
}
```

### 2.1 Stack Frame Anatomical Model

For any active Lua closure frame (`Frame::Lua`), the flat stack is divided into distinct regions:

```text
+-----------------------------------------------------------------------------------------+
|                                    Linear Thread Stack                                  |
+-----------------------------------------------------------------------------------------+
| ... Upper Frames ... | Varargs Storage |             Lua Local Registers                |
|                      |  [bottom..base) |  [base..base + fixed) | [base+fixed..base+size)|
+-----------------------------------------------------------------------------------------+
                       ^                 ^                       ^                        ^
                     bottom            base              base + fixed_params        base + stack_size
```

- **`bottom`**: The base stack index where the call was initiated. Arguments passed by the caller are originally positioned starting at `bottom`.
- **`base`**: The stack index where Register 0 (`R0`) of the function's local activation frame begins.
- **`stack_size`**: The total register allocation required by the function prototype (`proto.stack_size`), accommodating all fixed parameters, local variables, and temporaries.
- **`fixed_params`**: The number of declared fixed parameters (`proto.fixed_params`).

### 2.2 Parameter Alignment & Vararg Rotation

When a Lua function is invoked, the number of provided arguments (`given_params = stack.len() - bottom`) may not match `fixed_params`:

#### Case A: Under-Application (`given_params <= fixed_params`)

When fewer arguments are provided than declared fixed parameters:

```text
Given arguments: N
Fixed parameters: F (where N <= F)

+------------------------------------------------------------------------------+
| ... Upper Frames ... | Passed Args [0..N) | Unprovided [N..F) | Locals [F..S)|
+------------------------------------------------------------------------------+
                       ^                                        ^              ^
                    bottom = base                      base + fixed_params   base + stack_size
                       |-------- fixed_params = F --------------|
```

1. `var_params = 0`: No variable arguments exist.
2. `rotate_right(0)`: No rotation is needed.
3. `base = bottom`: Register 0 begins immediately at `bottom`.
4. `resize(base + stack_size, Value::Nil)`: Slots from `base + given_params` up to `base + stack_size` are filled with `Value::Nil`. All unprovided parameters in `[N..F)` evaluate strictly to `nil`.

#### Case B: Over-Application & Varargs (`given_params > fixed_params`)

When more arguments are provided than declared fixed parameters, the surplus arguments represent variable arguments (`...`):

```text
Given arguments: G
Fixed parameters: F (where G > F, var_params = G - F)

Before rotate_right(var_params):
+-------------------------------------------------------------+
| ... Upper Frames ... | Fixed Args [0..F) | Varargs [F..G)   |
+-------------------------------------------------------------+
                       ^
                     bottom

After rotate_right(var_params):
+------------------------------------------------------------------------------+
| ... Upper Frames ... | Varargs [0..V) | Fixed Args [0..F) | Locals [F..S)    |
+------------------------------------------------------------------------------+
                       ^                ^                   ^                  ^
                     bottom           base            base + fixed_params    base + stack_size
                       |<-- var_params >|
```

1. `var_params = given_params - fixed_params`.
2. `stack[bottom..].rotate_right(var_params)`: In-place right-rotation shifts the trailing `var_params` to the beginning of the frame (`[bottom..bottom + var_params]`).
3. `base = bottom + var_params`: Register 0 begins after the vararg window. The fixed parameters are cleanly positioned at `[base..base + fixed_params]`.
4. `resize(base + stack_size, Value::Nil)`: Slots above the fixed parameters up to `base + stack_size` are cleared to `Value::Nil`.

---

## 3. Invariants & Guarantees

Phodopus establishes and enforces five strict architectural invariants governing stack frames:

### Invariant 1: Deterministic Nil-Fill for Unprovided Parameters

> *For any function call with $N$ passed arguments and $F$ declared fixed parameters where $N < F$, all parameter registers $R_i$ with $N \le i < F$ evaluate strictly to `Value::Nil` upon frame entry.*

This invariant guarantees that parameter defaults and conditional initialization behave predictably without stale residue.

### Invariant 2: Zero-Cost Frame Cleanliness

> *All local and temporary registers $R_j$ with $F \le j < S$ (where $S$ is `stack_size`) are guaranteed to evaluate to `Value::Nil` upon initial frame activation and upon return from callee functions.*

Uninitialized local variables declared in Lua blocks always observe `nil`, eliminating uninitialized register bugs across all execution paths.

### Invariant 3: Strict Cross-Frame Memory Isolation

> *No activation frame can read, observe, or mutate values from preceding, sibling, or deeper frames that have returned or been popped, except through explicitly passed arguments, captured upvalues, or returned values.*

Stack truncation and resizing prevent untrusted guest code from inspecting garbage memory or internal host references.

### Invariant 4: Vararg Boundary Isolation

> *When $N \le F$, the vararg slice `[bottom..base)` has length exactly 0. `select('#', ...)` evaluates to `0`, and `{ ... }` produces an empty table.*

Vararg operations cannot alias preceding caller registers or fixed parameters.

### Invariant 5: Safe Upvalue Closing Prior to Stack Truncation

> *Whenever a frame returns (`return_upper`) or tail-calls (`tail_call_function`), all open upvalues pointing into stack slots $\ge bottom$ are closed into heap-allocated upvalues via `close_upvalues` before the stack is truncated or overwritten.*

This prevents dangling stack pointers in closures that outlive their declaring scope.

---

## 4. Sequence & Execution Flows

Stack safety is upheld uniformly across all function call paradigms in Phodopus through a unified entry point: `ThreadState::push_call`.

```text
  +-----------------------------------------------------------------------+
  |                             Call Triggers                             |
  |  - Regular Call:        lua_frame.call_function(...)                  |
  |  - Tail Call:           lua_frame.tail_call_function(...)             |
  |  - Non-destructive Call: lua_frame.call_function_keep(...)            |
  |  - Metamethod Call:     lua_frame.call_meta_function(...)             |
  |  - Sequence Call:       SequencePoll::Call / SequencePoll::TailCall   |
  |  - Callback Call:       CallbackReturn::Call                          |
  +-----------------------------------------------------------------------+
                                      |
                                      v
  +-----------------------------------------------------------------------+
  |              ThreadState::push_call(bottom, function)                 |
  |                                                                       |
  |  1. Inspect Prototype: fixed_params, stack_size                       |
  |  2. Compute given_params = stack.len() - bottom                       |
  |  3. Compute var_params = max(0, given_params - fixed_params)          |
  |  4. In-place rotate: stack[bottom..].rotate_right(var_params)         |
  |  5. Compute base = bottom + var_params                                |
  |  6. Zero-cost Nil Fill: stack.resize(base + stack_size, Value::Nil)   |
  |  7. Push Frame::Lua onto frames stack                                 |
  +-----------------------------------------------------------------------+
```

### 4.1 Regular Call Flow (`call_function`)

In `lua_frame.call_function(ctx, func, args, returns)`:

1. `function_index = base + func.0`.
2. Stack is truncated to `function_index + arg_count` after removing the function object.
3. `push_call(function_index, call)` initializes the callee frame at `bottom = function_index`.
4. On return, `return_to` copies results starting at `function_index` and resizes the caller frame back to `caller_base + caller_stack_size` with `Value::Nil`.

### 4.2 Tail Call Flow (`tail_call_function`)

In `lua_frame.tail_call_function(ctx, func, args)`:

1. Open upvalues for the current frame are closed: `self.state.close_upvalues(&ctx, bottom)`.
2. The current frame is popped: `self.state.frames.pop()`.
3. Arguments are shifted down in-place: `stack.copy_within(function_index + 1..function_index + 1 + arg_count, bottom)`.
4. Stack is truncated: `stack.truncate(bottom + arg_count)`.
5. `push_call(bottom, call)` replaces the frame in-place, reusing the stack space with zero stack growth.

### 4.3 Non-Destructive Iterator Call Flow (`call_function_keep`)

Used by `Operation::GenericForCall` during generic `for ... in` loops:

1. Iterator state and variables remain intact in caller registers.
2. Arguments are copied to `top = function_index + 1 + arg_count`.
3. `push_call(top, call)` establishes the iterator invocation without destroying caller state.
4. On return, results are written after the iterator arguments, preserving loop invariants.

### 4.4 Metamethod Call Flow (`call_meta_function`)

Triggered by arithmetic, bitwise, indexing, or comparison operators when operands specify custom metatables:

1. Caller frame registers are left intact.
2. Metamethod arguments are appended at `top = stack.len()`.
3. `push_call(top, func)` executes the metamethod.
4. On completion, `return_to` extracts the single result, truncates `stack` back to `bottom`, and restores caller registers via `resize(caller_base + caller_stack_size, Value::Nil)`.

---

## 5. Cross-Cutting Concerns

### 5.1 Root Cause of Upstream Issue #145

In upstream Piccolo, activation frame initialization occasionally omitted explicit nil-filling when reusing stack capacity across sequential calls at the same stack depth. If Function A used 10 registers and populated registers $R_0 \dots R_9$ with numbers and tables, and then returned to a caller which immediately invoked Function B (declaring 4 parameters but passing only 1), Function B's unprovided parameter registers $R_1, R_2, R_3$ occupied the exact physical stack slots previously used by Function A.

Because the stack vector's length was simply extended or indexed without overwriting existing elements with `Value::Nil`, Function B observed the stale values left behind by Function A.

Phodopus guarantees this defect cannot occur by:

1. **Mandatory Stack Truncation**: Every call path explicitly truncates the stack to `bottom + given_params` prior to `push_call`.
2. **Unified `resize(base + stack_size, Value::Nil)`**: The standard library `Vec::resize` method fills all newly exposed slots with `Value::Nil`.
3. **Safe Return Normalization**: When returning to a caller frame (`return_to`), the caller frame is similarly normalized to its required `base + stack_size` with `Value::Nil`.

### 5.2 Memory Allocation & Amortized Performance

The `Vec::resize(new_len, Value::Nil)` pattern operates with optimal performance:

- **No Heap Reallocations**: When re-entering a previously allocated stack depth, `Vec::capacity()` is already sufficient. `resize` performs a fast, contiguous write of `Value::Nil` tags (a 16-byte tagged union in 64-bit builds).
- **GC Metrics Accounting**: Because the stack vector is allocated using `MetricsAlloc<'gc>`, all capacity changes are accurately accounted for in the GC metrics.

### 5.3 Security & Sandbox Integrity

Stack frame safety directly bolsters Phodopus's sandbox boundaries:

1. **Leak Prevention**: Host callbacks returning sensitive tokens or internal userData references do not risk leaking those references to untrusted guest code executed subsequently.
2. **Deterministic Preemption**: Stack frame structures are strictly validated across Fuel preemption interrupts and coroutine yields (`coroutine.yield` / `coroutine.resume`).
3. **Pcall Isolation**: Calls wrapped in `pcall` properly unwind and normalize frame registers even when trapped errors abort execution mid-frame.

---

## 6. Verification & Test Evidence

The stack frame safety architecture is comprehensively validated by the Phodopus regression test suite:

- **`tests/scripts/dirty_stack.lua`**:
  - Tests dirty stack inheritance across standard calls, tail calls, generic for loops (`call_function_keep`), and metamethod dispatches (`call_meta_function`).
  - Verifies the Lua default parameter idiom `param = param or default` under repeated dirty-stack cycles.
  - Verifies mixed fixed parameters and varargs (`function f(a, b, c, ...)` with $0, 1, 2, 3, 4, 6$ arguments).
  - Tests coroutine resumption with omitted parameters and deep recursion across alternating argument counts.
  - Tests protected calls (`pcall`) with under-applied parameter lists.
- **`crates/phodopus/tests/scripts.rs`**:
  - Runs all `.lua` regression scripts, ensuring 100% test pass on standard test targets.
