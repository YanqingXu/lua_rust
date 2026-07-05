-- ============================================================================
-- test_if.lua
-- Lua 解释器 if 语句全面测试脚本
--
-- 测试内容：
--   1. 基础 if 语句（if-then）
--   2. if-else 语句
--   3. if-elseif-else 语句（多个 elseif 分支）
--   4. 嵌套 if 语句（多层嵌套）
--   5. 带有逻辑运算符的 if 语句（and、or、not）
--   6. 带有比较运算符的 if 语句（==、~=、<、>、<=、>=）
--   7. 边界情况测试（nil、false、0、空字符串等）
-- ============================================================================

-- 测试计数器
local pass_count = 0
local fail_count = 0
local total_count = 0

-- 辅助函数：断言检查并输出结果
local function check(name, condition)
    total_count = total_count + 1
    if condition then
        pass_count = pass_count + 1
        print("  [✓] " .. name)
    else
        fail_count = fail_count + 1
        print("  [✗] " .. name)
    end
end

print("========================================")
print("  Lua if 语句全面测试")
print("========================================")

-- ============================================================================
-- 1. 基础 if 语句（if-then）
-- ============================================================================
print("\n--- 1. 基础 if 语句 ---")

-- 测试1.1：条件为 true 时执行 if 块
local r1 = false
if true then
    r1 = true
end
check("if true 执行 if 块", r1 == true)