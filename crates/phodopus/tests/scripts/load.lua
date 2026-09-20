local function array_generator(arr)
    local i = 0
    return function()
        i = i + 1
        return arr[i]
    end
end

local primitives = { ["nil"] = 0, number = 0, string = 0, boolean = 0, ["function"] = 0, thread = 0 }
local function cmp_array_recurse(a, b)
    local a_ty = type(a)
    local b_ty = type(b)
    if a_ty ~= b_ty then
        return false
    end
    if primitives[a_ty] ~= nil then
        return a == b
    end
    if rawlen(a) ~= rawlen(b) then
        return false
    end
    for i = 1, rawlen(a) do
        if not cmp_array_recurse(rawget(a, i), rawget(b, i)) then
            return false
        end
    end
    return true
end

local log_arr = {}
function log(val)
    table.insert(log_arr, val)
end

-- Basic string loading
do
    local f, err = load("return 1 + 2")
    assert(f, err)
    assert(f() == 3)
end

-- Argument passing into loaded chunk
do
    local f, err = load("local a, b = ...; return a + b")
    assert(f, err)
    assert(f(10, 20) == 30)

    log_arr = {}
    local f2, err2 = load([[
        local args = table.pack(...)
        for i = 1, args.n do
            log(args[i])
        end
        return args[1]
    ]])
    assert(f2, err2)
    local r = f2("a", "b", "c")
    assert(r == "a")
    assert(cmp_array_recurse(log_arr, { "a", "b", "c" }))
end

-- Chunkname preservation in traceback
do
    local env = { debug = debug, error = error }
    local f, err = load([[
        tb = debug.traceback()
        error("boom")
    ]], "custom_chunk", "t", env)
    assert(f, err)

    local ok, res = pcall(f)
    assert(not ok)
    assert(res == "boom")
    assert(type(env.tb) == "string")
    assert(string.find(env.tb, "custom_chunk") ~= nil, "traceback must preserve chunkname")
end

-- Custom _ENV sandbox and caller local scope isolation
do
    local caller_secret = "secret_in_caller"
    local f, err = load("return secret", "test", "t", { secret = "sandboxed" })
    assert(f, err)
    assert(f() == "sandboxed")

    local f_isolated, err_isolated = load("return caller_secret", "test", "t", {})
    assert(f_isolated, err_isolated)
    assert(f_isolated() == nil, "caller locals must not be leaked into loaded chunk")

    local module = {}
    local f_mod, err_mod = load([[
        a = 1
        b = 2
        c = { [1] = "a" }
    ]], "name", "t", module)
    assert(f_mod, err_mod)
    f_mod()
    assert(module.a == 1 and module.b == 2 and module.c[1] == "a")
end

-- Environment defaults to global context
do
    local res = load("return _G")()
    assert(res == _G)

    local a = 15
    local res2 = load("return a")()
    assert(res2 == nil)

    local old_globals = _G
    _G = {}
    local res3 = load("return _G")()
    assert(res3 == _G)
    _G = old_globals
end

-- Nested load defaults to global context
do
    log_arr = {}

    local f, err = load([[
        local inner = load("log(32)")
        inner()
        log(16)
    ]], "name", "t", { load = load })
    assert(f, err)

    local ok, res = pcall(function() f() end)
    assert(not ok)
    assert(cmp_array_recurse(log_arr, { 32 }))
end

-- Function iterator chunks
do
    local read_func = array_generator({ "local x = 1", "; return x + 5" })
    local f, err = load(read_func)
    assert(f, err)
    assert(f() == 6)
end

do
    log_arr = {}

    local read_func = array_generator({ "log(1)", "log(2)", "log(3)" })
    local f, err = load(read_func)
    assert(f, err)
    f()

    assert(cmp_array_recurse(log_arr, { 1, 2, 3 }))
end

-- Numbers returned by chunk iterator coerced to strings
do
    log_arr = {}

    local read_func = array_generator({ "log(", 1, ")", "log(", 1.5, ")" })
    local f, err = load(read_func)
    assert(f, err)
    f()

    assert(cmp_array_recurse(log_arr, { 1, 1.5 }))
end

-- Non-string non-number returned by chunk iterator returns (nil, error)
do
    log_arr = {}

    local read_func = array_generator({ [[log("]], {}, [[")]] })
    local f, err = load(read_func)
    assert(f == nil)
    assert(type(err) == "string")
    assert(string.find(err, "error loading string") ~= nil)
end

-- Non-function non-string chunk argument raises type error
do
    local callable = setmetatable({}, {
        __call = array_generator({ "log(3)" })
    })
    local res, err = pcall(function()
        local f, err = load(callable)
        assert(f == nil and err ~= nil)
    end)
    assert(not res)

    local res2, err2 = pcall(function()
        load(true)
    end)
    assert(not res2)
end

-- Syntax error handling
do
    local f, err = load("function broken(")
    assert(f == nil)
    assert(type(err) == "string")
end

-- Binary chunk rejection (Sandbox-First Security)
do
    -- Mode 'b' must be rejected
    local f1, err1 = load("return 1", "chunk", "b")
    assert(f1 == nil)
    assert(type(err1) == "string")
    assert(string.find(err1, "binary chunk") ~= nil)

    -- Lua bytecode magic must be rejected even with mode 'bt'
    local f2, err2 = load("\x1bLua\x54\x00\x19\x93\r\n\x1a\n", "chunk", "bt")
    assert(f2 == nil)
    assert(type(err2) == "string")
    assert(string.find(err2, "binary chunk") ~= nil)

    -- Lua bytecode magic must be rejected with mode 't'
    local f3, err3 = load("\x1bLua\x54\x00\x19\x93\r\n\x1a\n", "chunk", "t")
    assert(f3 == nil)
    assert(type(err3) == "string")
    assert(string.find(err3, "binary chunk") ~= nil)

    -- Invalid mode must be rejected
    local f4, err4 = load("return 1", "chunk", "invalid_mode")
    assert(f4 == nil)
    assert(type(err4) == "string")
    assert(string.find(err4, "invalid") ~= nil)
end
