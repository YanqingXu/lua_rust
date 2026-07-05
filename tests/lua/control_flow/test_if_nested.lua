-- Nested if cases

print("test_if_nested.lua")

local a = true
local b = true
local result = ""

if a then
    if b then
        result = "both_true"
    else
        result = "a_only"
    end
else
    result = "a_false"
end
print("a=true,b=true:", result)

a = true
b = false
if a then
    if b then
        result = "both_true"
    else
        result = "a_only"
    end
else
    result = "a_false"
end
print("a=true,b=false:", result)

a = false
b = true
if a then
    if b then
        result = "both_true"
    else
        result = "a_only"
    end
else
    result = "a_false"
end
print("a=false,b=true:", result)
