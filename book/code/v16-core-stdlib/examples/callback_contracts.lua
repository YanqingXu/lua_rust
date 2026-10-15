-- Predict: protected failures are followed by a legal yield in this same coroutine.
local co = coroutine.create(function()
    local ok, a, b, c = pcall(function() return 7, nil, 9 end)
    assert(ok and a == 7 and b == nil and c == 9)
    local seen = 0
    local failed, handled = xpcall(function() error('body') end, function(message)
        seen = seen + 1
        gc_collect()
        assert(type(message) == 'string')
        return { answer = 42 }, 99
    end)
    assert(not failed and handled.answer == 42 and seen == 1)
    local denied = pcall(function() coroutine.yield('forbidden') end)
    assert(not denied)
    local values = { 3, 1, 2 }
    local rejected = pcall(table.sort, values, function()
        coroutine.yield('comparator cannot suspend')
    end)
    assert(not rejected)
    table.sort(values, function(left, right)
        gc_collect()
        return left < right
    end)
    assert(table.concat(values, ',') == '1,2,3')
    local input = coroutine.yield(41, nil)
    return input + 1
end)
local first, yielded, empty = coroutine.resume(co)
assert(first and yielded == 41 and empty == nil)
local last, result = coroutine.resume(co, 42)
return last and result == 43 and coroutine.status(co) == 'dead'
