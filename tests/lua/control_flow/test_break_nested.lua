-- Test break in nested loops
print("=== Nested Loop Break Test ===")

local outer = 0
while outer < 3 do
    outer = outer + 1
    print("Outer:", outer)
    
    local inner = 0
    while inner < 5 do
        inner = inner + 1
        print("  Inner:", inner)
        
        if inner == 2 then
            print("  Breaking inner loop")
            break
        end
    end
    
    print("After inner loop")
end

print("=== Test Complete ===")

