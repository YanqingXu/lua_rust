print("=== Short Circuit Materialization Regression ===")

local rhsCalls = 0

local function mark(v)
    rhsCalls = rhsCalls + 1
    return v
end

local andSkip = false and mark("and-skip")
assert(andSkip == false, "false and rhs should stay false")
assert(rhsCalls == 0, "false and rhs should not execute")

local andTake = true and mark("and-take")
assert(andTake == "and-take", "true and rhs should return rhs")
assert(rhsCalls == 1, "true and rhs should execute once")

local nilSkip = nil and mark("nil-skip")
assert(nilSkip == nil, "nil and rhs should stay nil")
assert(rhsCalls == 1, "nil and rhs should not execute")

local zeroTake = 0 and mark("zero-is-truthy")
assert(zeroTake == "zero-is-truthy", "0 should be truthy in Lua")
assert(rhsCalls == 2, "0 and rhs should execute rhs")

local orSkip = true or mark("or-skip")
assert(orSkip == true, "true or rhs should keep lhs")
assert(rhsCalls == 2, "true or rhs should not execute")

local orTake = false or mark("or-take")
assert(orTake == "or-take", "false or rhs should return rhs")
assert(rhsCalls == 3, "false or rhs should execute")

assert((not nil) == true, "not nil should be true")
assert((not false) == true, "not false should be true")
assert((not 0) == false, "not truthy value should be false")

local branch = 0
if false and mark(true) then
    branch = -100
else
    branch = branch + 1
end

if true or mark(false) then
    branch = branch + 10
end

local whileCount = 0
while whileCount < 2 and mark(true) do
    whileCount = whileCount + 1
end

local repeatCount = 0
repeat
    repeatCount = repeatCount + 1
until repeatCount == 2 or mark(false)

assert(branch == 11, "if conditions should take expected branches")
assert(whileCount == 2, "while condition should loop twice")
assert(repeatCount == 2, "repeat-until condition should stop on second iteration")
assert(rhsCalls == 6, "only live rhs branches should execute")

print("Short circuit materialization regression passed")
