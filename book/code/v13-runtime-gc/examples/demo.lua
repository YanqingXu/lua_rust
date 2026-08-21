local reclaimed

do
    local cycle = {}
    cycle.self = cycle
end

reclaimed = gc_collect()

local getter
do
    local kept = { answer = 42 }
    kept.self = kept
    getter = function()
        return kept.answer
    end
end

gc_collect()

return reclaimed * 100 + getter()
