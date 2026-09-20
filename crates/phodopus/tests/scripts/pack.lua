-- Comprehensive tests for binary packaging (string.pack, string.unpack, string.packsize)

-- 1. string.packsize tests
do
    -- Fixed integer sizes
    assert(string.packsize("b") == 1)
    assert(string.packsize("B") == 1)
    assert(string.packsize("h") == 2)
    assert(string.packsize("H") == 2)
    assert(string.packsize("l") == 8)
    assert(string.packsize("L") == 8)
    assert(string.packsize("j") == 8)
    assert(string.packsize("J") == 8)
    assert(string.packsize("T") == 8)

    -- Variable-width integers (default 4 bytes)
    assert(string.packsize("i") == 4)
    assert(string.packsize("I") == 4)
    for n = 1, 16 do
        assert(string.packsize("i" .. n) == n)
        assert(string.packsize("I" .. n) == n)
    end

    -- Floating point
    assert(string.packsize("f") == 4)
    assert(string.packsize("d") == 8)
    assert(string.packsize("n") == 8)

    -- Fixed strings
    assert(string.packsize("c0") == 0)
    assert(string.packsize("c1") == 1)
    assert(string.packsize("c5") == 5)
    assert(string.packsize("c100") == 100)

    -- Padding
    assert(string.packsize("x") == 1)
    assert(string.packsize("xxx") == 3)

    -- Endianness prefixes do not add size
    assert(string.packsize("<h") == 2)
    assert(string.packsize(">h") == 2)
    assert(string.packsize("=h") == 2)

    -- Combined formats
    assert(string.packsize("b h l") == 11)
    assert(string.packsize("<i2 i4 f d") == 2 + 4 + 4 + 8)

    -- Alignment with !4
    -- "b" at 0: size 1, pos 1
    -- "i4" at 1: align 4 -> 3 bytes pad, then 4 bytes -> pos 8
    assert(string.packsize("!4 b i4") == 8)
    assert(string.packsize("!4 b h") == 4) -- 1 + 1 pad + 2 = 4
    assert(string.packsize("!4 b b b b") == 4)

    -- Empty item alignment 'X'
    assert(string.packsize("!4 b Xi4") == 4) -- 1 + 3 pad = 4
    assert(string.packsize("!4 b Xh") == 2)  -- 1 + 1 pad = 2

    -- Variable-length options must error in packsize
    assert(not pcall(string.packsize, "z"))
    assert(not pcall(string.packsize, "s"))
    assert(not pcall(string.packsize, "s1"))
    assert(not pcall(string.packsize, "s2"))
    assert(not pcall(string.packsize, "s8"))
    assert(not pcall(string.packsize, "b z"))
    assert(not pcall(string.packsize, "s b"))

    -- Invalid formats
    assert(not pcall(string.packsize, "c"))   -- missing size for c
    assert(not pcall(string.packsize, "!3"))  -- not a power of 2
    assert(not pcall(string.packsize, "i17")) -- integral size out of range
    assert(not pcall(string.packsize, "invalid"))
end

