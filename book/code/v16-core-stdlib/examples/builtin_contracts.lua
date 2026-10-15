-- Predict the result before running: shared rules, installed IDs, one environment.
local a = math.abs('-9')
local b = math.sqrt('9')
local missing, missing_message = pcall(math.sqrt)
local explicit_nil, nil_message = pcall(math.sqrt, nil)
local binary = string.char(65, 0, 255)
local x, zero, ff = string.byte(binary, 1, 3)
answer = 42
local forward = _G.answer == 42
_G.answer = 43
local backward = answer == 43
answer = nil
local deleted = _G.answer == nil
local env = _G
_G = { answer = 99 }
local rebound = answer == nil and _G.answer == 99 and env._G == _G
_G = env
return a == 9 and b == 3 and not missing and not explicit_nil
    and missing_message ~= nil_message
    and x == 65 and zero == 0 and ff == 255
    and forward and backward and deleted and rebound and _G._G == _G
