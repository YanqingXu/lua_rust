-- Truthy and falsy edge cases in Lua

print("test_if_truthy.lua")

local r1 = ""
if nil then
    r1 = "if"
else
    r1 = "else"
end
print("nil:", r1)

local r2 = ""
if false then
    r2 = "if"
else
    r2 = "else"
end
print("false:", r2)

local r3 = false
if "" then
    r3 = true
end
print("empty string:", r3)

local r4 = false
if {} then
    r4 = true
end
print("empty table:", r4)
