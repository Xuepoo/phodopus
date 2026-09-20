- ⚫️️ = unimplemented
- 🟡 = differing
- 🔵 = implemented
- ❗= will not implement
- 🤷‍♀️ = low importance

This table records **current Phodopus behavior against PUC-Lua 5.4**. It is
inherited from the upstream Piccolo project and is being reconciled with the
Phodopus implementation; statuses describe this repository, not upstream
Piccolo. "Implemented" does not imply sandbox-hardening, proportional Fuel
accounting, or production readiness; see the [Evolution Roadmap](docs/architecture/roadmap.md)
for phase status.

"Implemented" means "near 1:1 PUC-Lua behavior"[^0].

"Differing" means that there is an implementation, but it doesn't correspond to PUC-Lua behavior.

"Unimplemented" means there is no implementation (when used, `nil` is found) _or_
that calling the implementation with the corresponding arguments will error where in PUC-Lua it does not.

"Will Not Implement" is for functions that will not be implemented due to a fundamental difference between Phodopus's sandbox-first execution model and PUC-Lua.

"Low Importance" is for things that, while technically implementable, will
likely not be implemented due to differences between Phodopus and PUC-Lua.

**NOTE**: `(a[, b, c])` corresponds to the Lua docs' `(a[, b[, c]])` usage.

## Base

| Status | Function                                                       | Differences                                                                                                                            | Notes |
| ------ | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| 🔵     | `assert(v[, message])`                                         |                                                                                                                                        |       |
| 🔵     | `collectgarbage("count")`                                      |                                                                                                                                        |       |
| ⚫️    | `collectgarbage("collect")`                                    |                                                                                                                                        |       |
| ⚫️    | `collectgarbage("stop")`                                       |                                                                                                                                        |       |
| ⚫️    | `collectgarbage("restart")`                                    |                                                                                                                                        |       |
| ⚫️    | `collectgarbage("step"[, memkb])`                              |                                                                                                                                        |       |
| ⚫️    | `collectgarbage("isrunning")`                                  |                                                                                                                                        |       |
| 🤷‍♀️     | `collectgarbage("incremental"[, gcpause, stepmult, stepsize])` |                                                                                                                                        |       |
| 🤷‍♀️     | `collectgarbage("generational"[, minormult, majormult])`       |                                                                                                                                        |       |
| ⚫️    | `dofile([filename])`                                           |                                                                                                                                        |       |
| 🟡     | `error(message)`                                               | Due to `level` not being implemented for, all calls here give the same result as PUC-Lua `error(message, 0)` (or any invalid `level`). |       |
| ⚫️    | `error(message, level)`                                        |                                                                                                                                        |       |
| 🔵     | `_G` (value)                                                   |                                                                                                                                        | Pointing to global environment table                                                                                   |
| 🔵     | `getmetatable(object)`                                         |                                                                                                                                        |                                                                                                                        |
| 🟡     | `ipairs(t)`                                                    | PUC-Lua returns `iter, table, 0`, whereas Phodopus returns `iter, table`.                                                              |                                                                                                                        |
| 🔵     | `load(chunk[, chunkname, mode, env])`                          | Text-only compilation enforced for sandbox security; binary bytecode chunks (`mode = "b"` or binary signature) are rejected.           | Piecewise iterator protocol, fuel accounting, and 16 MiB assembly ceiling supported.                                   |
| ⚫️    | `loadfile([filename, mode, env])`                              |                                                                                                                                        |       |
| 🔵     | `next(table [, index])`                                        |                                                                                                                                        |       |
| 🔵     | `pairs(t)`                                                     | By default, PUC-Lua returns `iter, table, nil`, whereas Phodopus returns `iter, table`.                                                  |       |
| 🔵     | `pcall(f, args...)`                                            |                                                                                                                                        |       |
| 🔵     | `print(args...)`                                               |                                                                                                                                        |       |
| ⚫️    | `rawequal(v1, v2)`                                             |                                                                                                                                        |       |
| 🔵     | `rawget(table, index)`                                         |                                                                                                                                        |       |
| 🔵    | `rawlen(v)`                                                    |                                                                                                                                        |       |
| 🔵     | `rawset(table, index, value)`                                  |                                                                                                                                        |       |
| 🔵     | `select(index, args...)`                                       |                                                                                                                                        |       |
| 🔵     | `setmetatable(table, metatable)`                               |                                                                                                                                        |       |
| 🔵    | `tonumber(e[, base])`                                          |                                                                                                                                        |       |
| 🟡     | `tostring(v)`                                                  | Phodopus does not use the metatable field `__name` by default, while PUC-Lua does.                                                      |       |
| 🔵     | `type(v)`                                                      |                                                                                                                                        |       |
| 🔵    | `_VERSION` (value)                                             |                                                                                                                                        |       |
| ⚫️    | `warn(msg, args...)`                                           |                                                                                                                                        |       |
| ⚫️    | `xpcall(f, msgh, args...)`                                     |                                                                                                                                        |       |

