-- Tests for string.format

-- 1. Basic string formatting
assert(string.format("hello %s", "world") == "hello world")
assert(string.format("%s", 123) == "123")
assert(string.format("%s", true) == "true")
assert(string.format("%s", false) == "false")
assert(string.format("%s", nil) == "nil")
assert(string.format("%10s", "foo") == "       foo")
assert(string.format("%-10s", "foo") == "foo       ")
assert(string.format("%.3s", "abcdef") == "abc")
assert(string.format("%5.3s", "abcdef") == "  abc")
assert(string.format("%-5.3s", "abcdef") == "abc  ")
assert(string.format("%%") == "%")
assert(string.format("foo %% bar %s", "baz") == "foo % bar baz")
assert(string.format("plain literal string") == "plain literal string")
assert(string.format("") == "")

-- 2. Quoted strings (%q)
assert(string.format("%q", "simple") == "\"simple\"")
assert(string.format("%q", "with \"quotes\" and \\slash") == "\"with \\\"quotes\\\" and \\\\slash\"")
assert(string.format("%q", "line1\nline2\rline3\0zero") == "\"line1\\nline2\\rline3\\0zero\"")
assert(string.format("%q", nil) == "nil")
assert(string.format("%q", true) == "true")
assert(string.format("%q", false) == "false")
assert(string.format("%q", 12345) == "12345")
assert(string.format("%q", "UTF-8: 世界 🦀") == "\"UTF-8: 世界 🦀\"")

assert(string.format("%q", "hello world") == "\"hello world\"")
assert(string.format("%q", "hello\nworld\r\t\"quoted\"\\backslash") == "\"hello\\nworld\\r\\9\\\"quoted\\\"\\\\backslash\"")
assert(string.format("%q", "null\0byte") == "\"null\\0byte\"")
assert(string.format("%q", "null\000123digit") == "\"null\\000123digit\"")
assert(string.format("%q", "unicode: 你好世界") == "\"unicode: 你好世界\"")

-- 3. Characters (%c)
assert(string.format("%c", 65) == "A")
assert(string.format("%c", 97) == "a")
assert(string.format("%3c", 65) == "  A")
assert(string.format("%-3c", 65) == "A  ")
assert(string.format("%c%c%c", 76, 117, 97) == "Lua")

-- 4. Integers (%d, %i, %x, %X, %o, %u)
-- Extremes: math.maxinteger and math.mininteger
assert(string.format("%d", math.maxinteger) == "9223372036854775807")
assert(string.format("%d", math.mininteger) == "-9223372036854775808")
assert(string.format("%i", math.maxinteger) == "9223372036854775807")
assert(string.format("%i", math.mininteger) == "-9223372036854775808")

-- Basic integers, signs, and flags
assert(string.format("%d", 0) == "0")
assert(string.format("%d", 42) == "42")
assert(string.format("%d", -42) == "-42")
assert(string.format("%+d", 42) == "+42")
assert(string.format("%+d", -42) == "-42")
assert(string.format("% d", 42) == " 42")
assert(string.format("% d", -42) == "-42")

-- %u: unsigned integer
assert(string.format("%u", 0) == "0")
assert(string.format("%u", 42) == "42")
assert(string.format("%u", -1) == "18446744073709551615")
assert(string.format("%u", math.maxinteger) == "9223372036854775807")
assert(string.format("%u", math.mininteger) == "9223372036854775808")

-- %x and %X: hexadecimal
assert(string.format("%x", 0) == "0")
assert(string.format("%x", 255) == "ff")
assert(string.format("%X", 255) == "FF")
assert(string.format("%#x", 255) == "0xff")
assert(string.format("%#X", 255) == "0XFF")
assert(string.format("%#x", 0) == "0")
assert(string.format("%x", -1) == "ffffffffffffffff")
assert(string.format("%X", -1) == "FFFFFFFFFFFFFFFF")
assert(string.format("%x", math.mininteger) == "8000000000000000")
assert(string.format("%X", math.mininteger) == "8000000000000000")

-- %o: octal
assert(string.format("%o", 0) == "0")
assert(string.format("%o", 8) == "10")
assert(string.format("%#o", 8) == "010")
assert(string.format("%#o", 0) == "0")
assert(string.format("%o", -1) == "1777777777777777777777")

