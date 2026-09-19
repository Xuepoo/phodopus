-- Native Lua Pattern Matching Test Suite for Phodopus (Task CTX-0004)

function is_err(f)
    return pcall(f) == false
end

-- =========================================================================
-- 1. Character Classes (%a, %d, %w, %s, sets [a-z], inverted sets [^0-9])
-- =========================================================================
do
    -- %a (letters) and %A (non-letters)
    assert(string.match("123abc456", "%a+") == "abc")
    assert(string.match("abc123xyz", "%A+") == "123")

    -- %d (digits) and %D (non-digits)
    assert(string.match("hello 42 world", "%d+") == "42")
    assert(string.match("42hello42", "%D+") == "hello")

    -- %w (alphanumeric) and %W (non-alphanumeric)
    assert(string.match("!@#abc123$%^", "%w+") == "abc123")
    assert(string.match("abc!@#xyz", "%W+") == "!@#")

    -- %s (whitespace) and %S (non-whitespace)
    assert(string.match("hello \t\n world", "%s+") == " \t\n ")
    assert(string.match("   hello   ", "%S+") == "hello")

    -- Character sets [a-z]
    assert(string.match("ABCdefGHI", "[a-z]+") == "def")
    assert(string.match("123-abc-789", "[0-9]+") == "123")

    -- Inverted character sets [^0-9]
    assert(string.match("12345abcdef67890", "[^0-9]+") == "abcdef")
    assert(string.match("abc123xyz", "[^a-z]+") == "123")
end

-- =========================================================================
-- 2. Modifiers (*, +, -, ?)
-- =========================================================================
do
    -- * (0 or more, greedy)
    assert(string.match("aaabbb", "a*") == "aaa")
    assert(string.match("bbb", "a*") == "")

    -- + (1 or more, greedy)
    assert(string.match("aaabbb", "a+") == "aaa")
    assert(string.match("bbb", "a+") == nil)

    -- - (0 or more, lazy / non-greedy)
    assert(string.match("aaabbb", "a-") == "")
    assert(string.match("<foo> and <bar>", "<.*>") == "<foo> and <bar>")
    assert(string.match("<foo> and <bar>", "<.->") == "<foo>")

    -- ? (0 or 1)
    assert(string.match("colour", "colou?r") == "colour")
    assert(string.match("color", "colou?r") == "color")
    assert(string.match("hello", "he?l") == "hel")
    assert(string.match("hllo", "he?l") == "hl")
end

-- =========================================================================
-- 3. Anchors (^ and $)
-- =========================================================================
do
    -- ^ anchor (start of string)
    local s, e = string.find("hello world", "^hello")
    assert(s == 1 and e == 5)
    assert(string.find("hello world", "^world") == nil)

    -- $ anchor (end of string)
    local s2, e2 = string.find("hello world", "world$")
    assert(s2 == 7 and e2 == 11)
    assert(string.find("hello world", "hello$") == nil)

    -- Combined ^ and $
    assert(string.match("12345", "^%d+$") == "12345")
    assert(string.match("123a45", "^%d+$") == nil)
end

-- =========================================================================
-- 4. Captures () and Back-references in Replacement (%0, %1..%9)
-- =========================================================================
do
    -- Single capture
    assert(string.match("hello 123 world", "(%d+)") == "123")

    -- Multiple captures
    local k, v = string.match("name = Phodopus", "(%w+)%s*=%s*(%w+)")
    assert(k == "name" and v == "Phodopus")

    -- Nested captures
    local whole, first = string.match("foo", "((f)oo)")
    assert(whole == "foo" and first == "f")

    -- Back-references in gsub (%1, %2)
    local swapped = string.gsub("first second", "(%w+)%s+(%w+)", "%2 %1")
    assert(swapped == "second first")

    -- Whole match back-reference (%0)
    local bracketed = string.gsub("hello world", "%a+", "[%0]")
    assert(bracketed == "[hello] [world]")

    -- Escaped percent (%%)
    local percented = string.gsub("price 100", "%d+", "%0%%")
    assert(percented == "price 100%")
end

-- =========================================================================
-- 5. Frontier Patterns (%f[set])
-- =========================================================================
do
    -- Frontier pattern for word boundary
    local m = string.match("the cat in the hat", "%f[%w]cat%f[^%w]")
    assert(m == "cat")

    -- Matching whole words only
    local res, n = string.gsub("the cat and scattered cats", "%f[%w]cat%f[^%w]", "DOG")
    assert(res == "the DOG and scattered cats" and n == 1)

    -- Frontier at start of string
    assert(string.match("cat", "%f[%w]cat") == "cat")
end

