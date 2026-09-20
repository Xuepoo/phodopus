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
- Safe string repetition (`string.rep`) with memory sandbox ceilings.
- Binary packing and unpacking (`string.pack`, `string.unpack`, `string.packsize`) with standard format specifiers, checked alignment, endianness, and memory sandbox limits.
- Standard Lua `utf8` library implementation.
- Separation of standard UTF-8 code points from terminal typography (monospace column width).
- Sandboxed dynamic code loading (`load`) enforcing text-only compilation, custom environment isolation, fuel consumption, and memory allocation bounds.
- Global environment registration (`_G`).

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
- `string.format` is implemented as a resumable
  `Sequence` (`FormatSequence`): the format string is parsed into owned
  elements once, each `poll` expands directives until the remaining Fuel runs
  out, then returns `SequencePoll::Pending` with the partial output preserved.
  Verbatim runs are emitted in Fuel-sized chunks at UTF-8 boundaries. Each
  conversion directive costs `4` Fuel plus `1` per output byte.
- Total output is capped by the checked `MAX_STDLIB_STRING_BYTES` (16 MiB);
  exceeding it raises `"resulting string too large"`.

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

8. **Fuel & Resumption Cost Model**:
   - `string.gsub` is implemented as a resumable `Sequence` (`GsubSequence`). Each `poll` performs as many search/replace iterations as the remaining Fuel allows, then returns `SequencePoll::Pending` with the output buffer, cursor, replacement count, and replacement mode preserved. Output is capped by the checked `MAX_STDLIB_STRING_BYTES` (16 MiB).
   - Each pattern search charges `16` Fuel per candidate start position in the remaining window (`remaining_bytes + 1` attempts), plus `1` Fuel per output byte appended.
   - `string.find`, `string.match`, and each `string.gmatch` iteration charge the same attempt bound for their single unanchored search. The `lsonar` engine call itself is bounded by `MAX_RECURSION_DEPTH` (500) and the input window and is not preemptible below one call; this residual bound is documented in `sandbox-and-fuel.md` §4.1.3.

### 4.3 Unicode Support (`utf8` Library)

The `utf8` module is implemented in `crates/phodopus/src/stdlib/utf8.rs` and loaded by default via `load_core()` or modularly via `lua.load_utf8()`. It implements standard Lua 5.3/5.4 functions:

- `utf8.char(...)`: Encodes zero or more Unicode code points into a UTF-8 byte string.
- `utf8.charpattern`: Standard Lua pattern matching one UTF-8 byte sequence (`[\0-\x7F\xC2-\xF4][\x80-\xBF]*`).
- `utf8.codepoint(s [, i [, j [, lax]]])`: Returns integer code points from UTF-8 string positions.
- `utf8.len(s [, i [, j [, lax]]])`: Validates UTF-8 encoding and counts code points.
- `utf8.offset(s, n [, i])`: Computes byte offset for the n-th code point.
- `utf8.codes(s [, lax])`: Iterates over pairs of `(byte_position, code_point)`.

**Architectural Invariant**: The standard `utf8` library measures **code points**, not terminal visual cell width. Grapheme clusters, emoji modifiers, and East Asian double-width characters (`unicode-width`, `unicode-segmentation`) belong strictly to the higher-level terminal host ABI (`bitty.text`), preserving strict Lua conformance in Phodopus.

**Fuel & Resumption Cost Model**: `utf8.len`, `utf8.codepoint`, and `utf8.offset` are implemented as resumable `Sequence`s that scan one code point at a time, charge `1` Fuel per byte examined, and return `SequencePoll::Pending` when Fuel is exhausted while preserving the byte cursor. `utf8.char` charges `1` Fuel per output byte and enforces the checked `MAX_STDLIB_STRING_BYTES` (16 MiB) ceiling; each `utf8.codes` iterator step charges the bytes it scans.

### 4.4 String Metatable and OOP Method Ergonomics

