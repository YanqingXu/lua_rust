-- Test break with counter
local i = 0
while true do
    i = i + 1
    print(i)
    if i == 3 then
        break
    end
end
print("done")

