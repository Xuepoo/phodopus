-- Regression tests for dirty stack inheritance and unprovided parameter nil-fill
-- Verifies upstream Piccolo Issue #145 ("Unprovided function parameters inherit stale stack values instead of nil")

-- Helper to populate a large number of registers with non-nil values
local function dirty_stack()
    local r1 = 111
    local r2 = "dirty_string_2"
    local r3 = { key = "dirty_table" }
    local r4 = 4.44
    local r5 = true
    local r6 = function() return "dirty_fn" end
    local r7 = 777
    local r8 = "dirty_string_8"
    local r9 = { 1, 2, 3 }
    local r10 = 1010
    local r11 = false
    local r12 = 12.12
    local r13 = "dirty_string_13"
    local r14 = { nested = { value = 42 } }
    local r15 = 1515
    local r16 = "dirty_string_16"
    local r17 = 1717
    local r18 = { a = 1, b = 2 }
    local r19 = 1919
    local r20 = "dirty_string_20"
    return r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14, r15, r16, r17, r18, r19, r20
end

-- =========================================================================
-- 1. Dirty Stack Inheritance (Regular Call: call_function)
-- =========================================================================
do
    -- Dirty stack first
    dirty_stack()

    -- Immediately call function declaring 8 parameters but passing only 2
    local function check_params(p1, p2, p3, p4, p5, p6, p7, p8)
        assert(p1 == "first", "p1 should match first argument")
        assert(p2 == "second", "p2 should match second argument")
        assert(p3 == nil, "p3 must be nil (unprovided)")
        assert(p4 == nil, "p4 must be nil (unprovided)")
        assert(p5 == nil, "p5 must be nil (unprovided)")
        assert(p6 == nil, "p6 must be nil (unprovided)")
        assert(p7 == nil, "p7 must be nil (unprovided)")
        assert(p8 == nil, "p8 must be nil (unprovided)")

        -- Uninitialized local variables inside the function must also be nil
        local l1, l2, l3, l4
        assert(l1 == nil and l2 == nil and l3 == nil and l4 == nil, "uninitialized locals must be nil")
        return true
    end

    assert(check_params("first", "second") == true)

    -- Test calling with 0 arguments
    dirty_stack()
    local function check_zero_args(a, b, c, d)
        assert(a == nil and b == nil and c == nil and d == nil, "all params must be nil when called with 0 args")
        return 42
    end
    assert(check_zero_args() == 42)
end

-- =========================================================================
-- 2. Lua Idiom: param = param or default
-- =========================================================================
do
    local function make_options(host, port, timeout, tls, retry_count)
        host = host or "localhost"
        port = port or 8080
        timeout = timeout or 30
        if tls == nil then tls = true end
        retry_count = retry_count or 3
        return {
            host = host,
            port = port,
            timeout = timeout,
            tls = tls,
            retry_count = retry_count,
        }
    end

    for _ = 1, 50 do
        dirty_stack()

        -- Default call (all unprovided)
        local opt1 = make_options()
        assert(opt1.host == "localhost")
        assert(opt1.port == 8080)
        assert(opt1.timeout == 30)
        assert(opt1.tls == true)
        assert(opt1.retry_count == 3)

        dirty_stack()

        -- Partial call (some provided, some unprovided)
        local opt2 = make_options("127.0.0.1", 9000)
        assert(opt2.host == "127.0.0.1")
        assert(opt2.port == 9000)
        assert(opt2.timeout == 30)
        assert(opt2.tls == true)
        assert(opt2.retry_count == 3)

        dirty_stack()

        -- Explicit nil passed vs unprovided
        local opt3 = make_options(nil, nil, 60, false, nil)
        assert(opt3.host == "localhost")
        assert(opt3.port == 8080)
        assert(opt3.timeout == 60)
        assert(opt3.tls == false)
        assert(opt3.retry_count == 3)
    end
end

-- =========================================================================
-- 3. Tail Call (tail_call_function)
-- =========================================================================
do
    local function target(a, b, c, d, e)
        assert(a == "tail_arg1", "a must match passed arg")
        assert(b == nil, "b must be nil in tail call")
        assert(c == nil, "c must be nil in tail call")
        assert(d == nil, "d must be nil in tail call")
        assert(e == nil, "e must be nil in tail call")
        return "tail_success"
    end

    local function caller_dirty()
        -- Dirty local registers heavily in the caller frame
        local _r1 = 999
        local _r2 = "deep_tail_dirty_string"
        local _r3 = { x = 100, y = 200 }
        local _r4 = 3.14159
        local _r5 = { "alpha", "beta", "gamma" }
        local _r6 = 123456
        -- Tail call passing only 1 argument to target expecting 5
        return target("tail_arg1")
    end

    dirty_stack()
    assert(caller_dirty() == "tail_success")

    -- Chained tail calls with shrinking parameter counts
    local function step3(final_val, unused1, unused2, unused3)
        assert(final_val == 777)
        assert(unused1 == nil)
        assert(unused2 == nil)
        assert(unused3 == nil)
        return "chain_complete"
    end

    local function step2(v1, v2, unused)
        assert(v1 == 10)
        assert(v2 == 20)
        assert(unused == nil)
        local d1, d2, d3, d4 = dirty_stack()
        return step3(777)
    end

    local function step1(x, unused1, unused2)
        assert(x == 1)
        assert(unused1 == nil)
        assert(unused2 == nil)
        local d1, d2, d3, d4 = dirty_stack()
        return step2(10, 20)
    end

    dirty_stack()
    assert(step1(1) == "chain_complete")