-- 2. Basic integer pack and unpack
do
    -- Signed byte 'b' (-128 to 127)
    for _, val in ipairs({-128, -100, -1, 0, 1, 100, 127}) do
        local packed = string.pack("b", val)
        assert(#packed == 1)
        local unpacked, pos = string.unpack("b", packed)
        assert(unpacked == val and pos == 2)
    end

    -- Unsigned byte 'B' (0 to 255)
    for _, val in ipairs({0, 1, 128, 200, 255}) do
        local packed = string.pack("B", val)
        assert(#packed == 1)
        local unpacked, pos = string.unpack("B", packed)
        assert(unpacked == val and pos == 2)
    end

    -- Signed short 'h' (-32768 to 32767)
    for _, val in ipairs({-32768, -1234, -1, 0, 1, 1234, 32767}) do
        local packed = string.pack("<h", val)
        assert(#packed == 2)
        local unpacked, pos = string.unpack("<h", packed)
        assert(unpacked == val and pos == 3)
    end

    -- Unsigned short 'H' (0 to 65535)
    for _, val in ipairs({0, 1, 32768, 50000, 65535}) do
        local packed = string.pack("<H", val)
        assert(#packed == 2)
        local unpacked, pos = string.unpack("<H", packed)
        assert(unpacked == val and pos == 3)
    end

    -- 64-bit integers: 'l', 'L', 'j', 'J', 'T'
    for _, val in ipairs({-9223372036854775808, -1, 0, 1, 9223372036854775807}) do
        local packed = string.pack("<l", val)
        assert(#packed == 8)
        local unpacked, pos = string.unpack("<l", packed)
        assert(unpacked == val and pos == 9)

        local packed_j = string.pack("<j", val)
        assert(#packed_j == 8)
        local unpacked_j, pos_j = string.unpack("<j", packed_j)
        assert(unpacked_j == val and pos_j == 9)
    end

    for _, val in ipairs({0, 1, 0x12345678, 0x7FFFFFFFFFFFFFFF, -1}) do
        local packed_L = string.pack("<L", val)
        assert(#packed_L == 8)
        local unpacked_L, pos_L = string.unpack("<L", packed_L)
        assert(unpacked_L == val and pos_L == 9)

        local packed_J = string.pack("<J", val)
        assert(#packed_J == 8)
        local unpacked_J, pos_J = string.unpack("<J", packed_J)
        assert(unpacked_J == val and pos_J == 9)

        local packed_T = string.pack("<T", val)
        assert(#packed_T == 8)
        local unpacked_T, pos_T = string.unpack("<T", packed_T)
        assert(unpacked_T == val and pos_T == 9)
    end

    -- Variable-width integers: 'i1'..'i8' and 'I1'..'I8'
    local signed_test_vals = {
        [1] = -120,
        [2] = -30000,
        [3] = -800000,
        [4] = -2000000000,
        [5] = -500000000000,
        [6] = -100000000000000,
        [7] = -20000000000000000,
        [8] = -9000000000000000000,
    }
    for size = 1, 8 do
        local fmt = "<i" .. size
        local val = signed_test_vals[size]
        local packed = string.pack(fmt, val)
        assert(#packed == size)
        local unpacked, pos = string.unpack(fmt, packed)
        assert(unpacked == val and pos == size + 1)
    end

    local unsigned_test_vals = {
        [1] = 250,
        [2] = 60000,
        [3] = 16000000,
        [4] = 4000000000,
        [5] = 1000000000000,
        [6] = 200000000000000,
        [7] = 50000000000000000,
        [8] = 0x7FFFFFFFFFFFFFFF,
    }
    for size = 1, 8 do
        local fmt = "<I" .. size
        local val = unsigned_test_vals[size]
        local packed = string.pack(fmt, val)
        assert(#packed == size)
        local unpacked, pos = string.unpack(fmt, packed)
        assert(unpacked == val and pos == size + 1)
    end
end

-- 3. Endianness verification
do
    -- Little endian: least significant byte first
    assert(string.pack("<I2", 0x1234) == "\x34\x12")
    assert(string.pack(">I2", 0x1234) == "\x12\x34")

    assert(string.pack("<i4", 0x12345678) == "\x78\x56\x34\x12")
    assert(string.pack(">i4", 0x12345678) == "\x12\x34\x56\x78")

    assert(string.pack("<I4", 0xAABBCCDD) == "\xDD\xCC\xBB\xAA")
    assert(string.pack(">I4", 0xAABBCCDD) == "\xAA\xBB\xCC\xDD")

    -- Unpack according to endianness
    assert(string.unpack("<I2", "\x34\x12") == 0x1234)
    assert(string.unpack(">I2", "\x12\x34") == 0x1234)
    assert(string.unpack("<i4", "\x78\x56\x34\x12") == 0x12345678)
    assert(string.unpack(">i4", "\x12\x34\x56\x78") == 0x12345678)

    -- Mixed endianness in single format string
    local mixed = string.pack("<I2 >I2", 0x1234, 0x1234)
    assert(mixed == "\x34\x12\x12\x34")
    local v1, v2 = string.unpack("<I2 >I2", mixed)
    assert(v1 == 0x1234 and v2 == 0x1234)
end

-- 4. Floating point tests
do
    -- Single precision float 'f' (4 bytes)
    local float_val = 3.140625
    local packed_f = string.pack("<f", float_val)
    assert(#packed_f == 4)
    local unpacked_f = string.unpack("<f", packed_f)
    assert(math.abs(unpacked_f - float_val) < 1e-6)

    -- Double precision float 'd' / 'n' (8 bytes)
    local double_val = 123456.78901234
    local packed_d = string.pack(">d", double_val)
    assert(#packed_d == 8)
    local unpacked_d = string.unpack(">d", packed_d)
    assert(unpacked_d == double_val)

    local packed_n = string.pack("<n", double_val)
    assert(#packed_n == 8)
    local unpacked_n = string.unpack("<n", packed_n)
    assert(unpacked_n == double_val)
end

-- 5. String types: 'c[n]', 'z', 's[n]'
do
    -- Fixed strings 'c[n]'
    local s5 = string.pack("c5", "hello")
    assert(s5 == "hello")
    local u5, pos5 = string.unpack("c5", s5)
    assert(u5 == "hello" and pos5 == 6)

    -- Zero-padded if shorter than n
    local s_short = string.pack("c5", "hi")
    assert(s_short == "hi\0\0\0")
    local u_short = string.unpack("c5", s_short)
    assert(u_short == "hi\0\0\0")

    -- c0 empty string
    assert(string.pack("c0", "") == "")
    assert(string.unpack("c0", "abc") == "")

    -- Zero-terminated string 'z'
    local z_packed = string.pack("z", "hello")
    assert(z_packed == "hello\0")
    local z_val, z_pos = string.unpack("z", z_packed)
    assert(z_val == "hello" and z_pos == 7)

    -- Multiple zero-terminated strings
    local z_multi = string.pack("z z", "foo", "bar")
    assert(z_multi == "foo\0bar\0")
    local z1, z2, z_end = string.unpack("z z", z_multi)
    assert(z1 == "foo" and z2 == "bar" and z_end == 9)

    -- Unpack 'z' with initial position
    local z_part, z_part_pos = string.unpack("z", z_multi, 5)
    assert(z_part == "bar" and z_part_pos == 9)

    -- Length-prefixed string 's' (default 8-byte length)
    local s_packed = string.pack("s", "bitty")
    local s_val, s_next = string.unpack("s", s_packed)
    assert(s_val == "bitty" and s_next == #s_packed + 1)

    -- 's1': 1-byte length prefix
    local s1_packed = string.pack("s1", "cat")
    assert(s1_packed == "\x03cat")
    local s1_val, s1_next = string.unpack("s1", s1_packed)
    assert(s1_val == "cat" and s1_next == 5)

    -- 's2': 2-byte length prefix
    local s2_packed = string.pack(">s2", "fox")
    assert(s2_packed == "\x00\x03fox")
    local s2_val, s2_next = string.unpack(">s2", s2_packed)
    assert(s2_val == "fox" and s2_next == 6)
end

-- 6. Padding and Alignment: 'x', '!n', 'Xop'
do
    -- 'x' writes zero byte
    local x_packed = string.pack("x b x", 42)
    assert(x_packed == "\0\x2a\0")
    local x_val, x_pos = string.unpack("x b x", x_packed)
    assert(x_val == 42 and x_pos == 4)

    -- '!4' alignment
    -- 'b' at offset 0: 1 byte
    -- '!4 i4' at offset 1: aligns to 4 bytes -> 3 pad bytes + 4 data bytes = 8 bytes
    local aligned_packed = string.pack("!4 b i4", 1, 0x12345678)
    assert(#aligned_packed == 8)
    local av1, av2, apos = string.unpack("!4 b i4", aligned_packed)
    assert(av1 == 1 and av2 == 0x12345678 and apos == 9)

    -- 'X' aligns without consuming or producing an argument
    local x_align = string.pack("!4 b Xi4", 1)
    assert(#x_align == 4) -- 1 data byte + 3 pad bytes
    local xv, xpos = string.unpack("!4 b Xi4", x_align)
    assert(xv == 1 and xpos == 5)
end

-- 7. Complex round-trip test
do
    local fmt = "<!4 i2 I4 f d c6 z s1"
    local v_i2 = -1234
    local v_I4 = 987654321
    local v_f = 2.5
    local v_d = 1234567.890123
    local v_c6 = "phodop"
    local v_z = "zero_ended"
    local v_s1 = "len_prefixed"

    local packed = string.pack(fmt, v_i2, v_I4, v_f, v_d, v_c6, v_z, v_s1)
    local r_i2, r_I4, r_f, r_d, r_c6, r_z, r_s1, r_pos = string.unpack(fmt, packed)

    assert(r_i2 == v_i2)
    assert(r_I4 == v_I4)
    assert(r_f == v_f)
    assert(r_d == v_d)
    assert(r_c6 == v_c6)
    assert(r_z == v_z)
    assert(r_s1 == v_s1)
    assert(r_pos == #packed + 1)
end

-- 8. Boundary conditions and graceful error handling
do
    -- pack: integer overflow
    assert(not pcall(string.pack, "b", 128))
    assert(not pcall(string.pack, "b", -129))
    assert(not pcall(string.pack, "B", -1))
    assert(not pcall(string.pack, "B", 256))
    assert(not pcall(string.pack, "<h", 32768))
    assert(not pcall(string.pack, "<h", -32769))
    assert(not pcall(string.pack, "<H", -1))
    assert(not pcall(string.pack, "<H", 65536))
    assert(not pcall(string.pack, "<i1", 128))
    assert(not pcall(string.pack, "<I1", -1))
    assert(not pcall(string.pack, "<I2", 65536))

    -- pack: string longer than 'c[n]'
    assert(not pcall(string.pack, "c3", "toolong"))

    -- pack: string containing zeros for 'z'
    assert(not pcall(string.pack, "z", "bad\0string"))

    -- pack: missing arguments
    assert(not pcall(string.pack, "i4"))
    assert(not pcall(string.pack, "i4 i4", 10))

    -- pack: invalid format strings
    assert(not pcall(string.pack, "k", 1))
    assert(not pcall(string.pack, "!3", 1))
    assert(not pcall(string.pack, "i17", 1))
    assert(not pcall(string.pack, "c", "abc"))
    assert(not pcall(string.pack, "X"))

    -- unpack: data string too short
    assert(not pcall(string.unpack, "i4", "ab"))
    assert(not pcall(string.unpack, "c5", "abc"))
    assert(not pcall(string.unpack, "z", "unterminated"))
    assert(not pcall(string.unpack, "s1", "\x05abc"))

    -- unpack: initial position out of bounds
    assert(not pcall(string.unpack, "b", "abc", 10))
    assert(not pcall(string.unpack, "b", "abc", 0))
    assert(not pcall(string.unpack, "b", "abc", -10))

    -- unpack: negative initial position (1-based from end)
    local s = "abcdef"
    local val, next_pos = string.unpack("c2", s, -2) -- last 2 bytes: "ef"
    assert(val == "ef" and next_pos == 7)

    -- pack & packsize: 16 MiB allocation limit
    assert(not pcall(string.packsize, "c17000000"))
    assert(not pcall(string.pack, "c17000000", "a"))

    -- unpack: integer overflow for sizes > 8
    local s_overflow_u = "\x00\x00\x00\x00\x00\x00\x00\x00\x01" -- 1 << 64
    assert(not pcall(string.unpack, "<I9", s_overflow_u))

    local s_overflow_s = "\x00\x00\x00\x00\x00\x00\x00\x80\x00" -- 0x8000000000000000 positive
    assert(not pcall(string.unpack, "<i9", s_overflow_s))

    -- empty and whitespace formats
    assert(string.packsize("") == 0)
    assert(string.pack("") == "")
    local empty_pos = string.unpack("", "abc")
    assert(empty_pos == 1)

    local ws_packed = string.pack("  <  i4 \t \n b  ", 1234, 5)
    local ws_v1, ws_v2, ws_next = string.unpack("  <  i4 \t \n b  ", ws_packed)
    assert(ws_v1 == 1234 and ws_v2 == 5 and ws_next == #ws_packed + 1)
end

-- 9. OOP method calling syntax via string metatable
do
    assert(("b"):packsize() == 1)
    assert((">I2"):packsize() == 2)
    assert(("!4 b i4"):packsize() == 8)

    local packed = (">I2"):pack(0x1234)
    assert(packed == "\x12\x34")

    local unpacked, pos = (">I2"):unpack(packed)
    assert(unpacked == 0x1234 and pos == 3)
end