Standard Lua (5.3/5.4) provides object-oriented method call ergonomics on string values (e.g. `s:sub(1, 3)`, `s:upper()`, `s:find("pat")`, `s:format(...)`). In standard Lua semantics, all string instances share a global metatable whose `__index` field defaults to the standard `string` library table.

Phodopus implements string metatable and method dispatch via:

1. **Dedicated VM State Field**: `State<'gc>` maintains a `string_metatable: Table<'gc>` initialized during state construction (`State::new`). The metatable is exposed on `Context<'gc>` through `ctx.string_metatable()`.
2. **Standard Library Wiring**: When `load_string(ctx)` executes, `ctx.string_metatable()` is configured to map `MetaMethod::Index` (`__index`) to the loaded `string` module table (`ctx.string_metatable().set(ctx, MetaMethod::Index, string)`).
3. **VM `__index` Dispatch**: In `meta_ops::index`, when evaluating an index operation on a `Value::String(_)`, the VM queries `ctx.string_metatable()` for `MetaMethod::Index`. If nil (such as in an unmapped minimal VM without string library support), indexing produces a standard `could not index into a string value` error. If present, the lookup resolves either via table indexing (e.g. `string.sub`) or by invoking an index metamethod callback with `[string_value, key]`.
4. **Custom Extensibility**: User scripts or host extensions adding functions to `string` (e.g. `string.custom_fn = ...`) immediately become available via method call syntax (`s:custom_fn(...)`) on all string instances.
5. **Chaining and Stack Hygiene**: Chained method invocations (`s:sub(...):upper():format(...)`) execute with clean register isolation and zero frame leaks across calls.

### 4.5 Safe String Repetition (`string.rep`)

The `string.rep(s, n [, sep])` function generates a repeated string separated by an optional delimiter:

1. **Parameters**: `s` (string or number), `n` (integer repetition count), and optional `sep` (string or number delimiter).
2. **Semantics**:
   - If `n <= 0`: returns the empty string `""`.
   - If `n == 1`: returns `s` directly without delimiter concatenation or reallocation.
   - If `n > 1`: concatenates `n` copies of `s` interleaved with `n - 1` copies of `sep` (defaulting to empty string).
   - Implicit number coercion: numbers passed as `s` or `sep` are coerced to strings.
   - Method call syntax: supported via string metatable (`("foo"):rep(3, ",") == "foo,foo,foo"`).
3. **Safe Allocation Ceiling & DoS Protection**:
   - Buffer allocation size is strictly constrained by `MAX_STRING_REP_BYTES = 16 * 1024 * 1024` (16 MiB).
   - Total capacity calculation uses checked arithmetic (`checked_mul` and `checked_add`) across both `s` copies and `sep` delimiters.
   - Any arithmetic overflow or required capacity exceeding 16 MiB raises a standard Lua error (`"resulting string too large"`) rather than panicking or triggering out-of-memory crashes.
   - `string.rep` is proven constant-bounded: the capacity is computed and checked before any allocation, so a single call cannot exceed a fixed wall slice. It still charges `1` Fuel per output byte so the work is accounted deterministically.

### 4.6 Binary Packing and Unpacking (`string.pack`, `string.unpack`, `string.packsize`)

Phodopus implements standard Lua 5.3 binary packing and unpacking according to specification §6.4.2:

1. **Functions**:
   - `string.pack(fmt, v1, v2, ...)`: Serializes values according to format string `fmt` and returns a binary byte string.
   - `string.unpack(fmt, s [, pos])`: Deserializes values from binary string `s` starting at 1-based index `pos` (defaults to 1). Returns unpacked values followed by the 1-based index of the first unread byte. Supports negative indices counting backward from the end of `s`.
   - `string.packsize(fmt)`: Computes the static byte size resulting from packing format `fmt`. Raises a Lua error if variable-length options (`s` or `z`) are present.
   - Method call syntax: All three functions are exposed on the string metatable (`("b"):packsize()`, `(">I2"):pack(0x1234)`, `fmt:unpack(s)`).

