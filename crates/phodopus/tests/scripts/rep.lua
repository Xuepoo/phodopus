-- Tests for string.rep with separator support, number coercion, OOP method syntax, and DoS limits

function is_err(f)
    return pcall(f) == false
end

-- 1. Standard repetition
do
    assert(string.rep("a", 5) == "aaaaa")
    assert(string.rep("hello", 3) == "hellohellohello")
    assert(string.rep("abc", 4) == "abcabcabcabc")
end

-- 2. Separator parameter
do
    assert(string.rep("foo", 3, ",") == "foo,foo,foo")
    assert(string.rep("x", 4, "---") == "x---x---x---x")
    assert(string.rep("a", 2, "b") == "aba")
end

-- 3. Count 1
do
    assert(string.rep("foo", 1, ",") == "foo")
    assert(string.rep("a", 1) == "a")
    assert(string.rep("", 1, "sep") == "")
    assert(string.rep(123, 1, ",") == "123")
end

-- 4. Count 0 and negative
do
    assert(string.rep("foo", 0) == "")
    assert(string.rep("foo", -3) == "")
    assert(string.rep("foo", 0, ",") == "")
    assert(string.rep("foo", -1, ",") == "")
    assert(string.rep("", 0) == "")
    assert(string.rep("", -5, "x") == "")
end

-- 5. Empty string and empty separator
do
    assert(string.rep("", 10, "-") == "---------")
    assert(string.rep("a", 3, "") == "aaa")
    assert(string.rep("", 5, "") == "")
    assert(string.rep("", 5) == "")
end

-- 6. Number coercion
do
    assert(string.rep(123, 2) == "123123")
    assert(string.rep("a", 2, 9) == "a9a")
    assert(string.rep(10, 3, 0) == "10010010")
    assert(string.rep(3.14, 2) == "3.143.14")
    assert(string.rep("x", "3") == "xxx")
    assert(string.rep("x", 2.0) == "xx")
end

-- 7. OOP method call syntax
do
    assert(("hello "):rep(2) == "hello hello ")
    assert(("a"):rep(3, "-") == "a-a-a")
    assert(("foo"):rep(1, ",") == "foo")
    assert(("foo"):rep(0) == "")
    assert((""):rep(5, "=") == "====")
    assert(("a"):rep(2, "-"):rep(2, "+") == "a-a+a-a")
end

-- 8. DoS protection and allocation ceiling (16 MiB)
do
    -- Huge repetition counts must fail safely via pcall with "resulting string too large"
    local ok, err = pcall(string.rep, "a", 1000000000)
    assert(not ok, "huge count should fail")
    assert(string.find(tostring(err), "resulting string too large", 1, true) ~= nil,
        "error should state resulting string too large, got: " .. tostring(err))

    ok, err = pcall(string.rep, "abc", 20000000)
    assert(not ok, "overflowing total allocation should fail")
    assert(string.find(tostring(err), "resulting string too large", 1, true) ~= nil,
        "error should state resulting string too large, got: " .. tostring(err))

    -- Large separator causing total to exceed limit
    ok, err = pcall(string.rep, "a", 10000000, "bb")
    assert(not ok, "large separator allocation should fail")
    assert(string.find(tostring(err), "resulting string too large", 1, true) ~= nil,
        "error should state resulting string too large, got: " .. tostring(err))

    -- Arithmetic overflow with i64 values
    ok, err = pcall(string.rep, "hello", 9223372036854775807)
    assert(not ok, "i64::MAX count should fail safely")
    assert(string.find(tostring(err), "resulting string too large", 1, true) ~= nil,
        "error should state resulting string too large, got: " .. tostring(err))
end

-- 9. Type errors
do
    assert(is_err(function() return string.rep(nil, 1) end))
    assert(is_err(function() return string.rep(true, 1) end))
    assert(is_err(function() return string.rep({}, 1) end))
    assert(is_err(function() return string.rep("a", nil) end))
    assert(is_err(function() return string.rep("a", true) end))
    assert(is_err(function() return string.rep("a", {}) end))
    assert(is_err(function() return string.rep("a", "not_a_number") end))
    assert(is_err(function() return string.rep("a", 2.5) end))
    assert(is_err(function() return string.rep("a", 2, true) end))
    assert(is_err(function() return string.rep("a", 2, {}) end))
end
