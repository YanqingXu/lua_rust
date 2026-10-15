local answer = 40
local payload = "\000\255\"\\\n"
if answer > 0 then
    answer = answer + 2
else
    answer = 0
end
return answer, payload
