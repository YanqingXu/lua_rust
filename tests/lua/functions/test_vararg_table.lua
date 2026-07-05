-- 测试表构造器中的vararg展开
function test_table_pack(...)
    local t = {...}
    print("t[1] =", t[1])
    print("t[2] =", t[2])
    print("t[3] =", t[3])
end

print("=== Test: Table Pack ===")
test_table_pack(100, 200, 300)

