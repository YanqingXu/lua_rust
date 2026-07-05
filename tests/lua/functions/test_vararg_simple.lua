-- 简单的 vararg 测试
function test1(...)
    local a, b, c = ...
    print("a =", a)
    print("b =", b)
    print("c =", c)
end

print("=== Test 1 ===")
test1(10, 20, 30)

