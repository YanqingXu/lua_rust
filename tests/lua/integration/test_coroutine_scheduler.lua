print("=== Integration: Coroutine Scheduler ===")

local function makeWorker(name, factor)
    return coroutine.create(function()
        local total = 0
        local step = 0

        while true do
            local input = coroutine.yield("need-value", name, step, total)
            if input == nil then
                return "done", name, step, total
            end

            step = step + 1
            total = total + input * factor
        end
    end)
end

local function expectResume(co, ...)
    local ok, state, name, step, total = coroutine.resume(co, ...)
    assert(ok, state)
    return state, name, step, total
end

local function makeRecorder()
    local entries = { n = 0 }

    local function record(tag, name, step, total)
        entries.n = entries.n + 1
        entries[entries.n] = tag .. ":" .. name .. ":" .. step .. ":" .. total
    end

    return record, entries
end

local record, log = makeRecorder()
local alpha = makeWorker("alpha", 2)
local beta = makeWorker("beta", 3)

local state, name, step, total = expectResume(alpha)
assert(state == "need-value" and name == "alpha" and step == 0 and total == 0, "alpha bootstrap")
record("start", name, step, total)

state, name, step, total = expectResume(beta)
assert(state == "need-value" and name == "beta" and step == 0 and total == 0, "beta bootstrap")
record("start", name, step, total)

state, name, step, total = expectResume(alpha, 5)
assert(step == 1 and total == 10, "alpha first tick")
record("tick", name, step, total)

state, name, step, total = expectResume(beta, 1)
assert(step == 1 and total == 3, "beta first tick")
record("tick", name, step, total)

state, name, step, total = expectResume(alpha, 2)
assert(step == 2 and total == 14, "alpha second tick")
record("tick", name, step, total)

state, name, step, total = expectResume(beta, 4)
assert(step == 2 and total == 15, "beta second tick")
record("tick", name, step, total)

state, name, step, total = expectResume(alpha, nil)
assert(state == "done" and step == 2 and total == 14, "alpha completion")
record("finish", name, step, total)

state, name, step, total = expectResume(beta, nil)
assert(state == "done" and step == 2 and total == 15, "beta completion")
record("finish", name, step, total)

assert(coroutine.status(alpha) == "dead", "alpha dead")
assert(coroutine.status(beta) == "dead", "beta dead")
assert(log.n == 8, "log entry count")
assert(log[3] == "tick:alpha:1:10", "alpha log detail")
assert(log[8] == "finish:beta:2:15", "beta log detail")

local broken = coroutine.create(function()
    coroutine.yield("armed")
    error("scheduler-broken")
end)

local ok1, armed = coroutine.resume(broken)
assert(ok1 and armed == "armed", "broken coroutine first yield")

local ok2, err = coroutine.resume(broken)
assert(not ok2, "broken coroutine should fail")
assert(string.find(err, "scheduler%-broken") ~= nil, "error text should mention scheduler-broken")
assert(coroutine.status(broken) == "dead", "broken coroutine dead")

print("alphaTotal =", 14)
print("betaTotal =", 15)
print("logCount =", log.n)
print("errorCaptured =", ok2)
print("=== Coroutine scheduler passed ===")
