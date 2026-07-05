-- Basic if-then cases

print("test_if_basic.lua")

local r1 = false
if true then
    r1 = true
end
print("if true:", r1)

local r2 = "unchanged"
if false then
    r2 = "changed"
end
print("if false:", r2)

local r3 = false
if 0 then
    r3 = true
end
print("if 0:", r3)
