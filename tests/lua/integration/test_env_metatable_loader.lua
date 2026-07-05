print("=== Integration: Environment + Metatable + Loader ===")

local writes = { count = 0 }
local defaults = {
    prefix = "base",
    scale = 3,
    bonus = 10
}

local env = {
    value = 5,
    writes = writes
}

setmetatable(env, {
    __index = defaults,
    __newindex = function(tbl, key, value)
        writes.count = writes.count + 1
        rawset(tbl, key, value)
    end
})

local function compute(tag, extra)
    label = prefix .. ":" .. tag
    result = value * scale + bonus + extra
    return label, result
end

setfenv(compute, env)
assert(rawequal(getfenv(compute), env), "environment should be attached")

local label1, number1 = compute("first", 2)
assert(label1 == "base:first", "first label")
assert(number1 == 27, "first number")
assert(rawget(env, "label") == "base:first", "label stored in env")
assert(rawget(env, "result") == 27, "result stored in env")
assert(writes.count == 2, "label and result were created")

env.prefix = "custom"
env.bonus = 20
assert(writes.count == 4, "prefix and bonus writes recorded")

local label2, number2 = compute("second", 1)
assert(label2 == "custom:second", "second label")
assert(number2 == 36, "second number")
assert(writes.count == 4, "existing env fields update in place")

local SummaryMT = {}
SummaryMT.__add = function(left, right)
    return {
        total = left.total + right.total,
        count = left.count + right.count
    }
end

local summary1 = setmetatable({ total = number1, count = 1 }, SummaryMT)
local summary2 = setmetatable({ total = number2, count = 1 }, SummaryMT)
local merged = summary1 + summary2

assert(merged.total == 63, "merged total")
assert(merged.count == 2, "merged count")

local pieces = {
    "generated_total = result + bonus\n",
    "generated_label = prefix .. ':' .. tostring(value)\n"
}

local pieceIndex = 0
local chunk = assert(load(function()
    pieceIndex = pieceIndex + 1
    return pieces[pieceIndex]
end))

setfenv(chunk, env)
chunk()

assert(env.generated_total == 56, "generated total")
assert(env.generated_label == "custom:5", "generated label")
assert(writes.count == 6, "loader created two more fields")

print("firstResult =", number1)
print("secondResult =", number2)
print("mergedTotal =", merged.total)
print("writeCount =", writes.count)
print("=== Environment/metatable/loader passed ===")
