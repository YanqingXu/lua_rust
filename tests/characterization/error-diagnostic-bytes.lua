-- Shared byte/type contracts of pinned C++ and official Lua 5.1.5.
-- Diagnostic wording differs between the oracles. The runner provides an
-- isolated working directory for the loadfile/dofile input below.
local ff = string.char(255)
local replacement = string.char(239, 191, 189)
local literal = "\\xff"

local function contains(value, part)
  return string.find(value, part, 1, true) ~= nil
end

local function byte_count(value, byte)
  local count = 0
  for i = 1, #value do
    if string.byte(value, i) == byte then count = count + 1 end
  end
  return count
end

local function diagnostic(value, count, has_literal)
  assert(type(value) == 'string')
  assert(byte_count(value, 255) == count, 'raw FF count changed: ' .. value)
  assert(not contains(value, replacement), 'replacement character in diagnostic')
  assert(contains(value, literal) == not not has_literal, 'literal escape changed')
  assert(not contains(value, '\\xFF'), 'uppercase escape in diagnostic')
  return value
end

local function syntax(source, name, count, has_literal)
  local f, err = loadstring(source, name)
  assert(f == nil)
  return diagnostic(err, count, has_literal)
end

local err = syntax(ff .. 'a = 1', '=byte-probe', 1)
assert(contains(err, 'byte-probe:1:') and contains(err, "near '" .. ff .. "'"))
syntax(ff .. 'a = 1', nil, 2)
syntax("'" .. ff .. "'", '=string-token', 1)
syntax('local a' .. ff .. '=1', '=identifier-byte', 1)
for _, name in ipairs({'=name-' .. ff, '@file-' .. ff, 'source-' .. ff}) do
  syntax('?', name, 1)
end

-- A real ASCII backslash-x sequence must not become a byte FF, including
-- when the same diagnostic also contains a genuine FF elsewhere.
syntax('?', '=literal-' .. literal, 0, true)
syntax('?', '=literal-' .. literal .. '-' .. ff, 1, true)
syntax('[[' .. literal .. ']]', '=literal-token', 0, true)
assert(contains(syntax('syntax error', '=identifier', 0), "near 'error'"))
assert(contains(syntax('1.000', '=number', 0), "near '1.000'"))

local chunks = {'local value = ', '1\n', ff, 'a = 1'}
local index = 0
local f, reader_err = load(function()
  index = index + 1
  return chunks[index]
end, '=reader')
assert(f == nil)
diagnostic(reader_err, 1)

local path = 'error-diagnostic-input.lua'
local file = assert(io.open(path, 'wb'))
assert(file:write(ff .. 'a = 1\n'))
assert(file:close())
local loaded, load_err = loadfile(path)
assert(loaded == nil)
diagnostic(load_err, 1)
local ok, do_err = pcall(dofile, path)
assert(not ok)
diagnostic(do_err, 1)
assert(os.remove(path))

for _, source in ipairs({
  'local t={}; return t["\\255"]()',
  'local t={}; return t["\\255"].x',
  'local t={}; return t["\\255"]+1',
}) do
  local field_operation = assert(loadstring(source, '=field'))
  ok, err = pcall(field_operation)
  assert(not ok)
  diagnostic(err, 1)
  assert(contains(err, "field '" .. ff .. "'"))
end

ok, err = pcall(function() error(ff, 0) end)
assert(not ok and err == ff)
ok, err = pcall(function() error(literal, 0) end)
assert(not ok and err == literal)
local located = assert(loadstring('error(string.char(255))', '=error-' .. ff))
ok, err = pcall(located)
assert(not ok)
diagnostic(err, 2)
assert(err == 'error-' .. ff .. ':1: ' .. ff)
local located_literal = assert(loadstring('error("\\\\xff")', '=literal-' .. literal))
ok, err = pcall(located_literal)
assert(not ok and err == 'literal-' .. literal .. ':1: ' .. literal)
diagnostic(err, 0, true)

ok, err = pcall(function() error(23, 0) end)
assert(not ok and type(err) == 'number' and err == 23)
local located_number = assert(loadstring('error(23)', '=number-error'))
ok, err = pcall(located_number)
assert(not ok and type(err) == 'string' and err == 'number-error:1: 23')
local object = {marker = 17}
for _, level in ipairs({0, 1}) do
  ok, err = pcall(function() error(object, level) end)
  assert(not ok and rawequal(err, object) and err.marker == 17)
end
ok, err = pcall(function() error(object) end)
assert(not ok and rawequal(err, object))
print('error diagnostic bytes passed')
