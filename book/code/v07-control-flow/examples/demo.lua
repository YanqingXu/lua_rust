local total = 0

for value = 1, 10 do
    if value == 6 then
        break
    elseif value % 2 == 0 then
        total = total + value
    else
        total = total + 100
    end
end

return total
