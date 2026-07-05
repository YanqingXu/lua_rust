-- Test break in for loop
for i = 1, 10 do
    print(i)
    if i == 3 then
        break
    end
end
print("done")

