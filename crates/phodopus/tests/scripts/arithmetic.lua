
do
    assert(1 << 1 == 2)
    assert(9223372036854775807 << 0 == 9223372036854775807)
    assert((1 << 63) - 1 == 9223372036854775807)

    -- Shift right is logical (unsigned)
    assert(-1 >> 1 == 9223372036854775807)
end

do
    -- Check overflow cases
    assert(1 << 64 == 0)
    assert(-1 >> 64 == 0)

    assert(1 << (1 << 32) == 0)
    assert(1 >> (1 << 32) == 0)
end

do
    assert(0 ~= 1 + 1)
end

-- Impl bugs caught by PUC-Rio Lua's test suite:
do
    local i, j = -16, 3
    assert(i // j == math.floor(i / j))
end

-- Regression tests for Constant::modulo / float Mod / i64::MIN concat (#67):
do
    assert((-9223372036854775807 - 1) % -1 == 0)
    assert(9223372036854775806 % 9223372036854775807 == 9223372036854775806)
    local a, b = 9223372036854775806, 9223372036854775807
    assert(a % b == 9223372036854775806)
    assert(1e308 % 1.5e308 == 1e308)
    assert("1e308" % 1.5e308 == 1e308)
    assert(tostring((-9223372036854775807 - 1) .. "") == "-9223372036854775808")
end