[^0]: Hedging b/c I don't know PUC-Lua like my reverse palm, and there might be differing behaviors if you poke both implementations to death, but that's not what this document is for.

## Coroutine

| Status | Function                | Differences | Notes |
| ------ | ----------------------- | ----------- | ----- |
| ⚫️️   | `close(co)`             |             |       |
| 🔵     | `create(f)`             |             |       |
| ⚫️️   | `isyieldable([co])`     |             |       |
| 🔵     | `resume(co[, vals...])` |             |       |
| 🔵     | `running()`             |             |       |
| 🔵     | `status(co)`            |             |       |
| ⚫️️   | `wrap(f)`               |             |       |
| 🔵     | `yield(args...)`        |             |       |

## Package

| Status | Function                             | Differences                                                                                     | Notes |
| ------ | ------------------------------------ | ----------------------------------------------------------------------------------------------- | ----- |
| ⚫️️   | (global) `require(modname)`          |                                                                                                 |       |
| ⚫️️   | `config` (value)                     |                                                                                                 |       |
| ❗     | `cpath` (value)                      |                                                                                                 |       |
| ⚫️️   | `loaded` (value)                     |                                                                                                 |       |
| ❗     | `loadlib(libname, funcname)`         |                                                                                                 |       |
| ⚫️️   | `path` (value)                       |                                                                                                 |       |
| ⚫️️   | `preload` (value)                    |                                                                                                 |       |
| ⚫️️   | `searchers` (value)                  | This implementation will differ from PUC-Lua because Phodopus does not support C loaders |       |
| ⚫️️   | `searchpath(name, path[, sep, rep])` |                                                                                                 |       |

## String

| Status | Function                          | Differences | Notes |
| ------ | --------------------------------- | ----------- | ----- |
| 🔵   | `byte(s[, i, j])`                 |             |       |
| 🔵   | `char(args...)`                   |             |       |
| ⚫️️   | `dump(function[, strip])`         |                                             |                                                      |
| 🔵   | `find(s, pattern[, init, plain])` |                                             | Implemented via native Lua pattern engine (`lsonar`) |
| 🔵   | `format(formatstring, args...)`   |                                             |                                                      |
| 🔵   | `gmatch(s, pattern[, init])`      |                                             | Stateful iterator callback; supports optional `init` |
| 🔵   | `gsub(s, pattern, repl[, n])`     | Function replacements currently unsupported | Supports string and table replacements               |
| 🔵     | `len(s)`                          |                                             |                                                      |
| 🔵   | `lower(s)`                        |                                             |                                                      |
| 🔵   | `match(s, pattern[, init])`       |                                             | Implemented via native Lua pattern engine (`lsonar`) |
| 🔵   | `pack(fmt, values...)`            | Safe allocation ceiling (16 MiB)            | Binary pack engine with 16 MiB allocation ceiling    |
| 🔵   | `packsize(fmt)`                   |                                             | Calculates binary size; rejects variable-length formats (`s`, `z`) |
| 🔵   | `rep(s, n[, sep])`                | Safe allocation ceiling (16 MiB)            | Enforces 16 MiB DoS ceiling to prevent OOM attacks   |
| 🔵   | `reverse(s)`                      |             |       |
| 🔵   | `sub(s, i[, j])`                  |             |       |
| 🔵   | `unpack(fmt, s[, pos])`           |                                             | Binary unpack engine; returns unpacked values and next position |
| 🔵   | `upper(s)`                        |             |       |
| 🔵   | String metatable (`__index`)      |             | String metatable `__index` bound to `string` table, enabling OOP method syntax (`s:method(...)`) |

## UTF8

| Status | Function                     | Differences | Notes |
| ------ | ---------------------------- | ----------- | ----- |
| 🔵     | `char(args..)`               |             |       |
| 🔵     | `charpattern` (value)        |             |       |
| 🔵     | `codes(s[, lax])`            |             | `lax` parameter is reserved/not yet supported |
| 🔵     | `codepoint(s[, i, j, lax])`  |             | `lax` parameter is reserved/not yet supported |
| 🔵     | `len(s[, i, j, lax])`        |             | `lax` parameter is reserved/not yet supported |
| 🔵     | `offset(s, n[, i])`          |             |       |

