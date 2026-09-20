-- Comprehensive tests for string method call syntax and string metatable

-- 1. Basic method call syntax
do
    local s = "hello"
    assert(s:len() == 5)
    assert(("hello"):len() == 5)
    assert(s:sub(1, 3) == "hel")
    assert(s:upper() == "HELLO")
    assert(("HELLO"):lower() == "hello")
    assert(s:byte(1) == 104)
    assert(s:byte(1, 2) == 104)
    assert(s:reverse() == "olleh")
end

-- 2. Pattern methods
do
    local start_pos, end_pos = ("hello world"):find("world")
    assert(start_pos == 7 and end_pos == 11)

    local matched = ("foo123"):match("%d+")
    assert(matched == "123")

    local replaced, count = ("hello %s"):gsub("%%s", "world")
    assert(replaced == "hello world")
    assert(count == 1)

    local words = {}
    for word in ("foo bar baz"):gmatch("%a+") do
        table.insert(words, word)
    end
    assert(#words == 3)
    assert(words[1] == "foo")
    assert(words[2] == "bar")
    assert(words[3] == "baz")
end

-- 3. Format method
do
    assert(("hello %s"):format("world") == "hello world")
    assert(("%d + %d = %d"):format(2, 3, 5) == "2 + 3 = 5")
    assert(("%04d"):format(42) == "0042")
end

-- 4. Custom extension methods on string table
do
    string.custom_pad = function(self, n)
        local res = self
        while #res < n do
            res = res .. " "
        end
        return res
    end

    local s = "cat"
    assert(s:custom_pad(6) == "cat   ")
    assert(("hi"):custom_pad(5) == "hi   ")
    assert(#("hi"):custom_pad(5) == 5)

    -- Clean up custom method
    string.custom_pad = nil
    assert(string.custom_pad == nil)
end

-- 5. Direct table indexing on string
do
    assert(("hello")["sub"] == string.sub)
    assert(("hello")["len"] == string.len)
    assert(("hello")["upper"] == string.upper)
    assert(("hello")["lower"] == string.lower)
    assert(("hello")[1] == nil)
    assert(("hello")["nonexistent"] == nil)
    assert(("hello")[true] == nil)
    assert(("hello")[{}] == nil)
end

-- 6. Edge cases and chaining
do
    -- Empty string
    assert((""):len() == 0)
    assert(#("") == 0)
    assert((""):sub(1, 1) == "")
    assert(("" .. ""):len() == 0)

    -- Chaining methods
    assert(("  foo  "):sub(3, 5):upper() == "FOO")
    assert(("hello"):upper():reverse():lower() == "olleh")
    assert(("apple"):sub(1, 4):upper():reverse() == "LPPA")

    -- Unicode strings (UTF-8 byte-oriented operations)
    local u = "你好世界"
    assert(u:len() == #u)
    assert(u:sub(1, 3) == "你")
    assert(u:sub(4, 6) == "好")
    assert(u:sub(1, 6):len() == 6)

    -- Calling nonexistent method errors gracefully
    local ok, err = pcall(function()
        ("hello"):missing_method()
    end)
    assert(not ok)
end
