local function expect_error(message, fn, ...)
    local ok, err = pcall(fn, ...)
    assert(not ok, message .. ': unexpectedly succeeded')
    assert(type(err) == 'string', message .. ': error object is ' .. type(err))
    assert(string.sub(err, -#message) == message, err)
end

local function argument(index, name, detail)
    return 'bad argument #' .. index .. " to 'string." .. name .. "' (" .. detail .. ')'
end

for _, name in ipairs({'len', 'byte', 'upper', 'lower', 'reverse'}) do
    expect_error('string.' .. name .. ': missing argument', string[name])
    expect_error(argument(1, name, 'string expected'), string[name], {})
    expect_error(argument(1, name, 'string expected'), string[name], nil)
end
for _, name in ipairs({'find', 'match', 'gmatch', 'sub', 'rep'}) do
    expect_error('string.' .. name .. ': missing arguments', string[name])
    expect_error(argument(1, name, 'string expected'), string[name], {}, 1)
end
for _, name in ipairs({'find', 'match', 'gmatch'}) do
    expect_error(argument(2, name, 'string expected'), string[name], 'abc', {})
end

for _, value in ipairs({{}, false, function() end, 'not a number'}) do
    for _, name in ipairs({'find', 'match'}) do
        expect_error(argument(3, name, 'number expected'), string[name], 'abc', 'a', value)
    end
    expect_error(argument(2, 'byte', 'number expected'), string.byte, 'abc', value)
    expect_error(argument(3, 'byte', 'number expected'), string.byte, 'abc', 1, value)
    expect_error(argument(2, 'sub', 'number expected'), string.sub, 'abc', value)
    expect_error(argument(3, 'sub', 'number expected'), string.sub, 'abc', 1, value)
    expect_error(argument(2, 'rep', 'number expected'), string.rep, 'abc', value)
    expect_error(argument(4, 'gsub', 'number expected'), string.gsub, 'abc', 'a', 'x', value)
    expect_error(argument(1, 'char', 'number expected'), string.char, value)
end

-- Explicit nil is an error in the pinned C++ numeric argument contract;
-- omitted optional arguments default. Official Lua 5.1 differs here.
expect_error(argument(2, 'byte', 'number expected'), string.byte, 'abc', nil)
expect_error(argument(3, 'find', 'number expected'), string.find, 'abc', 'a', nil)
expect_error(argument(3, 'sub', 'number expected'), string.sub, 'abc', 1, nil)
expect_error(argument(4, 'gsub', 'number expected'), string.gsub, 'a', 'a', 'x', nil)
assert(string.byte('abc') == 97 and string.sub('abc', 2) == 'bc')
assert(string.find('abc', 'a') == 1 and string.match('abc', 'a') == 'a')
assert(string.gsub('aba', 'a', 'x') == 'xbx')

for _, value in ipairs({math.huge, -math.huge, math.sqrt(-1), 1e300, -1e300}) do
    local detail = 'finite number expected'
    if value == value and value ~= math.huge and value ~= -math.huge then detail = 'number out of range' end
    expect_error(argument(2, 'byte', detail), string.byte, 'abc', value)
    expect_error(argument(2, 'sub', detail), string.sub, 'abc', value)
    expect_error(argument(2, 'rep', detail), string.rep, '', value)
    expect_error(argument(3, 'find', detail), string.find, 'abc', 'a', value)
    expect_error(argument(3, 'match', detail), string.match, 'abc', 'a', value)
    expect_error(argument(4, 'gsub', detail), string.gsub, 'abc', 'a', 'x', value)
    expect_error(argument(1, 'char', detail), string.char, value)
    expect_error(argument(2, 'format', detail), string.format, '%d', value)
end
for _, value in ipairs({256, -1}) do
    expect_error(argument(1, 'char', 'value out of range'), string.char, value)
end
expect_error(argument(2, 'char', 'value out of range'), string.char, 65, 256)
expect_error(argument(1, 'gsub', 'string expected'), string.gsub, {}, 'a', 'b')
expect_error(argument(2, 'gsub', 'string expected'), string.gsub, 'a', {}, 'b')
expect_error(argument(3, 'gsub', 'string expected'), string.gsub, 'a', 'a', false)
expect_error('invalid capture index', string.gsub, 'a', 'a', '%2')
assert(string.gsub('a', 'a', '%1') == 'a')
assert(string.gsub('a', 'a', '%') == '')

for _, spec in ipairs({'%s', '%q'}) do
    expect_error(argument(2, 'format', 'string expected'), string.format, spec, {})
    expect_error(argument(2, 'format', 'string expected'), string.format, spec, false)
end
expect_error(argument(2, 'format', 'number expected'), string.format, '%d', {})
expect_error(argument(2, 'format', 'number expected'), string.format, '%f', {})
expect_error("invalid option '%l' to 'format'", string.format, '%ld', 1)
expect_error("invalid option '%%' to 'format'", string.format, '%')
expect_error('string.format: result exceeds resource limit', string.format, '%67108865s', 'a')
expect_error('string.rep: result exceeds resource limit', string.rep, 'xx', 33554433)
assert(string.format('%u/%x', -1, -1) == '4294967295/ffffffff')
assert(string.byte('abc', ' 2 ') == 98)
assert(string.byte('abc', '0x2') == 98)
assert(string.byte('abc', 2.75) == 98)
assert(string.char(255.75, -0.75) == string.char(255, 0))
assert(string.rep('', 2147483647) == '')
assert(string.sub('abc', -2147483648, 2147483647) == 'abc')
assert(select('#', string.byte('abc', 2147483647)) == 0)

-- Plain search must accept bytes that are malformed as a Lua pattern.
for _, pattern in ipairs({'[', '%', '(', '%f'}) do
    assert(string.find(pattern, pattern, 1, true) == 1)
end
expect_error('unfinished capture', string.find, 'abc', '(')
expect_error('invalid capture index', string.match, 'abc', '%1')

local co = coroutine.create(function()
    expect_error(argument(3, 'find', 'number expected'), string.find, 'abc', 'a', {})
    coroutine.yield('recovered')
    return string.char(65)
end)
local ok, value = coroutine.resume(co)
assert(ok and value == 'recovered')
ok, value = coroutine.resume(co)
assert(ok and value == 'A')
print('string argument boundary passed')
