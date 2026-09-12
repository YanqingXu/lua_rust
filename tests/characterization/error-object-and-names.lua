-- Contracts on which the pinned C++ and official Lua 5.1.5 agree.
local function check(body, expected)
  local f = assert(loadstring(body, '=objects'))
  local ok, err = pcall(f)
  assert(not ok and err == expected, tostring(err))
  local ok, err = coroutine.resume(coroutine.create(f))
  assert(not ok and err == expected, tostring(err))
  local ok, err = xpcall(f, function(value) return value end)
  assert(not ok and err == expected, tostring(err))
end
check('error("bad argument fake",0)', 'bad argument fake')
check('error("old.lua:27: existing",0)', 'old.lua:27: existing')
check('error("message",1)', 'objects:1: message')
check('return error("bad argument fake",0)', 'bad argument fake')
check('error("bad argument" .. string.char(0,255),0)', 'bad argument' .. string.char(0,255))

local token = {}
local function same_object(f)
  local ok, err = pcall(f)
  assert(not ok and rawequal(err, token))
  local ok, err = coroutine.resume(coroutine.create(f))
  assert(not ok and rawequal(err, token))
end
same_object(function() error(token,0) end)
same_object(function()
  local t=setmetatable({}, {__add=function() error(token,0) end})
  return t+1
end)
same_object(function() return string.gsub('a', '.', function() error(token,0) end) end)
for _, value in ipairs({false, 123, 'bad argument #1 to fake'}) do
  local ok, err = pcall(function() error(value,0) end)
  assert(not ok and rawequal(err, value))
end
local ok, err = pcall(function() error(nil,0) end)
assert(not ok and err == nil)

local cases = {
  {[[prefix=nil; insert=nil; while 1 do local a; if nil then break end; insert(prefix,a) end]], "attempt to call global 'insert' (a nil value)"},
  {[[aaa={}; (aaa or aaa)()]], 'attempt to call a table value'},
  {[[aaa={}; x=(aaa or aaa)+(aaa and aaa)]], 'attempt to perform arithmetic on a table value'},
  {[[missing=nil; if false then return end; missing()]], "attempt to call global 'missing' (a nil value)"},
  {[[missing=nil; while true do local a=false; if a then break end; missing() end]], "attempt to call global 'missing' (a nil value)"},
  {[[missing=nil; if true then missing() end]], "attempt to call global 'missing' (a nil value)"},
  {[[missing=nil; repeat missing() until true]], "attempt to call global 'missing' (a nil value)"},
  {[[missing=nil; for i=1,1 do missing() end]], "attempt to call global 'missing' (a nil value)"},
  {[[local missing=nil; return missing()]], "attempt to call local 'missing' (a nil value)"},
  {[[local t={}; t()]], "attempt to call local 't' (a table value)"},
}
for _, case in ipairs(cases) do
  local ok, err=pcall(assert(loadstring(case[1], '=names')))
  assert(not ok and err == 'names:1: ' .. case[2], tostring(err))
end
print('error object and names passed')
