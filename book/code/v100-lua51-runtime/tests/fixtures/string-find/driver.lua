-- Fixed calls are independent of cases.tsv; no expected result is read here.
local F = string.find
local C = string.char
local cases = {
  ["literal-default"] = function() return F('abcdef','bc') end,
  ["plain-true-dot"] = function() return F('a.b','.',1,true) end,
  ["plain-zero-dot"] = function() return F('a.b','.',1,0) end,
  ["plain-empty-string-dot"] = function() return F('a.b','.',1,'') end,
  ["pattern-false-dot"] = function() return F('a.b','.',1,false) end,
  ["pattern-nil-dot"] = function() return F('a.b','.',1,nil) end,
  ["plain-bad-class-literal"] = function() return F('abc','[',1,true) end,
  ["plain-pattern-nul"] = function() return F('a'..C(0)..'B',C(0)..'B',1,true) end,
  ["auto-plain-pattern-nul"] = function() return F('a'..C(0)..'B',C(0)..'B') end,
  ["pattern-nul-meta-before"] = function() return F('ab','.'..C(0)..'x') end,
  ["pattern-nul-meta-after"] = function() return F('a'..C(0)..'x',C(0)..'.') end,
  ["literal-not-found"] = function() return F('abc','z') end,
  ["init-negative"] = function() return F('abcdef','cd',-4) end,
  ["init-very-negative"] = function() return F('abc','a',-99) end,
  ["init-zero"] = function() return F('abc','a',0) end,
  ["init-positive"] = function() return F('abcabc','a',4) end,
  ["init-past-end-nonempty"] = function() return F('abc','a',99) end,
  ["init-past-end-empty"] = function() return F('abc','',99) end,
  ["empty-subject-empty"] = function() return F('','',99) end,
  ["init-end-empty"] = function() return F('abc','',4) end,
  ["init-negative-empty"] = function() return F('abc','',-1) end,
  ["dot-subject-nul"] = function() return F('A'..C(0,255)..'B','.%z.') end,
  ["dot-high-byte"] = function() return F(C(255),'.') end,
  ["dot-linefeed"] = function() return F(C(10),'.') end,
  ["escaped-dot"] = function() return F('a.b','%.b') end,
  ["escaped-percent"] = function() return F('a%b','%%') end,
  ["set-initial-close-literal"] = function() return F(']x','[]]') end,
  ["escaped-plus"] = function() return F('a+b','%+') end,
  ["class-digits"] = function() return F('abc12x','%d+') end,
  ["class-nondigits"] = function() return F('12ab3','%D+') end,
  ["class-zero"] = function() return F('a'..C(0,255),'%z.') end,
  ["class-nonzero"] = function() return F(C(0)..'A'..C(255),'%Z+') end,
  ["class-word-excludes-underscore"] = function() return F('a_2','%w+') end,
  ["class-space"] = function() return F('A '..C(9)..'B','%s+') end,
  ["class-alpha"] = function() return F('12Az!','%a+') end,
  ["class-control"] = function() return F('A'..C(0,10)..'B','%c+') end,
  ["class-punctuation"] = function() return F('a_!0','%p+') end,
  ["class-hex"] = function() return F('gAf09z','%x+') end,
  ["class-upper"] = function() return F('aAZb','%u+') end,
  ["class-lower"] = function() return F('AazB','%l+') end,
  ["set-range"] = function() return F('x345y','[0-9]+') end,
  ["set-complement"] = function() return F('123ab4','[^0-9]+') end,
  ["set-class-and-literal"] = function() return F('ab_12!','[%w_]+') end,
  ["set-escaped-close"] = function() return F(']x','[%]]') end,
  ["set-literal-hyphen"] = function() return F('--a','[-a]+') end,
  ["set-high-byte-range"] = function() return F(C(255),'['..C(128)..'-'..C(255)..']') end,
  ["greedy-zero"] = function() return F('bbb','a*') end,
  ["greedy-suffix-backtrack"] = function() return F('aaab','a*ab') end,
  ["plus-suffix-backtrack"] = function() return F('aaab','a+ab') end,
  ["plus-needs-one"] = function() return F('bbb','a+') end,
  ["greedy-class-backtrack"] = function() return F('1234','%d*4') end,
  ["greedy-dot-backtrack"] = function() return F('abbbc','a.*bc') end,
  ["greedy-suffix-fails"] = function() return F('aaa','a*b') end,
  ["multiple-quantifier-backtrack"] = function() return F('aaabbc','a*b*bc') end,
  ["anchor-start-fails"] = function() return F('ba','^a') end,
  ["anchor-start-at-init"] = function() return F('ba','^a',2) end,
  ["anchor-end"] = function() return F('abbb','b+$') end,
  ["anchor-end-empty"] = function() return F('abc','$') end,
  ["anchors-empty-subject"] = function() return F('','^$') end,
  ["anchor-middle-literal"] = function() return F('a^b$c','a^b$c') end,
  ["escaped-anchors"] = function() return F('a^b$','%^b%$') end,
  ["bad-class-unreached"] = function() return F('abc','z[') end,
  ["bad-class-reached"] = function() return F('abc','a[') end,
  ["bad-escape-unreached"] = function() return F('abc','z%') end,
  ["bad-escape-reached"] = function() return F('abc','a%') end,
  ["bad-class-alone"] = function() return F('abc','[') end,
  ["bad-class-complement"] = function() return F('abc','[^') end,
  ["empty-set-rejected"] = function() return F('abc','[]') end,
  ["unknown-class-in-set"] = function() return F('f','[%f]') end,
  ["unknown-lower-escape"] = function() return F('ag','%g') end,
  ["unknown-upper-escape"] = function() return F('aG','%G') end,
  ["empty-subject-literal"] = function() return F('','a') end,
  ["missing-subject"] = function() return F() end,
  ["pattern-type-rejected"] = function() return F('abc',{}) end,
  ["init-type-rejected"] = function() return F('abc','a',{}) end,
}

-- Arithmetic/char encoding does not call string.format or pattern matching.
local function emit_integer(value)
  local digits = ''
  repeat
    local digit = value % 10
    digits = string.char(48 + digit) .. digits
    value = math.floor(value / 10)
  until value == 0
  io.write(digits)
end

local function report(ok, ...)
  if not ok then
    io.write('error|after')
    return
  end
  io.write('ok|')
  emit_integer(select('#', ...))
  for index = 1, select('#', ...) do
    local value = select(index, ...)
    if value == nil then
      io.write('|nil')
    else
      assert(type(value) == 'number' and value >= 0, 'unexpected find result')
      io.write('|n:')
      emit_integer(value)
    end
  end
  io.write('|after')
end

local call = assert(cases[arg[1]], 'unknown fixed find case')
report(pcall(call))
