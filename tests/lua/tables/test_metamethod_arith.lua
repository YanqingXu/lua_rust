--[[
    算术元方法测试脚本
    
    测试内容：
    1. 基本数字运算（无元方法）
    2. __add元方法
    3. __sub元方法
    4. __mul元方法
    5. __unm元方法（一元负号）
    6. 元方法回退机制
]]

print("=== Metamethod Arithmetic Tests ===")

-- =====================================================================
-- 测试1：基本数字运算（无元方法）
-- =====================================================================

print("\n--- Test 1: Basic number arithmetic ---")

local a = 10
local b = 3

print("a = " .. a)
print("b = " .. b)
print("a + b = " .. (a + b))  -- 应该输出 13
print("a - b = " .. (a - b))  -- 应该输出 7
print("a * b = " .. (a * b))  -- 应该输出 30
print("a / b = " .. (a / b))  -- 应该输出 3.333...
print("a % b = " .. (a % b))  -- 应该输出 1
print("a ^ b = " .. (a ^ b))  -- 应该输出 1000
print("-a = " .. (-a))        -- 应该输出 -10

-- =====================================================================
-- 测试2：向量加法（__add元方法）
-- =====================================================================

print("\n--- Test 2: Vector addition with __add ---")

-- 定义向量元表
local VectorMT = {}

-- __add元方法：向量加法
VectorMT.__add = function(v1, v2)
    return {x = v1.x + v2.x, y = v1.y + v2.y}
end

-- __sub元方法：向量减法
VectorMT.__sub = function(v1, v2)
    return {x = v1.x - v2.x, y = v1.y - v2.y}
end

-- __mul元方法：向量数乘
VectorMT.__mul = function(v, scalar)
    -- 处理两种情况：v * scalar 和 scalar * v
    if type(v) == "number" then
        v, scalar = scalar, v
    end
    return {x = v.x * scalar, y = v.y * scalar}
end

-- __unm元方法：向量取负
VectorMT.__unm = function(v)
    return {x = -v.x, y = -v.y}
end

-- 创建向量构造函数
function Vector(x, y)
    local v = {x = x, y = y}
    setmetatable(v, VectorMT)
    return v
end

-- 创建两个向量
local v1 = Vector(3, 4)
local v2 = Vector(1, 2)

print("v1 = {" .. v1.x .. ", " .. v1.y .. "}")
print("v2 = {" .. v2.x .. ", " .. v2.y .. "}")

-- 测试__add
local v3 = v1 + v2
print("v1 + v2 = {" .. v3.x .. ", " .. v3.y .. "}")  -- 应该输出 {4, 6}

-- 测试__sub
local v4 = v1 - v2
print("v1 - v2 = {" .. v4.x .. ", " .. v4.y .. "}")  -- 应该输出 {2, 2}

-- 测试__mul
local v5 = v1 * 2
print("v1 * 2 = {" .. v5.x .. ", " .. v5.y .. "}")   -- 应该输出 {6, 8}

-- 测试__unm
local v6 = -v1
print("-v1 = {" .. v6.x .. ", " .. v6.y .. "}")      -- 应该输出 {-3, -4}

-- =====================================================================
-- 测试3：元方法回退机制
-- =====================================================================

print("\n--- Test 3: Metamethod fallback ---")

-- 创建一个只有左操作数有元方法的情况
local LeftMT = {}
LeftMT.__add = function(a, b)
    print("Left __add called")
    return a.value + b
end

local left = {value = 10}
setmetatable(left, LeftMT)

local result = left + 5
print("left + 5 = " .. result)  -- 应该调用左操作数的__add

-- 创建一个只有右操作数有元方法的情况
local RightMT = {}
RightMT.__add = function(a, b)
    print("Right __add called")
    return a + b.value
end

local right = {value = 20}
setmetatable(right, RightMT)

local result2 = 5 + right
print("5 + right = " .. result2)  -- 应该调用右操作数的__add

-- =====================================================================
-- 测试4：错误处理
-- =====================================================================

print("\n--- Test 4: Error handling ---")

-- 尝试对没有元方法的表进行算术运算
local t1 = {x = 1}
local t2 = {x = 2}

-- 这应该抛出错误
local success, err = pcall(function()
    local result = t1 + t2
end)

if not success then
    print("Expected error: " .. err)
else
    print("ERROR: Should have thrown an error!")
end

print("\n=== All tests completed ===")

