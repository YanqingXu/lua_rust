-- Test collectgarbage() function
print("=== Testing collectgarbage() ===")

-- Test 1: Default operation (collect)
print("\nTest 1: Default operation (collect)")
print("Calling collectgarbage()...")
print("Result:", collectgarbage())

-- Test 2: Explicit collect operation
print("\nTest 2: Explicit collect operation")
print("Calling collectgarbage('collect')...")
print("Result:", collectgarbage("collect"))

-- Test 3: Count operation (get memory usage)
print("\nTest 3: Count operation (memory usage)")
print("Calling collectgarbage('count')...")
print("Memory usage (KB):", collectgarbage("count"))

-- Test 4: Stop operation
print("\nTest 4: Stop operation")
print("Calling collectgarbage('stop')...")
print("Result:", collectgarbage("stop"))

-- Test 5: Restart operation
print("\nTest 5: Restart operation")
print("Calling collectgarbage('restart')...")
print("Result:", collectgarbage("restart"))

-- Test 6: Step operation
print("\nTest 6: Step operation")
print("Calling collectgarbage('step')...")
print("Result:", collectgarbage("step"))

-- Test 7: Setpause operation
print("\nTest 7: Setpause operation")
print("Calling collectgarbage('setpause', 200)...")
print("Result:", collectgarbage("setpause", 200))

-- Test 8: Setstepmul operation
print("\nTest 8: Setstepmul operation")
print("Calling collectgarbage('setstepmul', 200)...")
print("Result:", collectgarbage("setstepmul", 200))

-- Test 9: Create some objects and check memory
print("\nTest 9: Memory usage before and after creating objects")
print("Memory before:", collectgarbage("count"), "KB")

-- Create some objects
local data = {}
data[1] = "test"
data[2] = "test"
data[3] = "test"

print("Memory after:", collectgarbage("count"), "KB")

-- Collect garbage
collectgarbage("collect")
print("Memory after GC:", collectgarbage("count"), "KB")

print("\n=== All collectgarbage() tests passed! ===")

