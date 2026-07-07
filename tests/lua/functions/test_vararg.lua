-- test_vararg.lua: 可变参数（vararg）功能测试
-- 测试 Lua 5.1 vararg 的完整实现

print("===== Vararg Test Suite =====")
print("")

-- Test 1: 基本可变参数展开
print("--- Test 1: Basic vararg unpacking ---")
function test1(...)
    local a, b, c = ...
    print("a =", a)
    print("b =", b)
    print("c =", c)
end
test1(10, 20, 30)
print("")

-- Test 2: 可变参数数量少于局部变量
print("--- Test 2: Fewer varargs than locals ---")
function test2(...)
    local a, b, c = ...
    print("a =", a)
    print("b =", b)
    print("c =", c)  -- should be nil
end
test2(10, 20)
print("")

-- Test 3: 混合固定参数和可变参数
print("--- Test 3: Fixed + vararg ---")
function test3(x, y, ...)
    print("x =", x)
    print("y =", y)
    local a, b = ...
    print("a =", a)
    print("b =", b)
end
test3(1, 2, 3, 4)
print("")

-- Test 4: 可变参数打包成表 {...}
print("--- Test 4: Pack varargs into table ---")
function test4(...)
    local t = {...}
    print("t[1] =", t[1])
    print("t[2] =", t[2])
    print("t[3] =", t[3])
end
test4(100, 200, 300)
print("")

-- Test 5: select('#', ...) 获取可变参数数量
print("--- Test 5: select('#', ...) ---")
function test5(...)
    local n = select('#', ...)
    print("count =", n)
end
test5(10, 20, 30)
print("")

-- Test 6: select(i, ...) 索引访问
print("--- Test 6: select(i, ...) ---")
function test6(...)
    print("select(1) =", select(1, ...))
    print("select(2) =", select(2, ...))
    print("select(3) =", select(3, ...))
end
test6(10, 20, 30)
print("")

-- Test 7: 无可变参数
print("--- Test 7: No varargs passed ---")
function test7(...)
    local a = ...
    print("a =", a)  -- should be nil
end
test7()
print("")

-- Test 8: 可变参数传递
print("--- Test 8: Vararg forwarding ---")
function inner(...)
    local a, b = ...
    print("inner a =", a)
    print("inner b =", b)
end

function outer(...)
    inner(...)
end
outer(42, 99)
print("")

-- Test 9: 混合固定参数 + 无额外可变参数
print("--- Test 9: Fixed params, no extra varargs ---")
function test9(a, b, ...)
    print("a =", a)
    print("b =", b)
    local c = ...
    print("c =", c)  -- should be nil
end
test9(1, 2)
print("")

-- Test 10: 可变参数在 print 中直接使用
print("--- Test 10: Vararg in print ---")
function test10(...)
    print(...)
end
test10("hello", "world", 42)
print("")

print("===== All Tests Complete =====")

