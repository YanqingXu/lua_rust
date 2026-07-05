-- test_upvalue_close.lua
-- Regression tests for closing captured locals when block scopes exit.

local pass = 0
local fail = 0

local function check(name, cond)
    if cond then
        pass = pass + 1
    else
        fail = fail + 1
        print("FAIL: " .. name)
    end
end

do
    local f
    do
        local x = 41
        f = function() return x end
    end
    local x = 99
    check("do block closes captured local", f() == 41)
end

do
    local f
    while true do
        local x = 7
        f = function() return x end
        break
    end
    local x = 99
    check("break closes loop-local upvalue", f() == 7)
end

do
    local outer = 1
    local readOuter = function() return outer end
    local f

    local function maker()
        do
            local x = 2
            f = function() return x end
        end
    end

    maker()
    outer = 3
    check("CLOSE is relative to current frame", readOuter() == 3)
    check("inner block still closes captured local", f() == 2)
end

print(string.format("upvalue_close: %d pass, %d fail", pass, fail))
assert(fail == 0, "upvalue close regression tests failed")
