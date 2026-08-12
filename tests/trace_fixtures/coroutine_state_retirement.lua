local function suspend_with_open_upvalue(value)
    local thread = coroutine.create(function()
        local captured = value
        local function read()
            return captured
        end
        coroutine.yield(read)
    end)

    local resumed = coroutine.resume(thread)
    if not resumed then
        error("cannot suspend coroutine")
    end
end

suspend_with_open_upvalue(11)
collectgarbage("collect")
suspend_with_open_upvalue(22)
collectgarbage("collect")
suspend_with_open_upvalue(33)

print("coroutine state retirement complete")
error("coroutine state retirement trace complete")
