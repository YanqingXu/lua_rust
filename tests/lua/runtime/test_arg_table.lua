-- 测试arg表功能
-- 用法：lua.exe test_arg_table.lua arg1 arg2 arg3

print("=== Test arg table ===")

-- 检查arg表是否存在
if arg then
    print("SUCCESS: arg table exists")
    print("arg type:", type(arg))

    -- 测试arg[0] (script name)
    print("\nTesting arg[0] (script name):")
    print("  type(arg[0]) =", type(arg[0]))
    print("  arg[0] =", arg[0])

    -- 测试arg[1] (first argument)
    print("\nTesting arg[1] (first argument):")
    print("  type(arg[1]) =", type(arg[1]))
    print("  arg[1] =", arg[1])

    -- 测试arg[2] (second argument)
    print("\nTesting arg[2] (second argument):")
    print("  type(arg[2]) =", type(arg[2]))
    print("  arg[2] =", arg[2])

    -- 测试arg[3] (third argument)
    print("\nTesting arg[3] (third argument):")
    print("  type(arg[3]) =", type(arg[3]))
    print("  arg[3] =", arg[3])

    print("\n=== All tests passed! ===")
else
    print("ERROR: arg table does not exist!")
end

