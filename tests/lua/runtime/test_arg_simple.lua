-- Simple arg table test (without break statement)
-- Test 1: Check arg table exists
print("=== Test 1: arg table exists ===")
if arg then
    print("PASS: arg table exists")
else
    print("FAIL: arg table does not exist")
end

-- Test 2: Check arg[-1] (interpreter name)
print("")
print("=== Test 2: arg[-1] (interpreter name) ===")
print("arg[-1] =", arg[-1])

-- Test 3: Check arg[0] (script name)
print("")
print("=== Test 3: arg[0] (script name) ===")
print("arg[0] =", arg[0])

-- Test 4: Check arg[1], arg[2], arg[3] (command-line arguments)
print("")
print("=== Test 4: Command-line arguments ===")
print("arg[1] =", arg[1])
print("arg[2] =", arg[2])
print("arg[3] =", arg[3])

-- Test 5: Print all arg entries
print("")
print("=== Test 5: All arg entries ===")
print("arg[-1] =", arg[-1])
print("arg[0] =", arg[0])
print("arg[1] =", arg[1])
print("arg[2] =", arg[2])
print("arg[3] =", arg[3])

