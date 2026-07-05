-- if-else cases

print("test_if_else.lua")

local r = ""

if true then
    r = "if"
else
    r = "else"
end
print("true branch:", r)

if false then
    r = "if"
else
    r = "else"
end
print("false branch:", r)

if nil then
    r = "if"
else
    r = "else"
end
print("nil branch:", r)
