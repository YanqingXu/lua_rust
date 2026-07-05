-- Lua 5.1 语义回归：
-- 表构造器最后一个 listfield 才会展开多返回值；
-- 括号表达式 (exp) 会将 multret 收敛为单值。

local function fail(msg)
    print("FAIL:", msg)
    local x = nil + 1
    return x
end

local function f()
    return 1, 2, 3
end

local t1 = { f() }
if t1[1] ~= 1 then fail("t1[1]") end
if t1[2] ~= 2 then fail("t1[2]") end
if t1[3] ~= 3 then fail("t1[3]") end
if t1[4] ~= nil then fail("t1[4]") end

local t2 = { (f()) }
if t2[1] ~= 1 then fail("t2[1]") end
if t2[2] ~= nil then fail("t2[2]") end

local t3 = { f(), 99 }
if t3[1] ~= 1 then fail("t3[1]") end
if t3[2] ~= 99 then fail("t3[2]") end
if t3[3] ~= nil then fail("t3[3]") end

local t4 = { 0, f() }
if t4[1] ~= 0 then fail("t4[1]") end
if t4[2] ~= 1 then fail("t4[2]") end
if t4[3] ~= 2 then fail("t4[3]") end
if t4[4] ~= 3 then fail("t4[4]") end
if t4[5] ~= nil then fail("t4[5]") end

local t5 = { 0, (f()) }
if t5[1] ~= 0 then fail("t5[1]") end
if t5[2] ~= 1 then fail("t5[2]") end
if t5[3] ~= nil then fail("t5[3]") end

local function g(...)
    local a = { ... }
    local b = { (...) }
    if a[1] ~= 10 then fail("a[1]") end
    if a[2] ~= 20 then fail("a[2]") end
    if a[3] ~= 30 then fail("a[3]") end
    if a[4] ~= nil then fail("a[4]") end
    if b[1] ~= 10 then fail("b[1]") end
    if b[2] ~= nil then fail("b[2]") end
end

g(10, 20, 30)
print("table constructor multret semantics: OK")
