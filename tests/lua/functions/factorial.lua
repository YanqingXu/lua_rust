-- 阶乘递归测试脚本
-- 测试递归函数调用

function factorial(n)
    if n <= 1 then
        return 1
    end
    return n * factorial(n - 1)
end

print(factorial(5))
return factorial(5)

