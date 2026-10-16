-- Fixed calls do not read cases.tsv or derive expected values from candidate output.
local F = string.find
local C = string.char
local cases = {
  ["single-capture"] = function() return F('abc','(b)') end,
  ["two-captures"] = function() return F('ab12','(%a+)(%d+)') end,
  ["nested-open-order"] = function() return F('abc','((a)(b))') end,
  ["nested-same-span"] = function() return F('a','((a))') end,
  ["empty-string-capture"] = function() return F('bbb','(a*)') end,
  ["position-only"] = function() return F('abc','()') end,
  ["empty-subject-position"] = function() return F('','()') end,
  ["position-around-string"] = function() return F('ab','()(a)()') end,
  ["nested-empty-and-position"] = function() return F('','(())') end,
  ["binary-nul-high-byte"] = function() return F('A'..C(0,255)..'B','(%z.)') end,
  ["binary-whole-capture"] = function() return F(C(0,255,10,65),'(.*)') end,
  ["capture-linefeed"] = function() return F(C(10),'(.)') end,
  ["parentheses-inside-set"] = function() return F('(()x','([()]+)') end,
  ["escaped-parenthesis-capture"] = function() return F('(','(%()') end,
  ["star-restores-closed-slot"] = function() return F('aaab','(a*)ab') end,
  ["plus-restores-slot-and-order"] = function() return F('aaab','(a+)(a)b') end,
  ["retry-truncates-new-slot"] = function() return F('aaab','a*(a)b') end,
  ["retry-restores-nested-slots"] = function() return F('aaab','((a*)a)b') end,
  ["retry-replaces-position"] = function() return F('aaab','a*()ab') end,
  ["retry-restores-mixed-slots"] = function() return F('aaab','((a*)())ab') end,
  ["multiple-retries-restore"] = function() return F('aaab','(a*)a*(a)b') end,
  ["candidate-reset-closed-slot"] = function() return F('abac','(a)c') end,
  ["candidate-reset-empty-slot"] = function() return F('xaab','(a*)b') end,
  ["candidate-reset-position"] = function() return F('xabc','()(a)b') end,
  ["complete-miss-has-no-captures"] = function() return F('aaac','(a*)b') end,
  ["next-call-after-miss"] = function() F('b','(a)'); return F('aaab','(a*)ab') end,
  ["next-call-fewer-captures"] = function() F('ab','(a)(b)'); return F('c','(c)') end,
  ["next-call-after-position"] = function() F('ab','()'); return F('xyz','(y)') end,
  ["unfinished-capture-misses"] = function() return F('b','(a') end,
  ["unfinished-capture-success-errors"] = function() return F('a','(a') end,
  ["extra-close-unreached"] = function() return F('b','^a)') end,
  ["extra-close-reached"] = function() return F('a','^a)') end,
  ["close-only-auto-plain"] = function() return F(')',')') end,
  ["position-extra-close"] = function() return F('a','())') end,
  ["thirty-three-positions-error"] = function() return F('',string.rep('()',33)) end,
  ["thirty-two-positions"] = function() return F('',string.rep('()',32)) end,
  ["thirty-two-string-captures"] = function() return F(string.rep('a',32),string.rep('(a)',32)) end,
  ["thirty-two-nested-captures"] = function() return F('a',string.rep('(',32)..'a'..string.rep(')',32)) end,
  ["thirty-three-string-captures-error"] = function() return F(string.rep('a',33),string.rep('(a)',33)) end,
  ["thirty-two-mixed-captures"] = function() return F('a',string.rep('()',31)..'(a)') end,
  ["thirty-three-mixed-captures-error"] = function() return F('a',string.rep('()',32)..'(a)') end,
  ["too-many-captures-unreached"] = function() return F('a','z'..string.rep('()',33)) end,
  ["capture-before-pattern-nul"] = function() return F('ab','(a)'..C(0)..'(b)') end,
  ["capture-after-pattern-nul-auto-plain"] = function() return F('x'..C(0)..'(a)',C(0)..'(a)') end,
  ["plain-capture-spelling"] = function() return F('a(b)','(b)',1,true) end,
  ["truthy-zero-capture-spelling"] = function() return F('a(b)','(b)',1,0) end,
  ["default-capture-spelling"] = function() return F('a(b)','(b)') end,
  ["position-past-end-init"] = function() return F('abc','()',99) end,
  ["captured-anchor"] = function() return F('abbb','(b+)$') end,
  ["unfinished-failed-end-anchor"] = function() return F('ac','(a$') end,
  ["bad-set-inside-capture-unreached"] = function() return F('b','(a[') end,
  ["bad-set-inside-capture-reached"] = function() return F('a','(a[') end,
  ["two-empty-string-captures"] = function() return F('','(a*)(b*)') end,
  ["no-explicit-captures"] = function() return F('abc','b') end,
}

local function emit_integer(value)
  local digits = ''
  repeat
    local digit = value % 10
    digits = string.char(48 + digit) .. digits
    value = math.floor(value / 10)
  until value == 0
  io.write(digits)
end

local function emit_hex(value)
  local digits = '0123456789abcdef'
  for index=1,#value do
    local byte=string.byte(value,index)
    local hi=math.floor(byte/16)+1
    local lo=byte%16+1
    io.write(string.sub(digits,hi,hi),string.sub(digits,lo,lo))
  end
end

local function report(ok,...)
  if not ok then io.write('error|after'); return end
  io.write('ok|'); emit_integer(select('#',...))
  for index=1,select('#',...) do
    local value=select(index,...)
    if value==nil then io.write('|nil')
    elseif type(value)=='number' then
      assert(value>=0,'unexpected negative coordinate')
      io.write('|n:'); emit_integer(value)
    elseif type(value)=='string' then
      io.write('|s:'); emit_integer(#value); io.write(':'); emit_hex(value)
    else error('unexpected find result type') end
  end
  io.write('|after')
end

local call=assert(cases[arg[1]],'unknown fixed capture case')
report(pcall(call))
