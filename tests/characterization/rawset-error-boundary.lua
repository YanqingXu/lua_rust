local function expect_error(call, message)
    local ok, err = pcall(call)
    assert(not ok, 'invalid rawset must fail')
    assert(type(err) == 'string', 'rawset error must be a string')
    assert(string.sub(err, -#message) == message, err)
end

expect_error(function() rawset() end, 'rawset: missing arguments')
expect_error(function() rawset({}, 'key') end, 'rawset: missing arguments')
expect_error(function() rawset(false, 'key', 1) end, 'rawset: table expected')
expect_error(function() rawset({}, nil, 1) end, 'rawset: table index is nil')
expect_error(function() rawset({}, math.sqrt(-1), 1) end, 'rawset: table index is NaN')
expect_error(function() rawset({}, math.sqrt(-1), nil) end, 'rawset: table index is NaN')

local co = coroutine.create(function()
    rawset({}, math.sqrt(-1), 1)
end)
local ok, err = coroutine.resume(co)
assert(not ok and type(err) == 'string')
local message = 'rawset: table index is NaN'
assert(string.sub(err, -#message) == message, err)
assert(coroutine.status(co) == 'dead')

local survivor = coroutine.create(function()
    expect_error(function() rawset({}, math.sqrt(-1), 1) end, 'rawset: table index is NaN')
    coroutine.yield('recovered')
    return 'finished'
end)
local resumed, value = coroutine.resume(survivor)
assert(resumed and value == 'recovered')
resumed, value = coroutine.resume(survivor)
assert(resumed and value == 'finished')

local t = {}
assert(rawequal(rawset(t, math.huge, 1), t))
assert(rawset(t, -math.huge, 2)[math.huge] == 1)
assert(t[-math.huge] == 2)
assert(rawequal(rawset(t, math.huge, nil), t))
assert(t[math.huge] == nil)
print('rawset error boundary passed')