2. **Format Specifiers**:
   - **Endianness**: `<` (little-endian), `>` (big-endian), `=` (native platform endianness). Default is native endian.
   - **Alignment**: `![n]` sets maximum alignment to `n` bytes (1 <= n <= 16, must be power of 2). Default alignment without `!` is 1 (unaligned). If `!` is provided without `n`, native alignment (8 bytes) is used.
   - **Signed Integers**: `b` (1 byte), `h` (2 bytes), `l` (8 bytes), `j` (Lua integer, 8 bytes), `i[n]` (signed integer of `n` bytes, 1 <= n <= 16, default 4 bytes).
   - **Unsigned Integers**: `B` (1 byte), `H` (2 bytes), `L` (8 bytes), `J` (Lua unsigned integer, 8 bytes), `T` (size_t, 8 bytes), `I[n]` (unsigned integer of `n` bytes, 1 <= n <= 16, default 4 bytes).
   - **Floating Point**: `f` (single precision IEEE 754, 4 bytes), `d` (double precision IEEE 754, 8 bytes), `n` (Lua number, 8 bytes).
   - **Fixed Strings**: `c[n]` (fixed-size string with `n` bytes; zero-padded on pack if string is shorter; errors if string is longer).
   - **Zero-Terminated Strings**: `z` (C-style string ending with `\0`; errors on pack if string contains embedded null bytes).
   - **Length-Prefixed Strings**: `s[n]` (string preceded by `n`-byte unsigned length prefix, 1 <= n <= 16, default `sizeof(size_t)` = 8 bytes). Length prefix is aligned according to `n` and maximum alignment.
   - **Padding**: `x` (1 zero byte), `Xop` (empty item aligning according to option `op` without reading or writing a value).
   - **Whitespace**: Space, tab, newline, and carriage return characters inside format strings are ignored.

3. **Memory Sandbox & Safe Bounds**:
   - Packed string allocation is strictly bounded by `MAX_STRING_PACK_BYTES = 16 * 1024 * 1024` (16 MiB sandbox ceiling).
   - All arithmetic on buffers, offsets, and string sizes uses checked arithmetic to prevent integer overflow.
   - Bounds errors, out-of-range integers, strings exceeding fixed buffer sizes, premature string termination during unpacking, and out-of-bounds `pos` values fail gracefully with standard Lua errors.
   - `string.pack`, `string.unpack`, and `string.packsize` are implemented as resumable `Sequence`s (`PackSequence`, `UnpackSequence`, `PacksizeSequence`) that advance a byte cursor over the format string, charge `1` Fuel per format byte, and return `SequencePoll::Pending` between format options when Fuel is exhausted. The cursor and partial output are preserved across resumption.

### 4.7 Sandboxed Dynamic Code Loading (`load`) and Global `_G`

Phodopus implements Lua 5.4 standard `load(chunk [, chunkname [, mode [, env]]])` and global `_G` in `crates/phodopus/src/stdlib/load.rs` and `crates/phodopus/src/stdlib/base.rs`:

1. **Global `_G` Registration**:
   - `_G` is registered as a global pointing directly to `ctx.globals()`.
   - Satisfies canonical Lua invariants: `_G == _ENV` in the main chunk, `_G._G == _G`, and global assignments (`_G.a = 1`) reflect across the global environment.

2. **Chunk Types & Iterator Protocol**:
   - **String Chunks**: Compiled directly via `Closure::load_with_env`.
   - **Function Chunks**: Consumed piecewise using an asynchronous `Sequence` (`BuildLoadString`) until the iterator function returns `nil` or the empty string `""`.
   - Number return values from the iterator function are automatically coerced to strings.
   - Non-string, non-number return values safely fail with `(nil, "error loading string: ...")`.
   - Other chunk argument types raise a standard `TypeError` (`"string or function"` expected).

