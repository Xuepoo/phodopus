---
title: Modular Standard Library Specification
description: Normative specification for capability-gated standard libraries, Lua patterns, string formatting, and UTF-8
category: specifications
audience: developers
document_type: specification
design_status: accepted
implementation_status: partial
website_publish: true
sidebar_order: 22
---

# Modular Standard Library Specification

> Status: Design **accepted** | Implementation: **partial** (baseline stdlib subsets present; authentic Lua patterns, string.format, and utf8 planned for Phase 1). This document defines the modular architecture, standard library implementations, authentic Lua pattern matching, and Unicode support for Phodopus.

---

## 1. Purpose and Scope

### In Scope

- Modular standard library loading: libraries can be loaded selectively to minimize footprint and attack surface.
- String formatting (`string.format`) specification and supported conversions.
- Authentic Lua pattern matching (`find`, `match`, `gsub`, `gmatch`) behavior.
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

- `%c`: Single character from integer byte code.
- `%d`, `%i`: Signed integer format.
- `%o`, `%u`, `%x`, `%X`: Unsigned octal, decimal, and hexadecimal.
- `%f`, `%e`, `%E`, `%g`, `%G`: Floating point formats.
- `%s`: String conversion (respecting embedded null bytes).
- `%q`: Quoted string suitable for Lua source deserialization.

**Safety Constraint**: To prevent memory bomb attacks via `%999999999s`, width specifiers are capped at an implementation bound of 64 KiB per field.

### 4.2 Authentic Lua Pattern Matching

Phodopus rejects mapping `string.match` directly to the Rust `regex` crate, which would introduce divergent semantics (e.g. `\d` instead of `%d`, different quantifier rules, missing frontier patterns).

Instead, Phodopus incorporates an authentic Lua pattern interpreter supporting:

1. **Character Classes**: `%a` (letters), `%c` (control), `%d` (digits), `%l` (lowercase), `%p` (punctuation), `%s` (space), `%u` (uppercase), `%w` (alphanumeric), `%x` (hex), and their uppercase inverse classes (`%A`, `%D`, etc.).
2. **Magic Characters**: `^`, `$`, `(`, `)`, `%`, `.`, `[`, `]`, `*`, `+`, `-`, `?`.
3. **Lazy Quantifier**: `-` matches 0 or more characters greedily-minimal.
4. **Frontier Patterns**: `%f[set]` matches empty string transitions into `set`.
5. **Captures**: Nested captures, position captures `()`, and replacement string tokens (`%0` to `%9`) in `string.gsub`.

### 4.3 Unicode Support (`utf8` Library)

The `utf8` module implements standard Lua 5.3/5.4 functions:

- `utf8.char(...)`: Encodes zero or more Unicode code points into a UTF-8 byte string.
- `utf8.codepoint(s [, i [, j]])`: Returns integer code points from UTF-8 string positions.
- `utf8.len(s [, i [, j]])`: Validates UTF-8 encoding and counts code points.
- `utf8.offset(s, n [, i])`: Computes byte offset for the n-th code point.
- `utf8.codes(s)`: Iterates over pairs of `(byte_position, code_point)`.

**Architectural Invariant**: The standard `utf8` library measures **code points**, not terminal visual cell width. Grapheme clusters, emoji modifiers, and East Asian double-width characters (`unicode-width`, `unicode-segmentation`) belong strictly to the higher-level terminal host ABI (`bitty.text`), preserving strict Lua conformance in Phodopus.

---

## 5. Security & Verification Plan

1. **Format Bomb Test**: `string.format("%999999999s", "a")` must return a descriptive error, not allocate gigabytes of whitespace.
2. **Pattern Conformance**: Run the full PUC-Rio Lua 5.4 string pattern test suite against Phodopus; assert 100% equivalence.
3. **Malformed UTF-8 Handling**: Assert `utf8.len` returns `nil` and the byte offset of invalid byte sequences without panicking.
