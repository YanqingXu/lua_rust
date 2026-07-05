print("=== MultiRet Edge Regression ===")

local function pack3()
    return 10, 20, 30
end

local a, b, c = pack3()
assert(a == 10 and b == 20 and c == 30, "assignment should keep all returned values")

local single = (pack3())
assert(single == 10, "parenthesized call should collapse to first value")

local function collapse()
    return (pack3())
end

local p, q, r = collapse()
assert(p == 10 and q == nil and r == nil, "return (f()) should collapse to one value")

local pad1, pad2, pad3, pad4 = pack3()
assert(pad1 == 10 and pad2 == 20 and pad3 == 30, "assignment should preserve the first three values")
assert(pad4 == nil, "assignment should pad extra targets with nil")

local t = {pack3()}
assert(t[1] == 10 and t[2] == 20 and t[3] == 30, "last table field should expand multret")
assert(t[4] == nil, "expanded table field should stop after returned values")

local t2 = {(pack3())}
assert(t2[1] == 10 and t2[2] == nil and t2[3] == nil, "parenthesized table field should collapse")

print("MultiRet edge regression passed")