3. **Sandbox-First Security (Text-Only Enforcement)**:
   - Binary bytecode chunks introduce severe security vulnerabilities (arbitrary memory inspection, bytecode verifier exploits, undefined behavior) in untrusted sandboxes.
   - Phodopus strictly restricts `load` to text chunks:
     - If `mode == "b"`, the operation is rejected immediately with `(nil, "attempt to load a binary chunk (mode is 't')")`.
     - If the chunk data starts with bytecode magic (any byte starting with `\x1b`, including `\x1bLua`), compilation is rejected with `(nil, "attempt to load a binary chunk (mode is 't')")`.
     - Invalid mode strings return `(nil, "invalid mode")`.

4. **Environment Isolation & Sandboxing**:
   - `env` defaults to `ctx.globals()`.
   - When a custom table `env` is supplied, the compiled closure's top-level `_ENV` upvalue binds strictly to that table (`Closure::load_with_env(ctx, Some(&*name), source, env)`).
   - Caller local variables are never leaked into the loaded chunk.

5. **Resource Limits & Fuel Accounting**:
   - Piecewise chunk assembly is bounded by `MAX_CHUNK_SIZE = 16 * 1024 * 1024` (16 MiB allocation ceiling). Exceeding this bound returns `(nil, "chunk too large")`.
   - Dynamic compilation consumes Fuel proportional to the chunk byte length: `exec.fuel().consume(count_fuel(32, source.len()))`.

6. **Error Return Semantics**:
   - Compilation and syntax errors do not panic or throw unhandled Rust errors; they return two values: `nil, error_message` where `error_message` is a Lua string.

---

## 5. Security & Verification Plan

1. **Format Bomb Test**: `string.format("%999999999s", "a")` must return a descriptive error, not allocate gigabytes of whitespace.
2. **Pattern Conformance**: Run the full PUC-Rio Lua 5.4 string pattern test suite against Phodopus; assert 100% equivalence.
3. **Malformed UTF-8 Handling**: Assert `utf8.len` returns `nil` and the byte offset of invalid byte sequences without panicking.
4. **String Method Invocation & Metatable Sandboxing**: Assert that string method calls (`s:len()`, `s:sub()`, `s:upper()`, `s:format()`, `s:find()`) correctly dispatch through the string metatable, custom extension functions on `string` propagate to method calls, direct indexing on strings behaves as expected, and missing methods error gracefully.
5. **String Repetition Ceiling & DoS Protection**: Assert that `string.rep` with astronomical counts (e.g. `string.rep("a", 1000000000)` or arithmetic overflow with `i64::MAX`) safely fails via `pcall` with `"resulting string too large"`, without memory blowup or panics.
6. **Binary Pack/Unpack Conformance & Sandbox Ceiling**: Assert round-trip fidelity across all integer widths, endianness flags, floating-point encodings, padding/alignment options, and string types in `crates/phodopus/tests/scripts/pack.lua`. Assert that requests exceeding the 16 MiB allocation ceiling safely fail via `pcall` without memory exhaustion.
7. **Sandboxed Dynamic Loading Conformance**: Assert in `crates/phodopus/tests/scripts/load.lua` and `globals.lua` that basic and piecewise string loading, argument passing, chunkname preservation in tracebacks, custom `_ENV` table sandboxing, syntax error returns (`nil, string`), and binary chunk rejections (`mode = "b"` or bytecode magic) operate without panics.
8. **Variable-Cost Fuel Interruption**: Assert in `crates/phodopus/tests/fuel_stdlib.rs` that under a small Fuel budget `string.format`, `string.gsub`, `utf8.len`, `utf8.codepoint`, and `string.pack`/`unpack` are preempted at least once and resume to the same result, and that hostile `format`/`gsub` growth over 16 MiB fails via `pcall` with `"resulting string too large"`.
