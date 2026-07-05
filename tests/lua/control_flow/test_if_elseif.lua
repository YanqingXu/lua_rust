-- if-elseif-else cases

print("test_if_elseif.lua")

local x = 1
local result = ""

if x == 1 then
    result = "one"
elseif x == 2 then
    result = "two"
else
    result = "other"
end
print("x=1:", result)

x = 2
if x == 1 then
    result = "one"
elseif x == 2 then
    result = "two"
else
    result = "other"
end
print("x=2:", result)

x = 9
if x == 1 then
    result = "one"
elseif x == 2 then
    result = "two"
else
    result = "other"
end
print("x=9:", result)
