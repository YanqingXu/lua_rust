//! 值栈 (Stack)
//!
//! 动态扩展的 Lua 值栈，用于存储函数参数、局部变量和临时值。
//!

use lua_core::value::Value;

/// 初始栈大小
const INITIAL_STACK_SIZE: usize = 64;
/// 栈增长余量
const STACK_GROW_MARGIN: usize = 32;

/// Lua 值栈
///
/// 管理 Lua 值的动态栈，支持 push/pop 操作和自动扩展。
///
#[derive(Debug)]
pub struct Stack {
    values: Vec<Value>,
    top: usize,
}

impl Stack {
    pub fn new(initial_size: usize) -> Self {
        Self {
            values: Vec::with_capacity(initial_size),
            top: 0,
        }
    }

    pub fn with_default() -> Self {
        Self::new(INITIAL_STACK_SIZE)
    }

    /// 栈中元素数量
    pub fn size(&self) -> usize {
        self.top
    }

    /// 栈是否为空
    pub fn is_empty(&self) -> bool {
        self.top == 0
    }

    /// 栈容量
    pub fn capacity(&self) -> usize {
        self.values.capacity()
    }

    /// 压入值到栈顶
    pub fn push(&mut self, value: Value) {
        if self.top >= self.values.len() {
            self.values.push(value);
        } else {
            self.values[self.top] = value;
        }
        self.top += 1;
    }

    /// 弹出栈顶值
    pub fn pop(&mut self) -> Option<Value> {
        if self.top == 0 {
            return None;
        }
        self.top -= 1;
        Some(std::mem::replace(&mut self.values[self.top], Value::Nil))
    }

    /// 返回栈顶值（不弹出）
    pub fn top_value(&self) -> Option<&Value> {
        if self.top == 0 {
            return None;
        }
        Some(&self.values[self.top - 1])
    }

    /// 返回栈顶值（可写，不弹出）
    pub fn top_value_mut(&mut self) -> Option<&mut Value> {
        if self.top == 0 {
            return None;
        }
        Some(&mut self.values[self.top - 1])
    }

    /// 按绝对索引访问
    pub fn at(&self, index: usize) -> Option<&Value> {
        self.values.get(index)
    }

    /// 按绝对索引访问（可写）
    pub fn at_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.values.get_mut(index)
    }

    /// 不检查边界的索引访问
    pub fn get_unchecked(&self, index: usize) -> &Value {
        &self.values[index]
    }

    /// 不检查边界的可写索引访问
    pub fn get_unchecked_mut(&mut self, index: usize) -> &mut Value {
        &mut self.values[index]
    }

    /// 清空栈
    pub fn clear(&mut self) {
        self.top = 0;
    }

    /// 设置栈顶
    pub fn set_top(&mut self, new_top: usize) {
        if new_top > self.values.len() {
            self.values.resize(new_top, Value::Nil);
        }
        self.top = new_top;
    }

    /// 确保栈有足够空间
    pub fn ensure_space(&mut self, needed: usize) {
        let required = self.top + needed;
        if required > self.values.capacity() {
            self.values
                .reserve(required + STACK_GROW_MARGIN - self.values.len());
        }
    }

    /// 检查栈空间（批量操作前）
    pub fn check_space(&mut self, needed: usize) {
        let required = self.top + needed;
        if required > self.values.len() {
            self.values.resize(required, Value::Nil);
        }
    }

    /// 获取内部 Vec 的可变引用（用于批量操作）
    pub fn as_mut_slice(&mut self) -> &mut [Value] {
        if self.top > self.values.len() {
            self.values.resize(self.top, Value::Nil);
        }
        &mut self.values[..self.top]
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::with_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_pop() {
        let mut s = Stack::new(4);
        s.push(Value::Number(42.0));
        s.push(Value::Boolean(true));
        assert_eq!(s.size(), 2);
        assert_eq!(s.pop(), Some(Value::Boolean(true)));
        assert_eq!(s.pop(), Some(Value::Number(42.0)));
        assert_eq!(s.pop(), None);
    }

    #[test]
    fn test_top() {
        let mut s = Stack::new(4);
        s.push(Value::Number(1.0));
        assert!(matches!(s.top_value(), Some(Value::Number(v)) if (*v - 1.0).abs() < f64::EPSILON));
    }

    #[test]
    fn test_ensure_space() {
        let mut s = Stack::new(2);
        s.ensure_space(100);
        assert!(s.capacity() >= 100);
    }
}
