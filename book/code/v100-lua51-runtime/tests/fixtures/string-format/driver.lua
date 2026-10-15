-- Fixed calls are independent of cases.tsv. Never derive expected bytes from this program.
local F = string.format
local cases = {
  ["literal"] = function() return F('plain') end,
  ["percent-no-consume"] = function() return F('%%:%d',7) end,
  ["default-f"] = function() return F('%f',1.5) end,
  ["negative-zero-f"] = function() return F('%f',-1/math.huge) end,
  ["plus-f"] = function() return F('%+f',1.5) end,
  ["space-f"] = function() return F('% f',1.5) end,
  ["zero-width-f"] = function() return F('%010.2f',-1.5) end,
  ["left-width-f"] = function() return F('%-10.2f',1.5) end,
  ["empty-precision-f"] = function() return F('%.f',1.4) end,
  ["alternate-f"] = function() return F('%#.0f',1) end,
  ["signed-integers"] = function() return F('%d:%i',7,-7) end,
  ["zero-width-d"] = function() return F('%04d',7) end,
  ["plus-zero-d"] = function() return F('%+05d',7) end,
  ["space-width-d"] = function() return F('% 5d',7) end,
  ["left-width-d"] = function() return F('%-5d',7) end,
  ["precision-suppresses-zero"] = function() return F('%05.3d',7) end,
  ["zero-empty-precision-d"] = function() return F('%.0d',0) end,
  ["negative-integer-precision"] = function() return F('%06.3i',-7) end,
  ["truncate-integers"] = function() return F('%d:%i',7.9,-7.9) end,
  ["numeric-string-coercion"] = function() return F('%d:%f','0x10',' 1.5 ') end,
  ["multi-flag-precedence"] = function() return F('%+- 05d',7) end,
  ["repeat-flag-allowed"] = function() return F('%++d',7) end,
  ["short-string-nul"] = function() return F('%s','A'..string.char(0,255)..'B') end,
  ["string-precision"] = function() return F('%.3s','abcdef') end,
  ["string-width-precision"] = function() return F('%5.2s','abcdef') end,
  ["string-left-width"] = function() return F('%-5s','A'..string.char(255)) end,
  ["number-string-fourteen-digits"] = function() return F('%s',1.234567890123456) end,
  ["number-string-exponents"] = function() return F('%s:%q',1e20,1e-5) end,
  ["number-string-negative-zero"] = function() return F('%s',-1/math.huge) end,
  ["string-length-ninety-nine"] = function() return F('%s',string.rep('a',98)..string.char(0)) end,
  ["string-length-one-hundred"] = function() return F('%s',string.rep('a',98)..string.char(0)..'Z') end,
  ["long-string-width-fastpath"] = function() return F('%99s',string.rep('a',98)..string.char(0)..'Z') end,
  ["long-string-precision-no-fastpath"] = function() return F('%.99s',string.rep('a',98)..string.char(0)..'Z') end,
  ["quote-binary"] = function() return F('%q',string.char(0,255,10,13,34,92)) end,
  ["quote-roundtrip"] = function() local original=string.char(0,255,10,13,34,92); local quoted=F('%q',original); local loaded=assert(loadstring('return '..quoted)); return loaded()==original and 'roundtrip' or 'bad' end,
  ["quote-flags-ignored"] = function() return F('%+05.2q','ab') end,
  ["string-table-rejected"] = function() return F('%s',{}) end,
  ["string-boolean-rejected"] = function() return F('%s',false) end,
  ["quote-boolean-rejected"] = function() return F('%q',true) end,
  ["integer-table-rejected"] = function() return F('%d',{}) end,
  ["float-nil-rejected"] = function() return F('%f',nil) end,
  ["missing-value"] = function() return F('%s') end,
  ["missing-invalid-value-priority"] = function() return F('%z') end,
  ["invalid-option"] = function() return F('%z',1) end,
  ["width-too-long"] = function() return F('%100d',1) end,
  ["precision-too-long"] = function() return F('%.100f',1) end,
  ["six-flags-rejected"] = function() return F('%------d',1) end,
  ["trailing-percent"] = function() return F('%',1) end,
  ["percent-unused-argument"] = function() return F('%%',{}) end,
  ["format-nul-literal"] = function() return F('A'..string.char(0)..'%s','B') end,
  ["format-number-small"] = function() return F(1e-5) end,
  ["format-number-large"] = function() return F(1e14) end,
  ["format-table-rejected"] = function() return F({}) end,
  ["format-boolean-rejected"] = function() return F(false) end,
  ["format-nil-rejected"] = function() return F(nil) end,
  ["format-missing"] = function() return F() end,
  ["format-nul-then-integer"] = function() return F('A'..string.char(0)..'%d',7) end,
}

-- Avoid C stdout text-mode LF translation: encode the value bytes before io.write.
-- This encoder uses byte/sub/arithmetic, not the formatter under test.
local function emit_hex(value)
  local digits = '0123456789abcdef'
  for index = 1, #value do
    local byte = string.byte(value, index)
    local hi = math.floor(byte / 16) + 1
    local lo = byte % 16 + 1
    io.write(string.sub(digits, hi, hi), string.sub(digits, lo, lo))
  end
end

local call = assert(cases[arg[1]], 'unknown fixed format case')
local ok, value = pcall(call)
if ok then
  io.write('ok|')
  emit_hex(value)
else
  io.write('error')
end
io.write('|after')