-- Integer padding, width, and precision
assert(string.format("%08d", 42) == "00000042")
assert(string.format("%08d", -42) == "-0000042")
assert(string.format("%-8d", 42) == "42      ")
assert(string.format("%-8d", -42) == "-42     ")
assert(string.format("%.5d", 42) == "00042")
assert(string.format("%.5d", -42) == "-00042")
assert(string.format("%8.5d", 42) == "   00042")
assert(string.format("%8.5d", -42) == "  -00042")
assert(string.format("%-8.5d", -42) == "-00042  ")
assert(string.format("%08.5d", -42) == "  -00042") -- precision overrides 0 flag
assert(string.format("%.0d", 0) == "")
assert(string.format("%.0d", 5) == "5")

-- 5. Floats (%f, %e, %E, %g, %G, %a, %A, nan, inf, -inf)
local nan = 0.0 / 0.0
local inf = 1.0 / 0.0
local neg_inf = -1.0 / 0.0

-- Special values
assert(string.format("%f", inf) == "inf")
assert(string.format("%f", neg_inf) == "-inf")
assert(string.format("%E", inf) == "INF")
assert(string.format("%E", neg_inf) == "-INF")
local nan_str = string.format("%f", nan)
assert(nan_str == "nan" or nan_str == "-nan")
local nan_upper = string.format("%E", nan)
assert(nan_upper == "NAN" or nan_upper == "-NAN")
assert(string.format("%08f", inf) == "     inf")
assert(string.format("%+8f", inf) == "    +inf")

-- %f decimal
assert(string.format("%.2f", 3.14159) == "3.14")
assert(string.format("%08.2f", 3.14) == "00003.14")
assert(string.format("%08.2f", -3.14) == "-0003.14")
assert(string.format("%-8.2f", 3.14) == "3.14    ")
assert(string.format("%+8.2f", 3.14) == "   +3.14")
assert(string.format("% 8.2f", 3.14) == "    3.14")
assert(string.format("%#.0f", 3.0) == "3.")

-- %e, %E scientific
assert(string.format("%.2e", 1.23) == "1.23e+00")
assert(string.format("%.2E", 1.23) == "1.23E+00")
assert(string.format("%.1e", 100.0) == "1.0e+02")
assert(string.format("%.1e", 0.01) == "1.0e-02")
assert(string.format("%#.0e", 1.23) == "1.e+00")

-- %g, %G compact
assert(string.format("%g", 1.23) == "1.23")
assert(string.format("%g", 0.000123) == "0.000123")
assert(string.format("%g", 0.0000123) == "1.23e-05")
assert(string.format("%G", 0.0000123) == "1.23E-05")
assert(string.format("%g", 1234567.0) == "1.23457e+06")
assert(string.format("%#g", 1.0) == "1.00000")

-- %a, %A hex float
assert(string.format("%a", 3.0) == "0x1.8p+1")
assert(string.format("%A", 3.0) == "0X1.8P+1")
assert(string.format("%a", 0.0) == "0x0p+0")
assert(string.format("%a", -1.0) == "-0x1p+0")
assert(string.format("%+a", 1.0) == "+0x1p+0")

-- 6. Pointers (%p)
local tbl = {}
local ptr = string.format("%p", tbl)
assert(type(ptr) == "string" and #ptr > 2)

-- 7. Coercion
assert(string.format("%d", "123") == "123")
assert(string.format("%.1f", "3.14") == "3.1")

-- 8. Error conditions
-- Missing arguments
assert(not pcall(string.format, "%d"))
assert(not pcall(string.format, "%s %s", "only one"))

-- Wrong types
assert(not pcall(string.format, "%d", true))
assert(not pcall(string.format, "%d", "not a number"))
assert(not pcall(string.format, "%f", {}))
assert(not pcall(string.format, "%c", "not an int"))

-- Invalid format options
assert(not pcall(string.format, "%z", 1))
assert(not pcall(string.format, "%"))
assert(not pcall(string.format, "%10"))
assert(not pcall(string.format, "%20q", "abc"))

-- Ridiculous width / precision (DOS protection)
assert(not pcall(string.format, "%99999s", "abc"))
assert(not pcall(string.format, "%.99999f", 1.0))
