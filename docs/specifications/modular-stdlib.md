---
title: Modular Standard Library Specification
description: Normative specification for capability-gated standard libraries, Lua patterns, string formatting, and UTF-8
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: complete
website_publish: true
sidebar_order: 22
---

# Modular Standard Library Specification

> Status: Design **accepted** | Implementation: **complete** (baseline stdlib subsets, utf8, string.format, and authentic Lua pattern matching implemented). This document defines the modular architecture, standard library implementations, authentic Lua pattern matching, and Unicode support for Phodopus.

---

## 1. Purpose and Scope

### In Scope

- Modular standard library loading: libraries can be loaded selectively to minimize footprint and attack surface.
- String formatting (`string.format`) specification and supported conversions.
- Authentic Lua pattern matching (`find`, `match`, `gsub`, `gmatch`) behavior.
- String metatable and object-oriented method call syntax (`s:sub()`, `s:upper()`, `s:format()`).
- Standard Lua `utf8` library implementation.
- Separation of standard UTF-8 code points from terminal typography (monospace column width).

### Out of Scope

- Unsandboxed host OS operations (`io.open`, `os.execute`, `os.getenv`).
- External C dynamic library loading (`package.loadlib`).

---

## 2. Normative Sources

- **Lua 5.4 Reference Manual**: Standard semantics for string patterns, formatting verbs, and UTF-8 utilities.
- **Bitty Security Policy**: Buffer allocation during string transformations must respect Fuel and memory limits.

---

## 3. Modular Architecture

Standard libraries in Phodopus are organized as discrete, independently linkable modules:

```text
+--------------------------------------------------------------+
|                        Phodopus Core                         |
+--------------------------------------------------------------+
        |                  |                 |             |
        v                  v                 v             v
 [Core Libs]         [String Lib]       [UTF-8 Lib]   [Math Lib]
 - base (pcall, print) - formatting       - codepoint   - sin/cos/sqrt
 - coroutine         - Lua patterns     - len/offset  - random/floor
 - table             - byte/char/sub    - codes iter  - min/max
```

Embedding applications may instantiate an empty runtime and load only the libraries required:

```rust
let mut lua = Lua::empty();
lua.load_base()?;
lua.load_string()?;
lua.load_table()?;
lua.load_utf8()?;
// Note: io and os remain unmapped
```

---

## 4. Technical Specification

### 4.1 String Formatting (`string.format`)

The `string.format` implementation supports standard Lua formatting specifiers:

- `%c`: Single byte character from integer code point.
- `%d`, `%i`: Signed decimal integer format. Handles `math.mininteger` (`i64::MIN`) safely without overflow panics.
- `%o`, `%u`, `%x`, `%X`: Unsigned octal, decimal, and hexadecimal formats.
- `%f`: Decimal floating point format.
- `%e`, `%E`: Scientific notation floating point formats (lowercase and uppercase).
- `%g`, `%G`: Compact floating point formats (switching between decimal and scientific based on exponent and precision).
- `%a`, `%A`: Hexadecimal floating point formats using the `fhex` crate (`fhex::ToHex`).
- `%s`: String conversion (formats strings, numbers, booleans, nil, and calls `__tostring` on objects when available).
- `%q`: Quoted string safely escaped for Lua syntax deserialization (`\n`, `\r`, `"`, `\\`, `\0`, and control characters).
- `%p`: Pointer representation formatted as hexadecimal address (`0x...`) using the underlying garbage-collected object address.
- `%%`: Escaped literal percent sign.

Flags, width, and precision modifiers:

- `-`: Left-adjust within the given field width.
- `+`: Always show sign (`+` or `-`) for signed numeric conversions.
- `' '` (space): Precede non-negative signed numbers with a space.
- `#`: Alternate form (prefix `0x`/`0X` for hex, force decimal point for floats, retain trailing zeros for `%g`/`%G`).
- `0`: Zero-padding to field width (ignored if `-` flag is present or when precision is specified on integers).

**Safety & Buffering Constraints**:

- Maximum field width is bounded to 1000 characters to prevent memory-bomb attacks (e.g. `%999999999s`).
- Maximum precision is bounded to 1000 digits.
- Specifiers exceeding bounds or containing invalid syntax trigger format errors.

### 4.2 Authentic Lua Pattern Matching

Phodopus rejects mapping `string.match` directly to the Rust `regex` crate, which would introduce divergent semantics (e.g. `\d` instead of `%d`, different quantifier rules, missing frontier patterns).

Instead, Phodopus incorporates an authentic Lua pattern engine (`lsonar` 0.2.4) integrated in `crates/phodopus/src/stdlib/string/patterns.rs`, supporting:

