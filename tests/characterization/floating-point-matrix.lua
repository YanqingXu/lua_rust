-- Characterized against the pinned C++ oracle and official Lua 5.1.5.
-- Both agree on every entry. Each expected row uses the operand order below.
-- Separate generated chunks exercise constant folding and register operands.
local operands = {'0', '-0', '1', '-1', '2', '-2',
                  '(1e308*1e308)', '(-1e308*1e308)', '((-1)^0.5)'}
local expected = {

    ['/'] = {
        {'nan', 'nan', '+zero', '-zero', '+zero', '-zero', '+zero', '-zero', 'nan'},
        {'nan', 'nan', '-zero', '+zero', '-zero', '+zero', '-zero', '+zero', 'nan'},
        {'+inf', '-inf', '1', '-1', '0.5', '-0.5', '+zero', '-zero', 'nan'},
        {'-inf', '+inf', '-1', '1', '-0.5', '0.5', '-zero', '+zero', 'nan'},
        {'+inf', '-inf', '2', '-2', '1', '-1', '+zero', '-zero', 'nan'},
        {'-inf', '+inf', '-2', '2', '-1', '1', '-zero', '+zero', 'nan'},
        {'+inf', '-inf', '+inf', '-inf', '+inf', '-inf', 'nan', 'nan', 'nan'},
        {'-inf', '+inf', '-inf', '+inf', '-inf', '+inf', 'nan', 'nan', 'nan'},
        {'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan'},
    },
    ['%'] = {
        {'nan', 'nan', '+zero', '+zero', '+zero', '+zero', 'nan', 'nan', 'nan'},
        {'nan', 'nan', '+zero', '+zero', '+zero', '+zero', 'nan', 'nan', 'nan'},
        {'nan', 'nan', '+zero', '+zero', '1', '-1', 'nan', 'nan', 'nan'},
        {'nan', 'nan', '+zero', '+zero', '1', '-1', 'nan', 'nan', 'nan'},
        {'nan', 'nan', '+zero', '+zero', '+zero', '+zero', 'nan', 'nan', 'nan'},
        {'nan', 'nan', '+zero', '+zero', '+zero', '+zero', 'nan', 'nan', 'nan'},
        {'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan'},
        {'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan'},
        {'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan'},
    },
    ['^'] = {
        {'1', '1', '+zero', '+inf', '+zero', '+inf', '+zero', '+inf', 'nan'},
        {'1', '1', '-zero', '-inf', '+zero', '+inf', '+zero', '+inf', 'nan'},
        {'1', '1', '1', '1', '1', '1', '1', '1', '1'},
        {'1', '1', '-1', '-1', '1', '1', '1', '1', 'nan'},
        {'1', '1', '2', '0.5', '4', '0.25', '+inf', '+zero', 'nan'},
        {'1', '1', '-2', '-0.5', '4', '0.25', '+inf', '+zero', 'nan'},
        {'1', '1', '+inf', '+zero', '+inf', '+zero', '+inf', '+zero', 'nan'},
        {'1', '1', '-inf', '-zero', '+inf', '+zero', '+inf', '+zero', 'nan'},
        {'1', '1', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan', 'nan'},
    },
}

local function classify(n)
    assert(type(n) == 'number')
    if not(n < 0) and not(n >= 0) then return 'nan' end
    if n <= 0 and n >= 0 then return 1/n < 0 and '-zero' or '+zero' end
    if n == math.huge then return '+inf' end
    if n == -math.huge then return '-inf' end
    return tostring(n)
end

local count = 0
for _, op in ipairs({'/', '%', '^'}) do
    for a, lhs in ipairs(operands) do
        for b, rhs in ipairs(operands) do
            local expression = '(' .. lhs .. ') ' .. op .. ' (' .. rhs .. ')'
            local sources = {
                'return ' .. expression,
                'local a=' .. lhs .. '; local b=' .. rhs .. '; return a ' .. op .. ' b'
            }
            for _, source in ipairs(sources) do
                local callback = assert(loadstring(source))
                local ok, value = pcall(callback)
                assert(ok, source)
                assert(classify(value) == expected[op][a][b], source)
                count = count + 1
            end
        end
    end
end

local nan = (-1)^0.5
assert(0 == -0 and not(0 ~= -0))
assert(not(nan == nan) and nan ~= nan)
assert(not(nan < nan) and not(nan <= nan))
assert(not(nan > nan) and not(nan >= nan))
assert(rawequal(0, -0) and not(rawequal(nan, nan)))
local function divide(a, b) return a / b end
local function modulo(a, b) return a % b end
assert(classify(divide('1', '-0')) == '-inf')
assert(classify(modulo('1', '0')) == 'nan')

-- Table equality uses numeric equality too, without losing the sign of the
-- originally inserted zero key exposed by next. Generate -0 at runtime so
-- an oracle's constant deduplication cannot conflate the two test inputs.
local positive = 0
local negative = -positive
for _, first in ipairs({positive, negative}) do
    local opposite = -first
    local t = {keep = true}
    t[first] = 'first'
    assert(t[opposite] == 'first')
    t[opposite] = 'updated'
    local count = 0
    for key, value in pairs(t) do
        count = count + 1
        if type(key) == 'number' then
            assert(1/key == 1/first and value == 'updated')
        end
    end
    assert(count == 2)
    assert(next(t, first) == next(t, opposite))
    t[opposite] = nil
    assert(t[first] == nil and t.keep)
    assert(next(t, first) == next(t, opposite))
end
assert(count == 486)
print('floating-point matrix passed')
