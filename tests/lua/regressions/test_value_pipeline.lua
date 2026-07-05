-- test_value_pipeline.lua
-- PR-4 ValueResult Pipeline regression tests
-- Tests literal materialization, name reads, paren expressions, RK encoding

-- 1. Literal types
local n = nil
local t = true
local f = false
local num = 42
local str = "hello"

assert(n == nil, "nil literal")
assert(t == true, "true literal")
assert(f == false, "false literal")
assert(num == 42, "number literal")
assert(str == "hello", "string literal")

-- 2. Name reads (local / global)
local a = 100
local b = a
assert(b == 100, "local name read")

g_val = 200
local c = g_val
assert(c == 200, "global name read")

-- 3. Paren expressions
local d = (a)
assert(d == 100, "paren local")

local e = ((a))
assert(e == 100, "double paren local")

-- 4. Paren call convergence: (f()) should return only first value
local function multi() return 1, 2, 3 end

local x = (multi())
assert(x == 1, "paren call first value")

local y, z = (multi())
assert(y == 1, "paren call y = 1")
assert(z == nil, "paren call z = nil (single value)")

-- 5. Function expression
local inc = function(v) return v + 1 end
assert(inc(10) == 11, "function expression")

-- 6. Arithmetic with constant folding / RK
local p = 10
local q = p + 1
assert(q == 11, "add with constant RK")

local r = p * 2
assert(r == 20, "mul with constant RK")

local s = 100 - p
assert(s == 90, "sub with constant RK")

-- 7. String constants
local s1 = "foo"
local s2 = "bar"
local s3 = s1 .. s2
assert(s3 == "foobar", "string concat")

-- 8. Mixed expressions in local init
local m1, m2, m3 = 1, "two", true
assert(m1 == 1, "multi local 1")
assert(m2 == "two", "multi local 2")
assert(m3 == true, "multi local 3")

-- 9. Excess variables get nil
local v1, v2, v3 = 10, 20
assert(v1 == 10, "excess vars v1")
assert(v2 == 20, "excess vars v2")
assert(v3 == nil, "excess vars v3 nil")

-- 10. Nested table access value reads
local tbl = {x = {y = 42}}
local val = tbl.x.y
assert(val == 42, "nested table read")

-- 11. Upvalue read
local outer = 99
local fn = function()
    return outer
end
assert(fn() == 99, "upvalue read")

print("PR-4 test_value_pipeline: ALL PASSED")