end

-- =========================================================================
-- 4. Non-Destructive Call: call_function_keep (Generic For Loop Iteration)
-- =========================================================================
do
    -- Generic for loop invokes iterator via Operation::GenericForCall (call_function_keep).
    -- We define an iterator function with extra declared parameters.
    local function my_iterator(state, current_index, extra1, extra2, extra3)
        -- GenericForCall passes 2 args: state and current index.
        -- extra1, extra2, extra3 must strictly evaluate to nil!
        assert(extra1 == nil, "extra1 must be nil in generic for call")
        assert(extra2 == nil, "extra2 must be nil in generic for call")
        assert(extra3 == nil, "extra3 must be nil in generic for call")

        if current_index < state.max then
            local next_idx = current_index + 1
            return next_idx, state.items[next_idx]
        end
        return nil
    end

    local function custom_ipairs(tbl)
        return my_iterator, { items = tbl, max = #tbl }, 0
    end

    dirty_stack()
    local collected = {}
    for idx, val in custom_ipairs({ "apple", "banana", "cherry" }) do
        -- Dirty stack inside the loop body to ensure next iteration's
        -- call_function_keep doesn't inherit dirty registers from the loop body
        local d1, d2, d3 = dirty_stack()
        table.insert(collected, val)
    end

    assert(#collected == 3)
    assert(collected[1] == "apple")
    assert(collected[2] == "banana")
    assert(collected[3] == "cherry")
end

-- =========================================================================
-- 5. Metamethod Call (call_meta_function)
-- =========================================================================
do
    -- Binary operator metamethods (__add, __sub, etc.) pass 2 arguments.
    -- Unprovided parameters must be nil.
    local mt = {
        __add = function(op1, op2, extra1, extra2, extra3)
            assert(extra1 == nil, "__add extra1 must be nil")
            assert(extra2 == nil, "__add extra2 must be nil")
            assert(extra3 == nil, "__add extra3 must be nil")
            return op1.val + op2.val
        end,
        __call = function(self, arg1, extra1, extra2)
            assert(arg1 == "called_arg", "arg1 must match passed argument")
            assert(extra1 == nil, "__call extra1 must be nil")
            assert(extra2 == nil, "__call extra2 must be nil")
            return "call_meta_ok"
        end,
        __index = function(self, key, extra1, extra2)
            assert(extra1 == nil, "__index extra1 must be nil")
            assert(extra2 == nil, "__index extra2 must be nil")
            if key == "magic" then
                return 42
            end
            return nil
        end,
        __concat = function(op1, op2, extra1, extra2)
            assert(extra1 == nil, "__concat extra1 must be nil")
            assert(extra2 == nil, "__concat extra2 must be nil")
            local v1 = type(op1) == "table" and op1.val or op1
            local v2 = type(op2) == "table" and op2.val or op2
            return tostring(v1) .. tostring(v2)
        end,
        __eq = function(op1, op2, extra1, extra2)
            assert(extra1 == nil, "__eq extra1 must be nil")
            assert(extra2 == nil, "__eq extra2 must be nil")
            return op1.val == op2.val
        end,
    }

    local obj1 = setmetatable({ val = 10 }, mt)
    local obj2 = setmetatable({ val = 20 }, mt)

    -- Test __add after dirtying stack
    dirty_stack()
    assert(obj1 + obj2 == 30)

    -- Test __call after dirtying stack
    dirty_stack()
    assert(obj1("called_arg") == "call_meta_ok")

    -- Test __index after dirtying stack
    dirty_stack()
    assert(obj1.magic == 42)

    -- Test __concat after dirtying stack
    dirty_stack()
    assert(obj1 .. obj2 == "1020")

    -- Test __eq after dirtying stack
    dirty_stack()
    assert(not (obj1 == obj2))
    assert(obj1 == setmetatable({ val = 10 }, mt))
end

-- =========================================================================
-- 6. Mixed Fixed Parameters and Varargs: function f(a, b, c, ...)
-- =========================================================================
do
    local function test_mixed(a, b, c, ...)
        local vararg_count = select("#", ...)
        local varargs = { ... }
        return {
            a = a,
            b = b,
            c = c,
            vararg_count = vararg_count,
            varargs = varargs,
        }
    end

    -- 6a. Calling with f(1) -> a == 1, b == nil, c == nil, select('#', ...) == 0
    dirty_stack()
    local res1 = test_mixed(1)
    assert(res1.a == 1, "a must be 1")
    assert(res1.b == nil, "b must be nil")
    assert(res1.c == nil, "c must be nil")
    assert(res1.vararg_count == 0, "vararg count must be 0")
    assert(#res1.varargs == 0, "varargs list must be empty")

    -- 6b. Calling with f() (zero args)
    dirty_stack()
    local res0 = test_mixed()
    assert(res0.a == nil)
    assert(res0.b == nil)
    assert(res0.c == nil)
    assert(res0.vararg_count == 0)
    assert(#res0.varargs == 0)

    -- 6c. Calling with f(1, 2)
    dirty_stack()
    local res2 = test_mixed(1, 2)
    assert(res2.a == 1)
    assert(res2.b == 2)
    assert(res2.c == nil)
    assert(res2.vararg_count == 0)
    assert(#res2.varargs == 0)

    -- 6d. Calling with f(1, 2, 3) (exact fixed params)
    dirty_stack()
    local res3 = test_mixed(1, 2, 3)
    assert(res3.a == 1)
    assert(res3.b == 2)
    assert(res3.c == 3)
    assert(res3.vararg_count == 0)
    assert(#res3.varargs == 0)

    -- 6e. Calling with f(1, 2, 3, 4) (fixed params + 1 vararg)
    dirty_stack()
    local res4 = test_mixed(1, 2, 3, 4)
    assert(res4.a == 1)
    assert(res4.b == 2)
    assert(res4.c == 3)
    assert(res4.vararg_count == 1)
    assert(#res4.varargs == 1 and res4.varargs[1] == 4)

    -- 6f. Calling with f(1, 2, 3, 4, 5, 6) (fixed params + multiple varargs)
    dirty_stack()
    local res6 = test_mixed(1, 2, 3, 4, 5, 6)
    assert(res6.a == 1)
    assert(res6.b == 2)
    assert(res6.c == 3)
    assert(res6.vararg_count == 3)
    assert(#res6.varargs == 3)
    assert(res6.varargs[1] == 4 and res6.varargs[2] == 5 and res6.varargs[3] == 6)

    -- 6g. Tail call with mixed fixed parameters and varargs
    local function tail_mixed_caller(arg)
        dirty_stack()
        return test_mixed(arg)
    end

    dirty_stack()
    local tail_res = tail_mixed_caller(42)
    assert(tail_res.a == 42)
    assert(tail_res.b == nil)
    assert(tail_res.c == nil)
    assert(tail_res.vararg_count == 0)
end

-- =========================================================================
-- 7. Deep Recursion and Coroutine Stack Safety
-- =========================================================================
do
    -- Recursive function with unprovided parameters at odd depths
    local function recurse(depth, a, b, c)
        if depth <= 0 then
            return "depth_reached"
        end
        dirty_stack()
        if depth % 2 == 0 then
            -- Pass all parameters
            assert(a ~= nil and b ~= nil and c ~= nil)
            return recurse(depth - 1, a)
        else
            -- b and c unprovided
            assert(a ~= nil)
            assert(b == nil, "b must be nil at odd depth")
            assert(c == nil, "c must be nil at odd depth")
            return recurse(depth - 1, a, "provided_b", "provided_c")
        end
    end

    assert(recurse(20, "initial_a", "initial_b", "initial_c") == "depth_reached")

    -- Coroutine stack safety: resume with fewer arguments than parameters
    local co = coroutine.create(function(x, y, z)
        assert(x == "co_arg")
        assert(y == nil, "coroutine unprovided y must be nil")
        assert(z == nil, "coroutine unprovided z must be nil")
        local yielded_val = coroutine.yield("yielded_1")
        assert(yielded_val == "resumed_arg")
        return "co_done"
    end)

    dirty_stack()
    local ok, ret = coroutine.resume(co, "co_arg")
    assert(ok and ret == "yielded_1")

    dirty_stack()
    local ok2, ret2 = coroutine.resume(co, "resumed_arg")
    assert(ok2 and ret2 == "co_done")
end

-- =========================================================================
-- 8. Protected Calls (pcall)
-- =========================================================================
do
    dirty_stack()
    local function pcall_target(p1, p2, p3, p4)
        assert(p1 == 100)
        assert(p2 == nil, "pcall unprovided p2 must be nil")
        assert(p3 == nil, "pcall unprovided p3 must be nil")
        assert(p4 == nil, "pcall unprovided p4 must be nil")
        return "pcall_ok", 200
    end

    local ok, res1, res2 = pcall(pcall_target, 100)
    assert(ok and res1 == "pcall_ok" and res2 == 200)

    -- pcall with 0 arguments
    dirty_stack()
    local function pcall_zero(a, b, c)
        assert(a == nil and b == nil and c == nil)
        return "zero_ok"
    end
    local ok_z, res_z = pcall(pcall_zero)
    assert(ok_z and res_z == "zero_ok")
end
