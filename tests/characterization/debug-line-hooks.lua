-- Line-hook history belongs to a Lua call frame, including recursive calls
-- of the same Proto. These expected traces are checked against both oracles.
-- Named chunks keep line numbers independent of this fixture's own layout.
local capture = [[local out={}
local function hook(event,line) assert(event=='line'); local info=debug.getinfo(2,'S'); out[#out+1]=info.short_src..':'..line end
]]

local function check(name, body, expected, prefix, helper_calls)
  local source = (prefix or capture) .. body ..
      '\nreturn table.concat(out,","), calls\n'
  local run = assert(loadstring(source, '=' .. name))
  local actual, calls = run()
  assert(actual == expected,
      name .. ': expected [' .. expected .. '], got [' .. actual .. ']')
  if helper_calls then assert(calls == helper_calls, name .. ': helper calls') end
  assert(debug.gethook() == nil, name .. ': hook must be cleared')
end

-- The first trace in official db.lua expects {2,4,7}. Returning to the
-- sethook/loadstring/clear sequence on db.lua:19 must not append that outer line.
check('db-first-trace', [[local child=assert(loadstring('if\nmath.sin(1)\nthen\n  a=1\nelse\n  a=2\nend\n','=db-child'))
debug.sethook(hook,'l'); child(); debug.sethook()]],
    'db-child:2,db-child:4,db-child:7')

check('same-line-return', [[local child=assert(loadstring('local a=1\nreturn a\n','=child'))
debug.sethook(hook,'l'); child(); local after=1; debug.sethook()]],
    'child:1,child:2')

-- Suppressing every first instruction after a return would lose these events.
check('next-line-return', [[local child=assert(loadstring('local a=1\nreturn a\n','=child'))
debug.sethook(hook,'l'); child()
local after=1
debug.sethook()]],
    'child:1,child:2,next-line-return:5,next-line-return:6')

-- A cache keyed only by Proto would conflate all three active invocations.
check('recursive-same-proto', [[local function f(n)
  if n==0 then return 0 end
  local x=f(n-1); x=x+1
  return x
end
debug.sethook(hook,'l'); assert(f(2)==2); debug.sethook()]],
    'recursive-same-proto:4,recursive-same-proto:5,' ..
    'recursive-same-proto:4,recursive-same-proto:5,' ..
    'recursive-same-proto:4,recursive-same-proto:6,recursive-same-proto:6')

-- Backward PC movement on the same line is observable, unlike forward progress.
check('one-line-backedge', [[local child=assert(loadstring('for i=1,4 do a=1 end','=loop'))
debug.sethook(hook,'l'); child(); debug.sethook()]],
    'loop:1,loop:1,loop:1,loop:1,loop:1')

-- Installing a hook seeds existing caller frames as well as the current frame.
check('installed-in-callee', [[local function install() debug.sethook(hook,'l') end
install(); local after=1; debug.sethook()]], '')

local nested_capture = [[local out={}; local calls=0; local function helper(value) calls=calls+1; return value end
local function hook(event,line) local info=debug.getinfo(2,'S'); assert(event=='line' and helper(line)==line); out[#out+1]=info.short_src..':'..line end
]]
check('hook-calls-lua', [[local child=assert(loadstring('local a=1\nreturn a\n','=child'))
debug.sethook(hook,'l'); child(); debug.sethook()]],
    'child:1,child:2', nested_capture, 2)

-- Reusing a physical frame for tail calls must still enter each callee's lines.
check('tailcall-reuse', [[local function f(n)
  if n==0 then return 7 end
  return f(n-1)
end
debug.sethook(hook,'l'); assert(f(2)==7); debug.sethook()]],
    'tailcall-reuse:4,tailcall-reuse:5,tailcall-reuse:4,' ..
    'tailcall-reuse:5,tailcall-reuse:4')

-- For these three straight-line instructions, count=1 dispatches count before
-- line at each instruction. A count callback must not consume the pending line
-- event, and callback instructions themselves must not trigger recursive hooks.
local count_source = [[local out={}
local function hook(event,line) local info=debug.getinfo(2,'S'); assert(event=='count' or event=='line'); if info.short_src=='count-child' then if event=='count' then assert(line==nil); out[#out+1]='count' else out[#out+1]='line:'..line end end end
local child=assert(loadstring('local x=0\nx=x+1\nreturn x\n','=count-child'))
debug.sethook(hook,'l',1); assert(child()==1); debug.sethook()
return table.concat(out,',')]]
local actual = assert(loadstring(count_source, '=count-and-line'))()
assert(actual == 'count,line:1,count,line:2,count,line:3', actual)
assert(debug.gethook() == nil)

local function recorder(out, label)
  return function(event, line)
    assert(event == 'line')
    local info = debug.getinfo(2, 'S')
    out[#out+1] = label .. info.short_src .. ':' .. line
  end
end

-- A host coroutine can configure another suspended coroutine. Replacing its
-- hook seeds the suspended frames; clearing it must stop subsequent callbacks.
do
  local out = {}
  local first, second = recorder(out, 'first:'), recorder(out, 'second:')
  local body = 'local x=0\ncoroutine.yield(x)\nx=x+1\n' ..
      'coroutine.yield(x)\nx=x+1\nreturn x\n'
  local co = coroutine.create(assert(loadstring(body, '=external-co')))
  debug.sethook(co, first, 'l')
  local hook, mask, count = debug.gethook(co)
  assert(hook == first and mask == 'l' and count == 0)
  local ok, value = coroutine.resume(co)
  assert(ok and value == 0)
  assert(table.concat(out, ',') == 'first:external-co:1,first:external-co:2')
  debug.sethook(co, second, 'l')
  assert(debug.gethook(co) == second)
  ok, value = coroutine.resume(co)
  assert(ok and value == 1)
  local expected = 'first:external-co:1,first:external-co:2,' ..
      'second:external-co:3,second:external-co:4'
  assert(table.concat(out, ',') == expected, table.concat(out, ','))
  debug.sethook(co)
  assert(debug.gethook(co) == nil)
  ok, value = coroutine.resume(co)
  assert(ok and value == 2 and coroutine.status(co) == 'dead')
  assert(table.concat(out, ',') == expected)
  assert(debug.gethook() == nil)
end

-- Explicit self targeting must work while the coroutine owns its active state
-- turn, including querying and clearing the same hook through that handle.
do
  local out = {}
  local hook = recorder(out, '')
  local body = [[local hook=...
local self=coroutine.running()
debug.sethook(self,hook,'l'); local installed,mask,count=debug.gethook(self); assert(installed==hook and mask=='l' and count==0); local x=1
x=x+1
debug.sethook(self); assert(debug.gethook(self)==nil)
return x]]
  local co = coroutine.create(assert(loadstring(body, '=self-co')))
  local ok, value = coroutine.resume(co, hook)
  assert(ok and value == 2, tostring(value))
  assert(coroutine.status(co) == 'dead')
  assert(table.concat(out, ',') == 'self-co:4,self-co:5', table.concat(out, ','))
  assert(debug.gethook(co) == nil and debug.gethook() == nil)
end

-- Replacing an active hook does not repeat its current line; a later restart
-- seeds the new setup location instead of reviving history from before stop.
do
  local out = {}
  local first, second = recorder(out, 'first:'), recorder(out, 'second:')
  local body = [[local first,second=...
debug.sethook(first,'l'); local x=0
x=x+1
debug.sethook(second,'l'); assert(debug.gethook()==second); x=x+1
x=x+1
debug.sethook(); x=x+1
x=x+1
debug.sethook(first,'l'); x=x+1
x=x+1
debug.sethook()
return x]]
  assert(assert(loadstring(body, '=hook-lifecycle'))(first, second) == 7)
  local expected = 'first:hook-lifecycle:3,first:hook-lifecycle:4,' ..
      'second:hook-lifecycle:5,second:hook-lifecycle:6,' ..
      'first:hook-lifecycle:9,first:hook-lifecycle:10'
  assert(table.concat(out, ',') == expected, table.concat(out, ','))
  assert(debug.gethook() == nil)
end

-- Replacing the callback from inside a hook retains the active-hook guard.
-- The replacement first observes the next instruction's line, not hook code.
do
  local out = {}
  local second = recorder(out, 'second:')
  local function first(event, line)
    assert(event == 'line')
    local info = debug.getinfo(2, 'S')
    out[#out+1] = 'first:' .. info.short_src .. ':' .. line
    debug.sethook(second, 'l')
  end
  local body = [[local hook=...
local child=assert(loadstring('local x=1\nreturn x\n','=replacement-child'))
debug.sethook(hook,'l'); assert(child()==1); debug.sethook()]]
  assert(loadstring(body, '=replace-inside-hook'))(first)
  local expected = 'first:replacement-child:1,second:replacement-child:2'
  assert(table.concat(out, ',') == expected, table.concat(out, ','))
  assert(debug.gethook() == nil)
end

print('debug line hooks passed')
