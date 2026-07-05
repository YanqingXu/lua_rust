-- Test successful function call
print("=== Test 1: Simple call ===")
local f = io.open("test.txt", "w")
print("Type:", type(f))
print("Value:", f)

print("\n=== Test 2: Multiple assignment ===")
local a, b = io.open("test2.txt", "w"), "hello"
print("a:", type(a), a)
print("b:", type(b), b)

print("\n=== SUCCESS ===")