1. **Character Classes**: `%a` (letters), `%c` (control), `%d` (digits), `%l` (lowercase), `%p` (punctuation), `%s` (space), `%u` (uppercase), `%w` (alphanumeric), `%x` (hex), and their uppercase inverse classes (`%A`, `%D`, etc.). Character sets `[a-z]` and inverted sets `[^0-9]` are supported.
2. **Magic Characters**: `^`, `$`, `(`, `)`, `%`, `.`, `[`, `]`, `*`, `+`, `-`, `?`. Literal instances of magic characters must be escaped with `%` (e.g. `%-` for literal hyphen).
3. **Lazy Quantifier**: `-` matches 0 or more characters greedily-minimal.
4. **Frontier Patterns**: `%f[set]` matches empty string transitions from non-set to set.
5. **Captures**: Nested captures, position captures `()`, and replacement string tokens (`%0` for full match, `%1` to `%9` for captures, and `%%` for literal percent).
6. **Functions**:
   - `string.find(s, pattern [, init [, plain]])`: 1-based indexing, negative `init` support (clamping to start on values smaller than `-len`), and `plain` literal byte matching. Returns `start, end, ...captures` on match, or `nil`.
   - `string.match(s, pattern [, init])`: Returns captures if any, whole matched string if no captures, or `nil`.
   - `string.gmatch(s, pattern [, init])`: Stateful iterator callback returning subsequent matches or captures on each invocation. Supports optional `init` starting position.
   - `string.gsub(s, pattern, repl [, n])`: String and table substitutions bounded by optional maximum count `n`. If `repl` is a table, lookups use the first capture (or entire match if no captures); string and number values substitute, whereas `false` or `nil` values retain the original match. Function replacement raises a clean descriptive error.
7. **Error Handling**: Malformed patterns raise standard Lua errors formatted as `malformed pattern (...)` rather than panicking.

### 4.3 Unicode Support (`utf8` Library)

The `utf8` module is implemented in `crates/phodopus/src/stdlib/utf8.rs` and loaded by default via `load_core()` or modularly via `lua.load_utf8()`. It implements standard Lua 5.3/5.4 functions:

- `utf8.char(...)`: Encodes zero or more Unicode code points into a UTF-8 byte string.
- `utf8.charpattern`: Standard Lua pattern matching one UTF-8 byte sequence (`[\0-\x7F\xC2-\xF4][\x80-\xBF]*`).
- `utf8.codepoint(s [, i [, j [, lax]]])`: Returns integer code points from UTF-8 string positions.
- `utf8.len(s [, i [, j [, lax]]])`: Validates UTF-8 encoding and counts code points.
- `utf8.offset(s, n [, i])`: Computes byte offset for the n-th code point.
- `utf8.codes(s [, lax])`: Iterates over pairs of `(byte_position, code_point)`.

**Architectural Invariant**: The standard `utf8` library measures **code points**, not terminal visual cell width. Grapheme clusters, emoji modifiers, and East Asian double-width characters (`unicode-width`, `unicode-segmentation`) belong strictly to the higher-level terminal host ABI (`bitty.text`), preserving strict Lua conformance in Phodopus.

### 4.4 String Metatable and OOP Method Ergonomics

Standard Lua (5.3/5.4) provides object-oriented method call ergonomics on string values (e.g. `s:sub(1, 3)`, `s:upper()`, `s:find("pat")`, `s:format(...)`). In standard Lua semantics, all string instances share a global metatable whose `__index` field defaults to the standard `string` library table.

Phodopus implements string metatable and method dispatch via:

1. **Dedicated VM State Field**: `State<'gc>` maintains a `string_metatable: Table<'gc>` initialized during state construction (`State::new`). The metatable is exposed on `Context<'gc>` through `ctx.string_metatable()`.
2. **Standard Library Wiring**: When `load_string(ctx)` executes, `ctx.string_metatable()` is configured to map `MetaMethod::Index` (`__index`) to the loaded `string` module table (`ctx.string_metatable().set(ctx, MetaMethod::Index, string)`).
3. **VM `__index` Dispatch**: In `meta_ops::index`, when evaluating an index operation on a `Value::String(_)`, the VM queries `ctx.string_metatable()` for `MetaMethod::Index`. If nil (such as in an unmapped minimal VM without string library support), indexing produces a standard `could not index into a string value` error. If present, the lookup resolves either via table indexing (e.g. `string.sub`) or by invoking an index metamethod callback with `[string_value, key]`.
4. **Custom Extensibility**: User scripts or host extensions adding functions to `string` (e.g. `string.custom_fn = ...`) immediately become available via method call syntax (`s:custom_fn(...)`) on all string instances.
5. **Chaining and Stack Hygiene**: Chained method invocations (`s:sub(...):upper():format(...)`) execute with clean register isolation and zero frame leaks across calls.

---

## 5. Security & Verification Plan

1. **Format Bomb Test**: `string.format("%999999999s", "a")` must return a descriptive error, not allocate gigabytes of whitespace.
2. **Pattern Conformance**: Run the full PUC-Rio Lua 5.4 string pattern test suite against Phodopus; assert 100% equivalence.
3. **Malformed UTF-8 Handling**: Assert `utf8.len` returns `nil` and the byte offset of invalid byte sequences without panicking.
4. **String Method Invocation & Metatable Sandboxing**: Assert that string method calls (`s:len()`, `s:sub()`, `s:upper()`, `s:format()`, `s:find()`) correctly dispatch through the string metatable, custom extension functions on `string` propagate to method calls, direct indexing on strings behaves as expected, and missing methods error gracefully.
