print("=== LValue Matrix Regression ===")

g_value = 0
holder = {}

local local_value
local_value, g_value, holder.answer = 10, 20, 30

assert(local_value == 10, "local target should receive first value")
assert(g_value == 20, "global target should receive second value")
assert(holder.answer == 30, "member target should receive third value")

local key = "dynamic"
holder[key] = 99
assert(holder.dynamic == 99, "indexed target should write through dynamic key")

holder.inner = {}
holder.inner.answer = 42
assert(holder.inner.answer == 42, "nested member assignment should write through")

local innerKey = "deep"
holder.inner[innerKey] = 64
assert(holder.inner.deep == 64, "nested indexed assignment should write through")

local function triple()
    return 7, 8, 9
end

g_slot = 0
holder.answer = 0
local local_slot
local_slot, g_slot, holder.answer = triple()

assert(local_slot == 7, "local mixed target should receive first return value")
assert(g_slot == 8, "global mixed target should receive second return value")
assert(holder.answer == 9, "table mixed target should receive third return value")

print("LValue matrix regression passed")
