-- Project C++ contracts: stock Lua rejects these during parsing with different
-- wording. Invalid chunks are compiled only; never call an unexpected function.
local cases = {
  {"do", "do 1=2 end"},
  {"if", "if false then 1=2 end"},
  {"elseif", "if true then elseif false then 1=2 end"},
  {"else", "if true then else 1=2 end"},
  {"while", "while false do 1=2 end"},
  {"repeat", "repeat 1=2 until true"},
  {"numeric-for", "for i=1,0 do 1=2 end"},
  {"generic-for", "for k in next,{} do 1=2 end"},
  {"global-function", "function f() do 1=2 end end"},
  {"local-function", "local function f() do 1=2 end end"},
  {"returned-function", "return function() do 1=2 end end"},
  {"function-expression", "local f=function() if false then 1=2 end end"},
  {"call-argument", "f(function() do 1=2 end end)"},
  {"table-field", "local t={function() do 1=2 end end}"},
  {"deep-function", "local function outer() return function() if false then 1=2 end end end"},
  {"direct-before-break", "do 1=2 end; break"},
  {"break-before-direct", "do break end; 1=2", "no loop to break"},
  {"function-direct-before-break", "local function f() do 1=2 end end; do break end"},
}

local valid_source = [=[
local funcs = {}
do
  local captured = 42
  funcs[1] = function(delta) captured = captured + delta; return captured end
end
local sum = 0
if false then error("wrong branch")
elseif true then sum = sum + 1
else error("wrong else") end
local i = 0
while true do
  i = i + 1
  if i == 3 then break end
  sum = sum + i
end
repeat sum = sum + 1 until sum == 6
for n = 1, 3 do
  if n == 3 then break end
  sum = sum + n
end
for _, n in ipairs({2, 3, 99}) do
  if n == 99 then break end
  sum = sum + n
end
local function outer(seed)
  return function(delta)
    seed = seed + delta
    return function() return seed end
  end
end
local nested = outer(sum)
local a, b = nested(1), nested(2)
collectgarbage("collect")
return funcs[1], a, b, sum
]=]

nested_compile_side_effect = 0
for _, case in ipairs(cases) do
  local source = "nested_compile_side_effect = nested_compile_side_effect + 1; " .. case[2]
  local bad, diagnostic = loadstring(source, "=nested-invalid")
  assert(bad == nil, case[1] .. ": unexpectedly returned an executable function")
  local expected = "loadstring: " .. (case[3] or "Expression is not a valid lvalue")
  assert(diagnostic == expected, case[1] .. ": wrong first diagnostic: " .. tostring(diagnostic))
  assert(nested_compile_side_effect == 0, case[1] .. ": invalid source executed")

  -- A successful compilation/execution follows every rejected compilation in
  -- the same Runtime. Keep escaping closures alive across explicit collection.
  collectgarbage("collect")
  local good, error_message = loadstring(valid_source, "=nested-valid")
  assert(good, error_message)
  local mutate, a, b, sum = good()
  good = nil
  collectgarbage("collect")
  assert(sum == 14 and mutate(0) == 42 and mutate(2) == 44)
  assert(a() == 17 and b() == 17)
end
collectgarbage("collect")
assert(nested_compile_side_effect == 0)
print("nested compilation contracts passed")
