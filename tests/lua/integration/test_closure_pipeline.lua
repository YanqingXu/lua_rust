print("=== Integration: Closure Pipeline ===")

local function pack(...)
    return { n = select("#", ...), ... }
end

local function makePipeline(scale, bias)
    local history = { n = 0 }
    local invocations = 0

    local function remember(value)
        history.n = history.n + 1
        history[history.n] = value
    end

    return function(...)
        invocations = invocations + 1

        local args = pack(...)
        local transformed = { n = args.n }
        local sum = 0
        local i = 1

        while i <= args.n do
            local value = args[i] * scale + bias + invocations
            transformed[i] = value
            sum = sum + value
            remember(value)
            i = i + 1
        end

        return sum, transformed, history, invocations
    end
end

local function fold(list)
    local total = 0
    local minValue = list[1]
    local maxValue = list[1]

    for i = 1, list.n do
        local value = list[i]
        total = total + value

        if value < minValue then
            minValue = value
        end

        if value > maxValue then
            maxValue = value
        end
    end

    return total, minValue, maxValue
end

local pipeline = makePipeline(3, -1)

local firstSum, firstBatch, firstHistory, firstCall = pipeline(1, 2, 3)
assert(firstCall == 1, "first invocation count")
assert(firstSum == 18, "first sum")
assert(firstBatch.n == 3, "first batch size")
assert(firstBatch[1] == 3 and firstBatch[2] == 6 and firstBatch[3] == 9, "first batch values")
assert(firstHistory.n == 3 and firstHistory[3] == 9, "first history snapshot")

local function forward(fn, ...)
    return fn(...)
end

local secondSum, secondBatch, secondHistory, secondCall = forward(pipeline, 4, 5)
assert(secondCall == 2, "second invocation count")
assert(secondSum == 29, "second sum")
assert(secondBatch.n == 2, "second batch size")

local secondA, secondB = unpack(secondBatch, 1, secondBatch.n)
assert(secondA == 13 and secondB == 16, "second batch values")
assert(secondHistory.n == 5, "history grows across calls")

local grandTotal, minValue, maxValue = fold(secondHistory)
assert(grandTotal == 47, "fold total")
assert(minValue == 3, "fold min")
assert(maxValue == 16, "fold max")

local named = {
    first = firstSum,
    second = secondSum,
    grand = grandTotal
}

local seenKeys = 0
for key, value in pairs(named) do
    assert(type(key) == "string", "named key type")
    assert(type(value) == "number", "named value type")
    seenKeys = seenKeys + 1
end

assert(seenKeys == 3, "pairs should see three keys")

print("firstSum =", firstSum)
print("secondSum =", secondSum)
print("grandTotal =", grandTotal)
print("historyCount =", secondHistory.n)
print("=== Closure pipeline passed ===")