-- =========================================================================
-- 6. string.find (plain=true, plain=false, negative init)
-- =========================================================================
do
    -- plain = true (literal search)
    local s, e = string.find("a.b.c", ".", 1, true)
    assert(s == 2 and e == 2)

    local s2, e2 = string.find("foo%bar", "%", 1, true)
    assert(s2 == 4 and e2 == 4)

    -- plain = false (pattern search)
    local s3, e3 = string.find("a.b.c", ".")
    assert(s3 == 1 and e3 == 1) -- . matches first character 'a'

    -- With captures in find
    local s4, e4, cap1, cap2 = string.find("key: value", "(%w+):%s*(%w+)")
    assert(s4 == 1 and e4 == 10)
    assert(cap1 == "key" and cap2 == "value")

    -- Negative init
    local s5, e5 = string.find("banana", "an", -4)
    assert(s5 == 4 and e5 == 5)

    local s6, e6 = string.find("banana", "an", -2)
    assert(s6 == nil and e6 == nil)

    -- Negative init smaller than -len clamps to 1
    local s7, e7 = string.find("banana", "ba", -10)
    assert(s7 == 1 and e7 == 2)

    -- Init > len + 1 returns nil
    assert(string.find("banana", "a", 10) == nil)

    -- Empty pattern
    local s8, e8 = string.find("abc", "")
    assert(s8 == 1 and e8 == 0)

    local s9, e9 = string.find("abc", "", 4)
    assert(s9 == 4 and e9 == 3)

    assert(string.find("abc", "", 5) == nil)
end

-- =========================================================================
-- 7. string.match (with captures and without captures)
-- =========================================================================
do
    -- Without captures: returns whole match
    assert(string.match("age: 25", "%d+") == "25")
    assert(string.match("no digits here", "%d+") == nil)

    -- With captures: returns each capture
    local y, m, d = string.match("2026-09-20", "(%d+)%-(%d+)%-(%d+)")
    assert(y == "2026")
    assert(m == "09")
    assert(d == "20")

    -- With init parameter
    assert(string.match("123 abc 456", "%d+", 5) == "456")
    assert(string.match("123 abc 456", "%d+", -4) == "456")
    assert(string.match("123 abc 456", "%d+", 20) == nil)
end

-- =========================================================================
-- 8. string.gmatch (iterating words in a sentence)
-- =========================================================================
do
    -- Iterating words
    local words = {}
    for w in string.gmatch("from Lua to Rust with love", "%a+") do
        table.insert(words, w)
    end
    assert(#words == 6)
    assert(words[1] == "from")
    assert(words[2] == "Lua")
    assert(words[3] == "to")
    assert(words[4] == "Rust")
    assert(words[5] == "with")
    assert(words[6] == "love")

    -- Iterating key-value pairs
    local config = {}
    for k, v in string.gmatch("host=localhost;port=8080;secure=true", "(%w+)=(%w+)") do
        config[k] = v
    end
    assert(config["host"] == "localhost")
    assert(config["port"] == "8080")
    assert(config["secure"] == "true")

    -- gmatch with init
    local partial = {}
    for w in string.gmatch("apple banana cherry", "%a+", 7) do
        table.insert(partial, w)
    end
    assert(#partial == 2)
    assert(partial[1] == "banana")
    assert(partial[2] == "cherry")
end

-- =========================================================================
-- 9. string.gsub (string and table replacements)
-- =========================================================================
do
    -- Basic string replacement
    local res, n = string.gsub("hello world", "world", "phodopus")
    assert(res == "hello phodopus" and n == 1)

    -- Count limit n
    local res2, n2 = string.gsub("banana", "a", "o", 2)
    assert(res2 == "bonona" and n2 == 2)

    -- n <= 0 produces 0 substitutions
    local res3, n3 = string.gsub("banana", "a", "o", 0)
    assert(res3 == "banana" and n3 == 0)

    local res3b, n3b = string.gsub("banana", "a", "o", -1)
    assert(res3b == "banana" and n3b == 0)

    -- Table replacement
    local map = {
        name = "Lua",
        lang = "Rust",
        vm = "Phodopus"
    }
    local templated, t_n = string.gsub("$name on $vm via $lang", "%$(%w+)", map)
    assert(templated == "Lua on Phodopus via Rust" and t_n == 3)

    -- Table replacement with numbers
    local num_map = { a = 1, b = 2 }
    local num_res, num_n = string.gsub("a + b", "(%w)", num_map)
    assert(num_res == "1 + 2" and num_n == 2)

    -- Table replacement retains original on missing key or false
    local retain_map = { keep = false }
    local retain_res, retain_n = string.gsub("keep miss", "%w+", retain_map)
    assert(retain_res == "keep miss" and retain_n == 2)

    -- Empty pattern replacement (inserts before/after every character)
    local empty_res, empty_n = string.gsub("abc", "", "-")
    assert(empty_res == "-a-b-c-" and empty_n == 4)
end

-- =========================================================================
-- 10. Error Handling
-- =========================================================================
do
    -- Malformed patterns
    assert(is_err(function() return string.find("abc", "[") end))
    assert(is_err(function() return string.find("abc", "%") end))
    assert(is_err(function() return string.match("abc", "(") end))
    assert(is_err(function() return string.gmatch("abc", "[") end))

    -- Invalid replacement string
    assert(is_err(function() return string.gsub("abc", "%a", "%") end))
    assert(is_err(function() return string.gsub("abc", "(%a)", "%2") end))
    assert(is_err(function() return string.gsub("abc", "%a", "%z") end))

    -- Invalid replacement value
    assert(is_err(function() return string.gsub("abc", "%a", true) end))
    assert(is_err(function() return string.gsub("abc", "%a", { a = true }) end))
end
