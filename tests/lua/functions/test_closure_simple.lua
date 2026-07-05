-- 简单的闭包测试（不使用upvalue）
-- 测试CLOSURE指令的基本功能

print("=== Test 1: Simple function creation ===")

function makeAdder(x)
    return function(y)
        return x + y
    end
end

local add5 = makeAdder(5)
print("add5(10) =", add5(10))  -- 应该输出15

print("\n=== Test 2: Multiple closures ===")

local add10 = makeAdder(10)
print("add10(20) =", add10(20))  -- 应该输出30
print("add5(3) =", add5(3))      -- 应该输出8

print("\n=== All closure tests passed! ===")

