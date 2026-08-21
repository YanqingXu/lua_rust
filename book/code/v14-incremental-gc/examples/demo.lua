local log = {}
local weak = setmetatable({}, { __mode = "v" })

do
    weak.answer = {}
    local item = newuserdata()
    setfinalizer(item, function(value)
        log[#log + 1] = 7
        rescued = value
    end)
end

while not gc_step(1) do
end

local weak_value_gone = weak.answer == nil
local before_second_death = gc_live()
rescued = nil
gc_collect()
local died_later = gc_live() < before_second_death

return (weak_value_gone and died_later and log[1] == 7) and 714 or 0
