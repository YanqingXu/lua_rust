local calls = 0

local function hook(event)
    calls = calls + 1
end

local function target()
    return 1
end

debug.sethook(hook, "c")
local value = target()
debug.sethook()
assert(value == 1)
assert(calls > 0)
