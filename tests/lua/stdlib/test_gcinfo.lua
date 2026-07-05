-- Test gcinfo() function
print("=== Testing gcinfo() ===")

-- Test 1: Basic functionality
print("Test 1: Basic functionality")
print("Memory usage (KB):", gcinfo())

-- Test 2: Multiple calls
print("Test 2: Multiple calls")
print("Call 1:", gcinfo(), "KB")
print("Call 2:", gcinfo(), "KB")
print("Call 3:", gcinfo(), "KB")

print("=== All gcinfo() tests passed! ===")