## Table

| Status | Function                     | Differences | Notes |
| ------ | ---------------------------- | ----------- | ----- |
| 🔵     | `concat(list[, sep, i, j])`  |             | Supports the `__concat` metamethod |
| 🔵     | `insert(list, [pos,] value)` |             |       |
| 🔵     | `move(a1, f, e, t[, a2])`    |             | Currently implemented with a Lua polyfill |
| 🔵     | `pack(args...)`              |             |       |
| 🔵     | `remove(list[, pos])`        |             |       |
| 🔵     | `sort(list[, comp])`         |             | Currently implemented with a Lua polyfill using a simple merge sort, rather than PUC-Rio Lua's quicksort impl |
| 🔵     | `unpack(list[, i, j])`       |             |       |

## Math

I'm not going over these with a fine-tooth comb, if it exists (and takes the specified number of arguments), it's considered implemented. (Except for "basic" identities like $\cos(0) = 1$ and stuff like that.)

| Status | Function             | Differences | Notes |
| ------ | -------------------- | ----------- | ----- |
| 🔵     | `abs(x)`             |             |       |
| 🔵     | `acos(x)`            |             |       |
| 🔵     | `asin(x)`            |             |       |
| 🔵     | `atan(y[, x])`       |             |       |
| 🔵     | `ceil(x)`            |             |       |
| 🔵     | `cos(x)`             |             |       |
| 🔵     | `deg(x)`             |             |       |
| 🔵     | `exp(x)`             |             |       |
| 🔵     | `floor(x)`           |             |       |
| 🔵     | `fmod(x, y)`         |             |       |
| 🔵     | `huge` (value)       |             |       |
| 🔵     | `log(x[, base])`     |             |       |
| 🔵     | `max(x, args...)`    |             |       |
| 🔵     | `maxinteger` (value) |             |       |
| 🔵     | `min(x, args...)`    |             |       |
| 🔵     | `mininteger` (value) |             |       |
| 🔵     | `modf(x)`            |             |       |
| 🔵     | `pi` (value)         |             |       |
| 🔵     | `rad(x)`             |             |       |
| 🔵     | `random([m, n])`     |             |       |
| 🔵     | `randomseed([x, y])` |             |       |
| 🔵     | `sin(x)`             |             |       |
| 🔵     | `sqrt(x)`            |             |       |
| 🔵     | `tan(x)`             |             |       |
| 🔵     | `tointeger(x)`       |             |       |
| 🔵     | `type(x)`            |             |       |
| 🔵     | `ult(m, n)`          |             |       |

## I/O

Phodopus intentionally omits the `io` module from its sandboxed core. The
`print` global exists as an opt-in host affordance, not as the `io` library;
file and stream APIs are absent so untrusted scripts have no ambient filesystem
access.

| Status | Function                      | Differences                                                                                                                 | Notes |
| ------ | ----------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ----- |
| ⚫️    | `close([file])`               |                                                                                                                             |       |
| ⚫️    | `flush()`                     |                                                                                                                             |       |
| ⚫️    | `input([file])`               |                                                                                                                             |       |
| ⚫️    | `lines([filename, args...])`  |                                                                                                                             |       |
| ⚫️    | `open(filename [, mode])`     |                                                                                                                             |       |
| ⚫️    | `output([file])`              |                                                                                                                             |       |
| ⚫️/❗ | `popen(prog[, mode])`         | Spawns an external process, which conflicts with the no-ambient-authority sandbox; not implemented. |       |
| ⚫️    | `read(args...)`               |                                                                                                                             |       |
| ⚫️    | `tmpfile()`                   |                                                                                                                             |       |
| ⚫️    | `type(obj)`                   |                                                                                                                             |       |
| ⚫️    | `write(args...)`              |                                                                                                                             |       |
| ⚫️    | `file:close()`                |                                                                                                                             |       |
| ⚫️    | `file:flush()`                |                                                                                                                             |       |
| ⚫️    | `file:lines(args...)`         |                                                                                                                             |       |
| ⚫️    | `file:read(args...)`          |                                                                                                                             |       |
| ⚫️    | `file:seek([whence, offset])` |                                                                                                                             |       |
| ⚫️    | `file:setvbuf(mode[, size])`  |                                                                                                                             |       |
| ⚫️    | `file:write(args...)`         |                                                                                                                             |       |

## OS

Phodopus intentionally omits the `os` module from its sandboxed core: process,
clock, environment, and filesystem operations are ambient authority that
untrusted scripts must not receive. Hosts that need such capabilities expose
them as explicit, capability-gated host services.

