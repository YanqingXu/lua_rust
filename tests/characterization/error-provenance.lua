-- Fixed C++ protected-call/resume provenance contract. Lua 5.1.5 differs on
-- bare coroutine.resume diagnostics and long chunk-name presentation.
local function check(body, chunk, located, raw)
  local f = assert(loadstring(body, chunk))
  local ok, err = pcall(f)
  assert(not ok and err == located, tostring(err))
  local ok, err = xpcall(f, function(value) return 'handled:' .. value end)
  assert(not ok and err == 'handled:' .. located, tostring(err))
  local co = coroutine.create(function()
    local ok, err = pcall(f)
    assert(not ok and err == located, tostring(err))
    return 'continued'
  end)
  local ok, result = coroutine.resume(co)
  assert(ok and result == 'continued', tostring(result))
  local ok, err = coroutine.resume(coroutine.create(f))
  assert(not ok and err == raw, tostring(err))
end

local arithmetic = "attempt to perform arithmetic on local 't' (a table value)"
check('local t={}\nreturn t+1', '=named', 'named:2: ' .. arithmetic, arithmetic)
check('local t={}\nreturn -t', '@dir/file.lua', 'dir/file.lua:2: ' .. arithmetic, arithmetic)
check('local t={}\nreturn t+1', 'source-text', '[string "source-text"]:2: ' .. arithmetic, arithmetic)
local long_name = string.rep('directory/', 9) .. 'file.lua'
check('local t={}\nreturn t+1', '@' .. long_name, long_name .. ':2: ' .. arithmetic, arithmetic)
local long_source = string.rep('source ', 15)
local long_chunk = '[string "' .. string.sub(long_source, 1, 60) .. '..."]'
check('local t={}\nreturn t+1', long_source, long_chunk .. ':2: ' .. arithmetic, arithmetic)
check('local function f()\n local t={}\n return t+1\nend\nf()', '=nested', 'nested:3: ' .. arithmetic, arithmetic)

local char_error = "bad argument #1 to 'string.char' (value out of range)"
check('local value=string.char(256)\nreturn value', '=native', 'native:1: ' .. char_error, char_error)
check('return string.char(256)', '=native-tail', 'native-tail:1: ' .. char_error, char_error)
local direct = assert(loadstring('local ok,err=pcall(string.char,256)\nreturn ok,err', '=native-direct'))
local ok, err = direct()
assert(not ok and err == 'native-direct:1: ' .. char_error, tostring(err))
local ok, inner_ok, err = coroutine.resume(coroutine.create(direct))
assert(ok and not inner_ok and err == 'native-direct:1: ' .. char_error, tostring(err))

-- A diagnostic caught inside a suspended callback retains its class and the
-- callback's actual Lua caller location across Runtime native frames.
local suspended = assert(loadstring('return string.gsub("a", ".", function()\nlocal ok,err=pcall(string.char,256)\nassert(not ok and err == "native-suspended:2: bad argument #1 to \'string.char\' (value out of range)", tostring(err))\nreturn "b" end)', '=native-suspended'))
assert(suspended() == 'b')

-- Existing continuation-generated errors must still propagate as failures.
local callback = assert(loadstring('return string.gsub("a", ".", function() return {} end)', '=native-resumed'))
local ok, err = pcall(callback)
assert(not ok and type(err) == 'string')
assert(string.find(err, 'invalid replacement value', 1, true), err)

print('error provenance passed')
