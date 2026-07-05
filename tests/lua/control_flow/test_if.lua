-- test_if.lua
-- Short smoke test for if statements.
-- The full scenarios were split into smaller files:
--   test_if_basic.lua
--   test_if_else.lua
--   test_if_elseif.lua
--   test_if_nested.lua
--   test_if_logic.lua
--   test_if_truthy.lua

print("test_if.lua: smoke test")

local value = ""

if true then
    value = "if"
else
    value = "else"
end

print("case1:", value)

if false then
    value = "wrong"
else
    value = "else"
end

print("case2:", value)

local x = 2
if x == 1 then
    value = "one"
elseif x == 2 then
    value = "two"
else
    value = "other"
end

print("case3:", value)
