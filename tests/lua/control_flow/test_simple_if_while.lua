-- Minimal test: while loop with if statement (no break)
local i = 0
while i < 2 do
    i = i + 1
    if i == 1 then
        print("one")
    end
    print("loop")
end
print("done")

