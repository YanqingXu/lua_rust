-- Debug I/O table
print("=== I/O Debug ===")

print("Type of io:", type(io))
print("Type of io.open:", type(io.open))

print("Calling io.open...")
local result = io.open("test.txt", "w")
print("Result type:", type(result))

print("=== Done ===")

