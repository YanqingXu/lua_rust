-- PR-3 LValue Pipeline regression test
-- Covers: local, global, upvalue, t[k], obj.x, multi-assign, multi-return

print("=== LValue Pipeline Regression ===")

-- 1. Basic local assignment
local x = 0
x = 42
assert(x == 42, "local assignment")

-- 2. Global assignment
g_val = 99
assert(g_val == 99, "global assignment")

-- 3. Upvalue write-back
local uv = 0
local function set_uv(v) uv = v end
set_uv(77)
assert(uv == 77, "upvalue write-back")

-- 4. Table index assignment (string key)
local t = {}
local key = "abc"
t[key] = 55
assert(t.abc == 55, "string key index")

-- 5. Table index assignment (numeric key)
t[1] = 100
assert(t[1] == 100, "numeric key index")

-- 6. Member assignment
local obj = {}
obj.x = 10
obj.y = 20
assert(obj.x == 10, "member x")
assert(obj.y == 20, "member y")

-- 7. Nested table write
local a = { b = { c = {} } }
a.b.c.d = 42
assert(a.b.c.d == 42, "nested member deep write")

local dk = "dynamic"
a.b[dk] = 88
assert(a.b.dynamic == 88, "nested indexed write")

-- 8. Self-increment: obj.x = obj.x + 1
obj.x = obj.x + 1
assert(obj.x == 11, "self increment")

-- 9. Mixed multi-assign
g_val = 0
local holder = {}
local loc
loc, g_val, holder.answer = 10, 20, 30
assert(loc == 10, "mixed local")
assert(g_val == 20, "mixed global")
assert(holder.answer == 30, "mixed member")

-- 10. Multi-return into mixed targets
local function triple() return 7, 8, 9 end

g_slot = 0
holder.answer = 0
local local_slot
local_slot, g_slot, holder.answer = triple()
assert(local_slot == 7, "multiret local")
assert(g_slot == 8, "multiret global")
assert(holder.answer == 9, "multiret member")

-- 11. t[k], x = f()
local function pair() return "alpha", "beta" end
local tbl = {}
tbl.x, g_y = pair()
assert(tbl.x == "alpha", "t[k] gets first return")
assert(g_y == "beta", "global gets second return")

-- 12. Excess variables get nil
g1, g2, g3 = 10
assert(g1 == 10, "first global")
assert(g2 == nil, "second global nil")
assert(g3 == nil, "third global nil")

-- 13. Multiple table index targets
local mt = {}
mt[1], mt[2], mt[3] = 10, 20, 30
assert(mt[1] == 10, "mt[1]")
assert(mt[2] == 20, "mt[2]")
assert(mt[3] == 30, "mt[3]")

print("LValue pipeline regression passed")
