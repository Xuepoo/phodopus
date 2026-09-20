-- Sandboxed module system defaults and preload behavior.
--
-- The test-script harness runs with the default (preload-only) module
-- configuration, so this script can only exercise behavior that needs no host
-- registered VFS root. Rooted VFS resolution, the searcher chain, and circular
-- dependencies are covered by Rust tests under `tests/require.rs`.

do
    -- Zero ambient host access: no search path, no native loader, no C searcher.
    assert(type(require) == "function")
    assert(package.path == "")
    assert(package.cpath == nil)
    assert(package.loadlib == nil)
    assert(type(package.loaded) == "table")
    assert(type(package.preload) == "table")
    assert(type(package.searchers) == "table")
end

do
    -- Core built-ins resolve through the preload searcher and return the same
    -- table as the corresponding global.
    assert(require("string") == string)
    assert(require("table") == table)
    assert(require("math") == math)
end

do
    -- A preload loader registered directly in Lua resolves and executes without
    -- any filesystem query.
    package.preload["greeting"] = function()
        return { text = "hello" }
    end

    local greeting = require("greeting")
    assert(greeting.text == "hello")

    -- The result is cached by identity in package.loaded.
    assert(require("greeting") == greeting)
    assert(package.loaded["greeting"] == greeting)
end

do
    -- A loader that returns nothing is stored as the sentinel `true`.
    package.preload["empty"] = function() end
    assert(require("empty") == true)
    assert(package.loaded["empty"] == true)
end

do
    -- Circular dependencies terminate through the sentinel `true`: while a
    -- module is loading, requiring it again returns `true` instead of
    -- re-entering the loader.
    package.preload["cycle_a"] = function()
        return "a(" .. tostring(require("cycle_b")) .. ")"
    end
    package.preload["cycle_b"] = function()
        return "b(" .. tostring(require("cycle_a")) .. ")"
    end

    assert(require("cycle_a") == "a(b(true))")
end

do
    -- A missing module reports the candidates checked by the searcher chain.
    local ok, err = pcall(require, "definitely.missing")
    assert(not ok)
    assert(err:find("not found", 1, true))
    assert(err:find("package.preload", 1, true))
end

do
    -- Traversal and root-escape attempts are rejected before resolution.
    for _, name in ipairs({
        "../../../etc/passwd",
        "./relative",
        "/absolute",
        "a..b",
        "trailing.",
        "C:\\windows",
        "\\\\server\\share",
    }) do
        local ok, err = pcall(require, name)
        assert(not ok, "require(" .. name .. ") should have been rejected")
        assert(err:find("access violation", 1, true), tostring(err))
    end
end
