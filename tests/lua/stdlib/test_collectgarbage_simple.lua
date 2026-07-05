-- Simple test for collectgarbage() function
print("=== Testing collectgarbage() ===")

-- Test 1: Count operation
print("Test 1: Count operation")
print("Memory usage (KB):", collectgarbage("count"))

-- Test 2: Collect operation
print("Test 2: Collect operation")
print("Result:", collectgarbage("collect"))

print("=== Tests complete ===")

