-- 测试Lua 5.1.5语法糖功能

-- 测试1: 方法定义语法糖
-- function t:method() 等价于 function t.method(self)
local obj = {}

function obj:greet(name)
    return "Hello, " .. name .. "!"
end

-- 测试2: 表成员函数定义
local math_utils = {}

function math_utils.add(a, b)
    return a + b
end

function math_utils.multiply(a, b)
    return a * b
end

-- 测试3: 多级表成员函数定义
local lib = {}
lib.math = {}
lib.string = {}

function lib.math.square(x)
    return x * x
end

function lib.string.upper(s)
    return s  -- 简化版本
end

-- 测试4: 函数调用语法糖 - 字符串参数
function print_msg(msg)
    return msg
end

local result1 = print_msg"Hello, World!"

-- 测试5: 函数调用语法糖 - 表参数
function create_point(t)
    return t.x + t.y
end

local result2 = create_point{x=10, y=20}

-- 测试6: 表构造器 - 混合形式
local mixed_table = {
    1, 2, 3,           -- 数组元素
    name = "test",     -- 命名字段
    [10] = "ten",      -- 索引字段
    4, 5,              -- 更多数组元素
    key = "value"      -- 更多命名字段
}

-- 测试7: 表构造器 - 复杂表达式作为数组元素
local complex_table = {
    math_utils.add(1, 2),
    math_utils.multiply(3, 4),
    lib.math.square(5)
}

-- 测试8: 嵌套方法调用
local nested = {}
nested.inner = {}

function nested.inner:process(data)
    return data * 2
end

-- 测试9: 组合语法糖
function nested:transform(value)
    return self.inner:process(value)
end

-- 测试10: 局部函数（不支持表路径）
local function local_func(x)
    return x + 1
end

return {
    obj = obj,
    math_utils = math_utils,
    lib = lib,
    result1 = result1,
    result2 = result2,
    mixed_table = mixed_table,
    complex_table = complex_table,
    nested = nested,
    local_func = local_func
}

