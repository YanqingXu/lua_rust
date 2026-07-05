-- test_call_pipeline.lua
-- PR-5 regression tests: Call/Vararg/MultiRet pipeline

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

-- helper: return multiple values
local function pack3() return 10, 20, 30 end

-- 1. return f() propagates multret
local function relay() return pack3() end
local a, b, c = relay()
check("return f() propagates multret", a == 10 and b == 20 and c == 30)

-- 2. return ... propagates multret
local function varRelay(...) return ... end
a, b, c = varRelay(10, 20, 30)
check("return ... propagates multret", a == 10 and b == 20 and c == 30)

-- 3. g(f()) passes all returns
local out = {}
local function capture(a, b, c, d)
    out[1], out[2], out[3], out[4] = a, b, c, d
end
capture(pack3())
check("g(f()) passes all returns", out[1] == 10 and out[2] == 20 and out[3] == 30 and out[4] == nil)

-- 4. g(fixed, f()) combines fixed + multret
capture(99, pack3())
check("g(fixed, f()) combines", out[1] == 99 and out[2] == 10 and out[3] == 20 and out[4] == 30)

-- 5. g(f(), fixed) collapses f to single
capture(pack3(), 99)
check("g(f(), fixed) collapses f", out[1] == 10 and out[2] == 99 and out[3] == nil and out[4] == nil)

-- 6. (f()) collapses to single
a, b = (pack3())
check("(f()) collapses", a == 10 and b == nil)

-- 7. return (f()) collapses to single
local function parenRelay() return (pack3()) end
a, b, c = parenRelay()
check("return (f()) collapses", a == 10 and b == nil and c == nil)

-- 8. print((f())) consumes one argument
do
    local printed = {}
    local function print(...)
        printed.n = select('#', ...)
        printed[1], printed[2], printed[3] = ...
    end
    print((pack3()))
    check("print((f())) collapses", printed.n == 1 and printed[1] == 10 and printed[2] == nil)
end

-- 9. local a,b,c = f()
local x, y, z = pack3()
check("local a,b,c = f()", x == 10 and y == 20 and z == 30)

-- 10. a,b,c = f() assignment
local p, q, r
p, q, r = pack3()
check("a,b,c = f() assign", p == 10 and q == 20 and r == 30)

-- 11. {f()} table multret
local t = {pack3()}
check("{f()} expands", t[1] == 10 and t[2] == 20 and t[3] == 30 and t[4] == nil)

-- 12. {"head", f()} in table
local t2 = {"head", pack3()}
check("{head, f()} expands", t2[1] == "head" and t2[2] == 10 and t2[3] == 20 and t2[4] == 30)

-- 13. method call multret
local obj = {}
function obj:multi() return 10, 20, 30 end
a, b, c = obj:multi()
check("method call multret", a == 10 and b == 20 and c == 30)

-- 14. nested return f(g())
local function double(v) return v * 2 end
local function getVal() return 5 end
local function chain() return double(getVal()) end
check("nested return chain", chain() == 10)

-- 15. vararg in local
local function varLocal(...)
    local a, b, c = ...
    return a, b, c
end
a, b, c = varLocal(10, 20, 30)
check("local a,b,c = ...", a == 10 and b == 20 and c == 30)

-- 16. extra locals get nil
local function shortRet() return 10 end
local u, v, w = shortRet()
check("extra locals nil", u == 10 and v == nil and w == nil)

-- summary
print(string.format("call_pipeline: %d pass, %d fail", pass, fail))
assert(fail == 0, "call_pipeline regression tests failed")
