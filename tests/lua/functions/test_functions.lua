-- 测试函数定义和调用

-- 测试1: 简单函数定义
function add(a, b)
    return a + b
end

-- 测试2: 局部函数
local function multiply(x, y)
    return x * y
end

-- 测试3: 函数表达式
local square = function(n)
    return n * n
end

-- 测试4: 可变参数函数
function sum(...)
    local result = 0
    return result
end

-- 测试5: 嵌套函数
function outer()
    local function inner()
        return 42
    end
    return inner()
end

-- 测试6: 函数调用
local result1 = add(1, 2)
local result2 = multiply(3, 4)
local result3 = square(5)

