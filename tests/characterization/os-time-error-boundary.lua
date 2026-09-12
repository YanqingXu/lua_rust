local function date()
    return {year = 2000, month = 1, day = 1, hour = 12, min = 0, sec = 0}
end

local function expect_error(value, message)
    local ok, err = pcall(os.time, value)
    assert(not ok, 'invalid os.time must fail')
    assert(type(err) == 'string', 'os.time error must be a string')
    assert(string.sub(err, -#message) == message, err)
end

local invalid = {
    1e300, -1e300, math.huge, -math.huge, math.sqrt(-1),
    2147483648, -2147483649, 2147483648.5, -2147483649.5
}
for _, field in ipairs({'year', 'month', 'day', 'hour', 'min', 'sec'}) do
    for _, value in ipairs(invalid) do
        local fields = date()
        fields[field] = value
        expect_error(fields, 'date table field has no valid integer representation')
    end
end

for _, field in ipairs({'year', 'month', 'day'}) do
    for _, value in ipairs({false, {}, '2000'}) do
        local fields = date()
        fields[field] = value
        expect_error(fields, "field '" .. field .. "' missing in date table")
    end
    local fields = date()
    fields[field] = nil
    expect_error(fields, "field '" .. field .. "' missing in date table")
end
expect_error(1, 'time: argument must be a table')
expect_error(false, 'time: argument must be a table')
expect_error('2000', 'time: argument must be a table')

-- Finite fractions are truncated, and optional nonnumber fields use defaults,
-- matching the pinned C++ field conversion policy.
local baseline = os.time(date())
for _, field in ipairs({'year', 'month', 'day', 'hour', 'min', 'sec'}) do
    local fields = date()
    fields[field] = fields[field] + 0.75
    assert(os.time(fields) == baseline)
end
for _, field in ipairs({'hour', 'min', 'sec'}) do
    for _, value in ipairs({false, {}, '17'}) do
        local fields = date()
        fields[field] = value
        assert(os.time(fields) == baseline)
    end
end

local co = coroutine.create(function()
    os.time({year = -1e300, month = 1, day = 1})
end)
local ok, err = coroutine.resume(co)
assert(not ok and type(err) == 'string')
local message = 'date table field has no valid integer representation'
assert(string.sub(err, -#message) == message, err)
assert(coroutine.status(co) == 'dead')
local survivor = coroutine.create(function()
    expect_error({year = 1e300, month = 1, day = 1}, message)
    coroutine.yield('recovered')
    return os.time(date()) == baseline
end)
local resumed, value = coroutine.resume(survivor)
assert(resumed and value == 'recovered')
resumed, value = coroutine.resume(survivor)
assert(resumed and value == true)
print('os.time error boundary passed')