| Status | Function                        | Differences                                                                                                                                                                                | Notes |
| ------ | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----- |
| ⚫️    | `clock()`                       |                                                                                                                                                                                            |       |
| ⚫️    | `date([format, time])`          |                                                                                                                                                                                            |       |
| ⚫️    | `difftime(t2, t1)`              |                                                                                                                                                                                            |       |
| ❗     | `execute([command])`            | Spawning a host process is ambient authority and is not implemented in the sandboxed core.                                                                                                 |       |
| ⚫️    | `exit([code, close])`           |                                                                                                                                                                                            |       |
| ⚫️    | `getenv(varname)`               | Environment access is ambient authority and is not implemented in the sandboxed core.                                                                                                      |       |
| ⚫️    | `remove(filename)`              |                                                                                                                                                                                            |       |
| ⚫️    | `rename(oldname, newname)`      |                                                                                                                                                                                            |       |
| ❗     | `setlocale(locale[, category])` | Host locale mutation is out of scope for a sandboxed runtime.                                                                                                                              |       |
| ⚫️    | `time([table])`                 |                                                                                                                                                                                            |       |
| ⚫️    | `tmpname()`                     |                                                                                                                                                                                            |       |

## Debug

Phodopus implements `debug.traceback` only. The remaining `debug` library is
largely unimplemented: several functions depend on C-style hooks, exact stack
introspection, or registry internals that the stackless VM deliberately does not
expose. This section records what is implemented and what is theoretically
possible.

| Status | Function                                  | Implementation Notes / Differences                                                                                                                                                                        | Notes |
| ------ | ----------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- |
| ⚫️    | `debug()`                                 |                                                                                                                                                                                                           |       |
| ❗     | `gethook([thread])`                       |                                                                                                                                                                                                           |       |
| ⚫️    | `getinfo([thread, ]f[, what])`            |                                                                                                                                                                                                           |       |
| ⚫️    | `getlocal([thread, ]f, local)`            |                                                                                                                                                                                                           |       |
| ⚫️    | `getmetatable(value)`                     |                                                                                                                                                                                                           |       |
| ⚫️    | `getregistry()`                           |                                                                                                                                                                                                           |       |
| ⚫️    | `getupvalue(f, up)`                       |                                                                                                                                                                                                           |       |
| ⚫️    | `getuservalue(u, n)`                      |                                                                                                                                                                                                           |       |
| ❗     | `sethook([thread, ] hook, mask[, count])` |                                                                                                                                                                                                           |       |
| ⚫️    | `setlocal([thread, ]level, local, value)` |                                                                                                                                                                                                           |       |
| ⚫️    | `setmetatable(value, table)`              | Interesting thing to note is that this is _not_ the base library `setmetatable`, as `debug.setmetatable`'s first argument accepts any Lua value, while `setmetatable`'s first argument _must_ be a table. |       |
| ⚫️    | `setupvalue(f, up, value)`                |                                                                                                                                                                                                           |       |
| ⚫️    | `setuservalue(udata, value, n)`           |                                                                                                                                                                                                           |       |
| 🔵    | `traceback([thread,][message, level])`    |                                                                                                                                                                                                           |       |
| ⚫️    | `upvalueid(f, n)`                         |                                                                                                                                                                                                           |       |
| ⚫️    | `upvaluejoin(f1, n1, f2, n2)`             |                                                                                                                                                                                                           |       |

## VM & Language Semantics

| Status | Feature / Semantics | Differences | Notes |
| ------ | ------------------- | ----------- | ----- |
| 🔵     | Function parameter nil-fill | None | Unprovided parameters evaluate strictly to `nil`. Verified against upstream Piccolo Issue #145. |
| 🔵     | Default parameter idiom (`param = param or default`) | None | Reliable under repeated calls, tail calls, and dirty stacks. |
| 🔵     | Stack frame isolation | None | Registers from previous frames are zero-cost nil-filled via `resize(base + stack_size, Value::Nil)`. |
| 🔵     | Tail calls (`return f(...)`) | None | Constant stack space tail calls supported; registers normalized on tail call push. |
| 🔵     | Vararg alignment (`...`) | None | Correctly rotated via `rotate_right(var_params)`. When $N \le F$, `select('#', ...)` evaluates to 0. |
| 🔵     | String method call syntax (`s:method(...)`) | None | String metatable `__index` dispatch via VM `meta_ops::index`. Absorbs upstream Piccolo PR #134 & PR #58. |
