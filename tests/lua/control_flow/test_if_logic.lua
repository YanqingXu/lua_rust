-- Logical operators inside if conditions

print("test_if_logic.lua")

local r1 = false
if true and true then
    r1 = true
end
print("true and true:", r1)

local r2 = false
if false or true then
    r2 = true
end
print("false or true:", r2)

local r3 = false
if not false then
    r3 = true
end
print("not false:", r3)

local p = true
local q = false
local r4 = false
if (p or q) and (not q) then
    r4 = true
end
print("(p or q) and not q:", r4)

local r5 = false
if not (p and q) then
    r5 = true
end
print("not (p and q):", r5)
