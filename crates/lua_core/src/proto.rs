//! Lua 函数原型（Proto）— 编译后函数的完整元数据
//!
//! `Proto` 包含 Lua 函数编译后的所有信息：字节码、常量表、嵌套函数原型、
//! 调试信息（行号映射、局部变量名称与生命周期、上值名称）等。
//!
//! ## 核心组成
//! 1. **字节码序列** (`code`): 函数的可执行指令数组
//! 2. **常量池** (`constants`): nil、布尔、数值、字符串及嵌套函数等常量
//! 3. **子函数原型** (`sub_protos`): 函数体内定义的嵌套函数
//! 4. **行号信息** (`line_info`): 字节码到源码行号的映射
//! 5. **局部变量信息** (`locvars`): 变量名称与生命周期（调试用）
//! 6. **上值名称** (`upvalue_names`): 闭包捕获的外部变量名称（调试用）
//! 7. **元数据**: 源文件名、参数个数、可变参数标志、栈大小等
//!
//! 中的 `Lua::Proto` 类。

use std::collections::HashMap;

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::gc_string::GcString;
use crate::types::{GcObjectType, LuaNumber};
use crate::value::Value;

// =====================================================================
// 类型别名
// =====================================================================

/// Lua 虚拟机指令（32 位无符号整数）
///
pub type Instruction = u32;

// =====================================================================
// VARARG 标志常量（Lua 5.1 兼容）
// =====================================================================

/// 有参数标志：函数有实际的可变参数
pub const VARARG_HASARG: u8 = 1;

/// 是可变参数函数：函数声明时使用了 `...` 语法
pub const VARARG_ISVARARG: u8 = 2;

/// 需要参数：函数需要可变参数（旧式兼容）
pub const VARARG_NEEDSARG: u8 = 4;

// =====================================================================
// ConstantKey — 常量去重键类型
// =====================================================================

/// 常量去重键，用于在编译期对常量进行哈希去重。
///
/// 参考 Lua 5.1 的 `addk()` 实现。支持 nil、bool、number、string
/// 四种常量类型。string 使用 `GcString` 的预计算哈希值。
///
#[derive(Debug, Clone)]
pub enum ConstantKey {
    /// nil 常量
    Nil,
    /// 布尔常量
    Boolean(bool),
    /// 数值常量（使用 f64 的位模式以确保 NaN 等边界情况的一致性）
    Number(LuaNumber),
    /// 字符串常量（使用 GcRef 指针身份）
    String(GcRef<GcString>),
}

impl From<&Value> for ConstantKey {
    fn from(value: &Value) -> Self {
        match value {
            Value::Nil => ConstantKey::Nil,
            Value::Boolean(b) => ConstantKey::Boolean(*b),
            Value::Number(n) => ConstantKey::Number(*n),
            Value::String(s) => ConstantKey::String(*s),
            // 非常量类型不参与去重
            _ => ConstantKey::Nil,
        }
    }
}

impl std::hash::Hash for ConstantKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            ConstantKey::Nil => 0_i32.hash(state),
            ConstantKey::Boolean(b) => b.hash(state),
            ConstantKey::Number(n) => n.to_bits().hash(state),
            ConstantKey::String(s) => {
                // SAFETY: GC single-threaded model ensures the GcString is alive;
                // reading the precomputed hash is a pure read.
                let h = unsafe { s.as_ref() }.map(|gs| gs.hash()).unwrap_or(0);
                h.hash(state);
            }
        }
    }
}

impl PartialEq for ConstantKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ConstantKey::Nil, ConstantKey::Nil) => true,
            (ConstantKey::Boolean(a), ConstantKey::Boolean(b)) => a == b,
            (ConstantKey::Number(a), ConstantKey::Number(b)) => a.to_bits() == b.to_bits(),
            (ConstantKey::String(a), ConstantKey::String(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ConstantKey {}

// =====================================================================
// LocVar — 局部变量调试信息
// =====================================================================

/// 局部变量信息 — 存储函数中局部变量的调试元数据
///
/// 用途：
/// - 调试器显示变量名称
/// - 错误报告中包含变量信息
/// - 反射和元编程支持
///
/// 生命周期：
/// - `startpc`: 变量开始有效的字节码位置
/// - `endpc`: 变量失效的字节码位置（不包含）
///
#[derive(Debug, Clone)]
pub struct LocVar {
    /// 变量名称（驻留字符串）
    pub varname: Option<GcRef<GcString>>,
    /// 起始 PC：变量开始有效的字节码位置
    pub startpc: i32,
    /// 结束 PC：变量失效的字节码位置（不包含）
    pub endpc: i32,
    /// 对应的寄存器槽位（相对于当前栈帧 base）
    pub reg: i32,
}

impl LocVar {
    /// 创建新的局部变量信息
    pub fn new(varname: Option<GcRef<GcString>>, startpc: i32, endpc: i32, reg: i32) -> Self {
        Self {
            varname,
            startpc,
            endpc,
            reg,
        }
    }
}

impl Default for LocVar {
    fn default() -> Self {
        Self {
            varname: None,
            startpc: 0,
            endpc: 0,
            reg: -1,
        }
    }
}

// =====================================================================
// Proto 结构体
// =====================================================================

/// Lua 函数原型 — 编译后函数的完整元数据
///
/// Proto 包含字节码序列、常量池、嵌套函数原型、调试信息等。
/// 是 Lua 虚拟机执行的基础数据结构。
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - constants: Vec<Value> (24 bytes)
/// - constant_map: HashMap<ConstantKey, usize> (~56 bytes)
/// - code: Vec<Instruction> (24 bytes)
/// - sub_protos: Vec<GcRef<Proto>> (24 bytes)
/// - line_info: Vec<i32> (24 bytes)
/// - locvars: Vec<LocVar> (24 bytes)
/// - upvalue_names: Vec<GcRef<GcString>> (24 bytes)
/// - source: Option<GcRef<GcString>> (8 bytes)
/// - 元数据字段 (8 bytes + 4 bytes padding)
///
#[repr(C)]
pub struct Proto {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    // ── 核心数据结构 ──────────────────────────────────────────────
    /// 常量表：函数使用的常量值数组
    constants: Vec<Value>,

    /// 常量去重缓存：从常量键到常量表索引的映射
    /// 参考 Lua 5.1 中 `addk()` 使用的哈希表（`fs->h`）
    constant_map: HashMap<ConstantKey, usize>,

    /// 字节码数组：函数的指令序列
    code: Vec<Instruction>,

    /// 子函数原型数组：函数内定义的嵌套函数
    sub_protos: Vec<GcRef<Proto>>,

    /// 行号信息：字节码到源码行号的映射（每条指令对应一个行号）
    line_info: Vec<i32>,

    /// 局部变量信息：调试用的局部变量描述
    locvars: Vec<LocVar>,

    /// 上值名称数组：闭包变量的名称（用于调试）
    upvalue_names: Vec<GcRef<GcString>>,

    // ── 元数据字段 ────────────────────────────────────────────────
    /// 源文件名：函数所在的源文件
    source: Option<GcRef<GcString>>,

    /// 函数定义开始行号
    linedefined: i32,

    /// 函数定义结束行号
    lastlinedefined: i32,

    /// GC 链表指针：用于垃圾回收遍历子函数原型
    /// 预留给增量 GC 和分代 GC
    gclist: Option<GcRef<Proto>>,

    // ── 函数签名信息（字节类型）──────────────────────────────────
    /// 上值数量：函数引用的外部变量个数
    nups: u8,

    /// 参数数量：函数的固定参数个数
    num_params: u8,

    /// 可变参数标志：函数是否接受可变数量的参数
    /// 对应 Lua 5.1 的 VARARG_HASARG 和 VARARG_ISVARARG 标志
    is_vararg: u8,

    /// 最大栈大小：函数执行时需要的最大栈空间
    max_stack_size: u8,
}

impl Proto {
    /// 创建新的函数原型
    ///
    pub fn new() -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Proto),
            constants: Vec::new(),
            constant_map: HashMap::new(),
            code: Vec::new(),
            sub_protos: Vec::new(),
            line_info: Vec::new(),
            locvars: Vec::new(),
            upvalue_names: Vec::new(),
            source: None,
            linedefined: 0,
            lastlinedefined: 0,
            gclist: None,
            nups: 0,
            num_params: 0,
            is_vararg: 0,
            max_stack_size: 0,
        }
    }

    // ── 基本属性访问 ──────────────────────────────────────────────

    /// 获取参数数量
    #[inline]
    pub fn num_params(&self) -> u8 {
        self.num_params
    }

    /// 设置参数数量
    #[inline]
    pub fn set_num_params(&mut self, n: u8) {
        self.num_params = n;
    }

    /// 是否为可变参数函数
    ///
    /// 注意：Lua 5.1 使用位标志表示可变参数特性：
    /// - VARARG_HASARG (1): 函数有实际的可变参数
    /// - VARARG_ISVARARG (2): 函数声明时使用了 `...` 语法
    #[inline]
    pub fn is_vararg(&self) -> bool {
        self.is_vararg != 0
    }

    /// 获取可变参数标志（原始值）
    #[inline]
    pub fn vararg_flags(&self) -> u8 {
        self.is_vararg
    }

    /// 设置可变参数标志（使用 VARARG_ISVARARG = 2）
    #[inline]
    pub fn set_vararg(&mut self, vararg: bool) {
        self.is_vararg = if vararg { VARARG_ISVARARG } else { 0 };
    }

    /// 设置可变参数标志（原始值）
    #[inline]
    pub fn set_vararg_flags(&mut self, flags: u8) {
        self.is_vararg = flags;
    }

    /// 获取最大栈大小
    #[inline]
    pub fn max_stack_size(&self) -> u8 {
        self.max_stack_size
    }

    /// 设置最大栈大小
    #[inline]
    pub fn set_max_stack_size(&mut self, size: u8) {
        self.max_stack_size = size;
    }

    /// 获取源文件名
    #[inline]
    pub fn source(&self) -> Option<GcRef<GcString>> {
        self.source
    }

    /// 设置源文件名
    #[inline]
    pub fn set_source(&mut self, src: Option<GcRef<GcString>>) {
        self.source = src;
    }

    // ── 常量表操作 ────────────────────────────────────────────────

    /// 添加常量（带去重）
    ///
    /// 对于常量类型（nil/bool/number/string），先在哈希表中查找：
    /// - 如果找到相同的常量，直接返回已有索引，避免重复存储
    /// - 如果未找到，添加新常量并记录到哈希表中
    ///
    /// 非常量类型（table/function 等）不参与去重，直接添加。
    ///
    pub fn add_constant(&mut self, value: Value) -> usize {
        // 仅对常量类型执行去重
        if matches!(
            value,
            Value::Nil | Value::Boolean(_) | Value::Number(_) | Value::String(_)
        ) {
            let key = ConstantKey::from(&value);
            if let Some(&index) = self.constant_map.get(&key) {
                return index;
            }
            let index = self.constants.len();
            self.constants.push(value);
            self.constant_map.insert(key, index);
            return index;
        }

        // 非常量类型直接添加
        let index = self.constants.len();
        self.constants.push(value);
        index
    }

    /// 按原始槽位追加常量（不去重）
    ///
    /// binary chunk 反序列化必须保留 dump 时的常量表索引，
    /// 不能像编译期 `add_constant()` 一样对常量去重。
    ///
    pub fn append_constant_slot(&mut self, value: Value) -> usize {
        let index = self.constants.len();
        self.constants.push(value);

        if matches!(
            &self.constants[index],
            Value::Nil | Value::Boolean(_) | Value::Number(_) | Value::String(_)
        ) {
            let key = ConstantKey::from(&self.constants[index]);
            // 使用 entry().or_insert() 保留首次插入的常量槽位:
            // 如果 key 已存在则不覆盖（保留首次写入的索引）
            self.constant_map.entry(key).or_insert(index);
        }

        index
    }

    /// 获取常量
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    pub fn constant(&self, index: usize) -> &Value {
        &self.constants[index]
    }

    /// 获取常量数量
    #[inline]
    pub fn constant_count(&self) -> usize {
        self.constants.len()
    }

    /// 获取常量表（只读）
    #[inline]
    pub fn constants(&self) -> &[Value] {
        &self.constants
    }

    // ── 字节码操作 ────────────────────────────────────────────────

    /// 添加指令
    ///
    pub fn add_instruction(&mut self, inst: Instruction) -> usize {
        self.code.push(inst);
        self.code.len() - 1
    }

    /// 获取指令
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    #[inline]
    pub fn instruction(&self, index: usize) -> Instruction {
        self.code[index]
    }

    /// 设置指令
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    #[inline]
    pub fn set_instruction(&mut self, index: usize, inst: Instruction) {
        self.code[index] = inst;
    }

    /// 获取指令数量
    #[inline]
    pub fn instruction_count(&self) -> usize {
        self.code.len()
    }

    /// 获取代码数组（只读）
    #[inline]
    pub fn code(&self) -> &[Instruction] {
        &self.code
    }

    /// 获取代码数组（可写）
    #[inline]
    pub fn code_mut(&mut self) -> &mut Vec<Instruction> {
        &mut self.code
    }

    // ── 行号信息 ──────────────────────────────────────────────────

    /// 添加行号信息
    ///
    pub fn add_line_info(&mut self, line: i32) {
        self.line_info.push(line);
    }

    /// 获取指令对应的行号
    ///
    pub fn line(&self, pc: usize) -> i32 {
        self.line_info.get(pc).copied().unwrap_or(0)
    }

    /// 获取行号信息数组（只读）
    #[inline]
    pub fn line_info(&self) -> &[i32] {
        &self.line_info
    }

    /// 获取行号信息数组（可写）
    #[inline]
    pub fn line_info_mut(&mut self) -> &mut Vec<i32> {
        &mut self.line_info
    }

    // ── 子函数原型管理 ────────────────────────────────────────────

    /// 添加子函数原型
    ///
    pub fn add_proto(&mut self, proto: GcRef<Proto>) -> usize {
        self.sub_protos.push(proto);
        self.sub_protos.len() - 1
    }

    /// 获取子函数原型
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    #[inline]
    pub fn sub_proto(&self, index: usize) -> GcRef<Proto> {
        self.sub_protos[index]
    }

    /// 获取子函数数量
    #[inline]
    pub fn sub_proto_count(&self) -> usize {
        self.sub_protos.len()
    }

    // ── 局部变量信息管理 ──────────────────────────────────────────

    /// 添加局部变量信息
    ///
    pub fn add_loc_var(
        &mut self,
        varname: Option<GcRef<GcString>>,
        startpc: i32,
        endpc: i32,
        reg: i32,
    ) -> usize {
        self.locvars.push(LocVar::new(varname, startpc, endpc, reg));
        self.locvars.len() - 1
    }

    /// 获取局部变量信息
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    #[inline]
    pub fn loc_var(&self, index: usize) -> &LocVar {
        &self.locvars[index]
    }

    /// 获取局部变量数量
    #[inline]
    pub fn loc_var_count(&self) -> usize {
        self.locvars.len()
    }

    /// 获取指定 PC 位置的局部变量信息
    ///
    /// `local_number`: 局部变量编号（从 1 开始）
    /// `pc`: 程序计数器位置
    ///
    pub fn local_var_info(&self, local_number: i32, pc: i32) -> Option<&LocVar> {
        let mut remaining = local_number;

        for locvar in &self.locvars {
            if locvar.startpc > pc {
                break;
            }
            if pc < locvar.endpc {
                remaining -= 1;
                if remaining == 0 {
                    return Some(locvar);
                }
            }
        }

        None
    }

    // ── 上值名称管理 ──────────────────────────────────────────────

    /// 添加上值名称
    ///
    pub fn add_upvalue_name(&mut self, name: GcRef<GcString>) -> usize {
        self.upvalue_names.push(name);
        self.upvalue_names.len() - 1
    }

    /// 获取上值名称
    ///
    #[inline]
    pub fn upvalue_name(&self, index: usize) -> Option<GcRef<GcString>> {
        self.upvalue_names.get(index).copied()
    }

    /// 获取上值名称数量
    #[inline]
    pub fn upvalue_name_count(&self) -> usize {
        self.upvalue_names.len()
    }

    // ── 函数定义位置信息 ──────────────────────────────────────────

    /// 获取函数定义开始行号
    #[inline]
    pub fn line_defined(&self) -> i32 {
        self.linedefined
    }

    /// 设置函数定义开始行号
    #[inline]
    pub fn set_line_defined(&mut self, line: i32) {
        self.linedefined = line;
    }

    /// 获取函数定义结束行号
    #[inline]
    pub fn last_line_defined(&self) -> i32 {
        self.lastlinedefined
    }

    /// 设置函数定义结束行号
    #[inline]
    pub fn set_last_line_defined(&mut self, line: i32) {
        self.lastlinedefined = line;
    }

    // ── 上值数量管理 ──────────────────────────────────────────────

    /// 获取上值数量
    #[inline]
    pub fn num_upvalues(&self) -> u8 {
        self.nups
    }

    /// 设置上值数量
    #[inline]
    pub fn set_num_upvalues(&mut self, n: u8) {
        self.nups = n;
    }

    // ── GC 链表管理 ───────────────────────────────────────────────

    /// 获取 GC 链表指针
    #[inline]
    pub fn gc_list(&self) -> Option<GcRef<Proto>> {
        self.gclist
    }

    /// 设置 GC 链表指针
    #[inline]
    pub fn set_gc_list(&mut self, list: Option<GcRef<Proto>>) {
        self.gclist = list;
    }
}

impl Default for Proto {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Proto 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Proto。
// mark_children 完整标记所有被引用的 GC 对象。
unsafe impl GcObject for Proto {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记 Proto 引用的所有 GC 对象
    ///
    /// 标记路径：
    /// 1. 源文件名 (`source`)
    /// 2. 常量表中的 GC 对象（String、Table、Function、Userdata、Thread）
    /// 3. 所有子函数原型
    /// 4. 所有局部变量的变量名
    /// 5. 所有上值名称
    ///
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        // 1. 标记源文件名
        if let Some(source_ref) = self.source {
            // SAFETY: source_ref is a valid GcRef held by this Proto;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(source_ref.as_ptr() as *mut GcObjectHeader);
            }
        }

        // 2. 标记常量表中的 GC 对象
        for val in &self.constants {
            // SAFETY: all GcRef pointers in constants_ are valid;
            // collector is valid during mark phase.
            unsafe {
                match val {
                    Value::String(s) => {
                        collector.mark_object(s.as_ptr() as *mut GcObjectHeader);
                    }
                    Value::Table(t) => {
                        collector.mark_object(t.as_ptr() as *mut GcObjectHeader);
                    }
                    Value::Function(f) => {
                        collector.mark_object(f.as_ptr() as *mut GcObjectHeader);
                    }
                    Value::Userdata(u) => {
                        collector.mark_object(u.as_ptr() as *mut GcObjectHeader);
                    }
                    Value::Thread(t) => {
                        collector.mark_object(t.as_ptr() as *mut GcObjectHeader);
                    }
                    _ => {}
                }
            }
        }

        // 3. 标记所有子函数原型
        for sub_proto in &self.sub_protos {
            // SAFETY: sub_proto is a valid GcRef<Proto> held by this Proto;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(sub_proto.as_ptr() as *mut GcObjectHeader);
            }
        }

        // 4. 标记局部变量名称
        for locvar in &self.locvars {
            if let Some(name_ref) = locvar.varname {
                // SAFETY: name_ref is a valid GcRef<GcString> held by this LocVar;
                // collector is valid during mark phase.
                unsafe {
                    collector.mark_object(name_ref.as_ptr() as *mut GcObjectHeader);
                }
            }
        }

        // 5. 标记上值名称
        for name in &self.upvalue_names {
            // SAFETY: name is a valid GcRef<GcString> held by this Proto;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(name.as_ptr() as *mut GcObjectHeader);
            }
        }
    }

    fn get_size(&self) -> usize {
        // 基础大小 + 所有动态数组的容量
        std::mem::size_of::<Self>()
            + self.constants.capacity() * std::mem::size_of::<Value>()
            + self.code.capacity() * std::mem::size_of::<Instruction>()
            + self.line_info.capacity() * std::mem::size_of::<i32>()
            + self.sub_protos.capacity() * std::mem::size_of::<GcRef<Proto>>()
            + self.locvars.capacity() * std::mem::size_of::<LocVar>()
            + self.upvalue_names.capacity() * std::mem::size_of::<GcRef<GcString>>()
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Proto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proto")
            .field("num_params", &self.num_params)
            .field("is_vararg", &self.is_vararg)
            .field("max_stack_size", &self.max_stack_size)
            .field("instructions", &self.code.len())
            .field("constants", &self.constants.len())
            .field("sub_protos", &self.sub_protos.len())
            .field("locvars", &self.locvars.len())
            .field("upvalue_names", &self.upvalue_names.len())
            .field("nups", &self.nups)
            .field("linedefined", &self.linedefined)
            .field("lastlinedefined", &self.lastlinedefined)
            .finish()
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_ref::GcRef;
    use crate::string_pool::StringPool;
    use crate::table::Table;

    // ── 创建测试 ──────────────────────────────────────────────────

    #[test]
    fn test_proto_new_defaults() {
        let p = Proto::new();
        assert_eq!(p.num_params(), 0);
        assert_eq!(p.max_stack_size(), 0);
        assert_eq!(p.instruction_count(), 0);
        assert_eq!(p.constant_count(), 0);
        assert_eq!(p.sub_proto_count(), 0);
        assert_eq!(p.loc_var_count(), 0);
        assert_eq!(p.upvalue_name_count(), 0);
        assert_eq!(p.num_upvalues(), 0);
        assert_eq!(p.line_defined(), 0);
        assert_eq!(p.last_line_defined(), 0);
        assert!(!p.is_vararg());
        assert!(p.source().is_none());
    }

    #[test]
    fn test_proto_basic_properties() {
        let mut p = Proto::new();
        p.set_num_params(3);
        p.set_max_stack_size(10);
        p.set_num_upvalues(2);
        p.set_line_defined(5);
        p.set_last_line_defined(20);
        p.set_vararg(true);
        p.set_vararg_flags(VARARG_HASARG | VARARG_ISVARARG);

        assert_eq!(p.num_params(), 3);
        assert_eq!(p.max_stack_size(), 10);
        assert_eq!(p.num_upvalues(), 2);
        assert_eq!(p.line_defined(), 5);
        assert_eq!(p.last_line_defined(), 20);
        assert!(p.is_vararg());
        assert_eq!(p.vararg_flags(), VARARG_HASARG | VARARG_ISVARARG);
    }

    // ── 常量表操作 ────────────────────────────────────────────────

    #[test]
    fn test_add_constant_dedup() {
        let mut p = Proto::new();

        let idx1 = p.add_constant(Value::Number(42.0));
        let idx2 = p.add_constant(Value::Number(42.0)); // 重复 — 应返回同一索引
        let idx3 = p.add_constant(Value::Number(3.14));

        assert_eq!(idx1, idx2);
        assert_ne!(idx1, idx3);
        assert_eq!(p.constant_count(), 2); // 仅 42.0 和 3.14
    }

    #[test]
    fn test_add_constant_nil_dedup() {
        let mut p = Proto::new();

        let idx1 = p.add_constant(Value::Nil);
        let idx2 = p.add_constant(Value::Nil);

        assert_eq!(idx1, idx2);
        assert_eq!(p.constant_count(), 1);
    }

    #[test]
    fn test_add_constant_bool_dedup() {
        let mut p = Proto::new();

        let idx1 = p.add_constant(Value::Boolean(true));
        let idx2 = p.add_constant(Value::Boolean(false));
        let idx3 = p.add_constant(Value::Boolean(true)); // 重复

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(idx3, 0); // 返回已有索引
        assert_eq!(p.constant_count(), 2);
    }

    #[test]
    fn test_add_constant_string_dedup() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern(&mut gc, "hello");
        let s2 = pool.intern(&mut gc, "hello"); // 同一驻留字符串

        let mut p = Proto::new();
        let idx1 = p.add_constant(Value::String(s1));
        let idx2 = p.add_constant(Value::String(s2));

        assert_eq!(idx1, idx2); // 同一驻留字符串 → 去重
        assert_eq!(p.constant_count(), 1);
    }

    #[test]
    fn test_add_constant_non_const_no_dedup() {
        let mut gc = GarbageCollector::new();
        let mut p = Proto::new();

        // Table 非常量 — 不参与去重
        let t1 = gc.create(Table::new());
        let t2 = gc.create(Table::new());

        let idx1 = p.add_constant(Value::Table(t1));
        let idx2 = p.add_constant(Value::Table(t2));

        assert_ne!(idx1, idx2); // 每次都是新的
        assert_eq!(p.constant_count(), 2);
    }

    #[test]
    fn test_append_constant_slot() {
        let mut p = Proto::new();

        // append_constant_slot 不去重
        let idx1 = p.append_constant_slot(Value::Number(1.0));
        let idx2 = p.append_constant_slot(Value::Number(1.0));

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1); // 不去重 → 新索引
        assert_eq!(p.constant_count(), 2);

        // 但后续 add_constant 会去重（因为 append_constant_slot 也注册了去重键）
        let idx3 = p.add_constant(Value::Number(1.0));
        assert_eq!(idx3, 0); // 返回第一个匹配的索引
    }

    #[test]
    fn test_get_constant() {
        let mut p = Proto::new();
        p.add_constant(Value::Number(1.0));
        p.add_constant(Value::Boolean(true));
        p.add_constant(Value::Nil);

        assert_eq!(*p.constant(0), Value::Number(1.0));
        assert_eq!(*p.constant(1), Value::Boolean(true));
        assert_eq!(*p.constant(2), Value::Nil);
    }

    #[test]
    fn test_constants_slice() {
        let mut p = Proto::new();
        p.add_constant(Value::Number(1.0));
        p.add_constant(Value::Number(2.0));

        let slice = p.constants();
        assert_eq!(slice.len(), 2);
        assert_eq!(slice[0], Value::Number(1.0));
    }

    // ── 字节码操作 ────────────────────────────────────────────────

    #[test]
    fn test_add_get_instruction() {
        let mut p = Proto::new();

        let idx = p.add_instruction(0x12345678);
        assert_eq!(idx, 0);
        assert_eq!(p.instruction(0), 0x12345678);
        assert_eq!(p.instruction_count(), 1);
    }

    #[test]
    fn test_set_instruction() {
        let mut p = Proto::new();
        p.add_instruction(0);
        p.set_instruction(0, 0xABCDEF00);
        assert_eq!(p.instruction(0), 0xABCDEF00);
    }

    #[test]
    fn test_code_slice() {
        let mut p = Proto::new();
        p.add_instruction(1);
        p.add_instruction(2);
        p.add_instruction(3);

        let code = p.code();
        assert_eq!(code, &[1, 2, 3]);
    }

    #[test]
    fn test_code_mut() {
        let mut p = Proto::new();
        p.add_instruction(0);
        p.code_mut()[0] = 42;
        assert_eq!(p.instruction(0), 42);
    }

    // ── 行号信息 ──────────────────────────────────────────────────

    #[test]
    fn test_line_info() {
        let mut p = Proto::new();

        p.add_line_info(1);
        p.add_line_info(3);
        p.add_line_info(5);

        assert_eq!(p.line(0), 1);
        assert_eq!(p.line(1), 3);
        assert_eq!(p.line(2), 5);
        assert_eq!(p.line(999), 0); // 越界返回 0
    }

    #[test]
    fn test_line_info_slice_and_mut() {
        let mut p = Proto::new();
        p.add_line_info(10);
        p.add_line_info(20);

        assert_eq!(p.line_info(), &[10, 20]);

        p.line_info_mut().push(30);
        assert_eq!(p.line(2), 30);
    }

    // ── 子函数原型 ────────────────────────────────────────────────

    #[test]
    fn test_sub_protos() {
        let mut gc = GarbageCollector::new();
        let mut p = Proto::new();

        let sub1 = gc.create(Proto::new());
        let sub2 = gc.create(Proto::new());

        let idx1 = p.add_proto(sub1);
        let idx2 = p.add_proto(sub2);

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(p.sub_proto_count(), 2);
        assert_eq!(p.sub_proto(0), sub1);
        assert_eq!(p.sub_proto(1), sub2);
    }

    // ── 局部变量信息 ──────────────────────────────────────────────

    #[test]
    fn test_loc_vars() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let name1 = pool.intern(&mut gc, "x");
        let name2 = pool.intern(&mut gc, "y");

        let mut p = Proto::new();
        let idx1 = p.add_loc_var(Some(name1), 0, 5, 0);
        let idx2 = p.add_loc_var(Some(name2), 2, 8, 1);

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(p.loc_var_count(), 2);

        let lv1 = p.loc_var(0);
        assert_eq!(lv1.varname, Some(name1));
        assert_eq!(lv1.startpc, 0);
        assert_eq!(lv1.endpc, 5);
        assert_eq!(lv1.reg, 0);

        let lv2 = p.loc_var(1);
        assert_eq!(lv2.varname, Some(name2));
        assert_eq!(lv2.reg, 1);
    }

    #[test]
    fn test_local_var_info_lookup() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let name_x = pool.intern(&mut gc, "x");
        let name_y = pool.intern(&mut gc, "y");

        let mut p = Proto::new();
        // x: PC 0..10, reg 0  (活跃范围最广)
        p.add_loc_var(Some(name_x), 0, 10, 0);
        // y: PC 3..7, reg 1   (活跃范围较小)
        p.add_loc_var(Some(name_y), 3, 7, 1);

        // PC=5: 两个变量都活跃 → local #1 = x, local #2 = y
        let info = p.local_var_info(1, 5);
        assert!(info.is_some());
        assert_eq!(info.unwrap().varname, Some(name_x));

        let info2 = p.local_var_info(2, 5);
        assert!(info2.is_some());
        assert_eq!(info2.unwrap().varname, Some(name_y));

        // local #3 不存在
        assert!(p.local_var_info(3, 5).is_none());

        // PC=8: 只有 x 活跃 (y 在 PC=7 结束)
        assert!(p.local_var_info(1, 8).is_some());
        assert!(p.local_var_info(2, 8).is_none());
    }

    // ── 上值名称 ──────────────────────────────────────────────────

    #[test]
    fn test_upvalue_names() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let uv1 = pool.intern(&mut gc, "outer_var");
        let uv2 = pool.intern(&mut gc, "counter");

        let mut p = Proto::new();
        let idx1 = p.add_upvalue_name(uv1);
        let idx2 = p.add_upvalue_name(uv2);

        assert_eq!(idx1, 0);
        assert_eq!(idx2, 1);
        assert_eq!(p.upvalue_name_count(), 2);
        assert_eq!(p.upvalue_name(0), Some(uv1));
        assert_eq!(p.upvalue_name(1), Some(uv2));
        assert_eq!(p.upvalue_name(999), None);
    }

    // ── GC 类型测试 ───────────────────────────────────────────────

    #[test]
    fn test_proto_gc_header_type() {
        let p = Proto::new();
        assert_eq!(p.gc_header().gc_type(), GcObjectType::Proto);
    }

    #[test]
    fn test_proto_gc_create_and_register() {
        let mut gc = GarbageCollector::new();
        let p = Proto::new();
        let p_ref: GcRef<Proto> = gc.create(p);

        assert!(!p_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    // ── GC 标记测试 ───────────────────────────────────────────────

    #[test]
    fn test_proto_mark_constants() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s_ref = pool.intern(&mut gc, "constant_string");

        let mut p = Proto::new();
        p.add_constant(Value::String(s_ref));
        let p_ref = gc.create(p);

        gc.reset_marks();

        // 标记 proto
        unsafe {
            let p_ptr = p_ref.as_ptr();
            (*p_ptr).mark_children(&mut gc);
        }

        // 字符串应被标记
        let s_header = s_ref.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*s_header).is_white(), "String constant should be marked");
        }
    }

    #[test]
    fn test_proto_mark_sub_protos() {
        let mut gc = GarbageCollector::new();

        let sub = gc.create(Proto::new());
        let mut p = Proto::new();
        p.add_proto(sub);
        let p_ref = gc.create(p);

        gc.reset_marks();

        unsafe {
            let p_ptr = p_ref.as_ptr();
            (*p_ptr).mark_children(&mut gc);
        }

        // 子 proto 应被标记
        let sub_header = sub.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*sub_header).is_white(), "Sub-proto should be marked");
        }
    }

    #[test]
    fn test_proto_mark_loc_var_names() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let name = pool.intern(&mut gc, "local_x");

        let mut p = Proto::new();
        p.add_loc_var(Some(name), 0, 10, 0);
        let p_ref = gc.create(p);

        gc.reset_marks();

        unsafe {
            let p_ptr = p_ref.as_ptr();
            (*p_ptr).mark_children(&mut gc);
        }

        let name_header = name.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*name_header).is_white(), "LocVar name should be marked");
        }
    }

    #[test]
    fn test_proto_mark_upvalue_names() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let uv_name = pool.intern(&mut gc, "captured_var");

        let mut p = Proto::new();
        p.add_upvalue_name(uv_name);
        let p_ref = gc.create(p);

        gc.reset_marks();

        unsafe {
            let p_ptr = p_ref.as_ptr();
            (*p_ptr).mark_children(&mut gc);
        }

        let name_header = uv_name.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*name_header).is_white(), "Upvalue name should be marked");
        }
    }

    #[test]
    fn test_proto_mark_source() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let source_name = pool.intern(&mut gc, "test.lua");

        let mut p = Proto::new();
        p.set_source(Some(source_name));
        let p_ref = gc.create(p);

        gc.reset_marks();

        unsafe {
            let p_ptr = p_ref.as_ptr();
            (*p_ptr).mark_children(&mut gc);
        }

        let src_header = source_name.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*src_header).is_white(), "Source should be marked");
        }
    }

    // ── GC 回收测试 ───────────────────────────────────────────────

    #[test]
    fn test_proto_swept_when_unreachable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create(Proto::new());
        assert_eq!(gc.object_count(), 1);

        // Proto 不是根 → 保持白色 → 被回收
        gc.mark();
        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 0);
    }

    #[test]
    fn test_proto_kept_when_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create_root(Proto::new());
        assert_eq!(gc.object_count(), 1);

        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 1);
    }

    // ── get_size ──────────────────────────────────────────────────

    #[test]
    fn test_proto_get_size() {
        let p = Proto::new();
        let size = p.get_size();
        assert!(size >= std::mem::size_of::<Proto>());
    }

    #[test]
    fn test_proto_get_size_grows_with_content() {
        let mut p = Proto::new();

        let size_empty = p.get_size();

        // 添加大量数据
        for i in 0..100 {
            p.add_instruction(i as Instruction);
            p.add_constant(Value::Number(i as f64));
            p.add_line_info(i as i32);
        }

        let size_full = p.get_size();
        assert!(size_full > size_empty, "Size should increase with content");
    }

    // ── Debug 输出 ────────────────────────────────────────────────

    #[test]
    fn test_proto_debug() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let name = pool.intern(&mut gc, "local_x");

        let mut p = Proto::new();
        p.set_num_params(2);
        p.set_max_stack_size(5);
        p.add_instruction(0xABCD);
        p.add_constant(Value::Number(42.0));
        p.add_loc_var(Some(name), 0, 10, 0);

        let debug_str = format!("{:?}", p);
        assert!(debug_str.contains("num_params"));
        assert!(debug_str.contains("instructions"));
        assert!(debug_str.contains("constants"));
        assert!(debug_str.contains("locvars"));
    }

    // ── VARARG 常量 ───────────────────────────────────────────────

    #[test]
    fn test_vararg_constants() {
        assert_eq!(VARARG_HASARG, 1);
        assert_eq!(VARARG_ISVARARG, 2);
        assert_eq!(VARARG_NEEDSARG, 4);
    }

    // ── Default trait ─────────────────────────────────────────────

    #[test]
    fn test_proto_default_equals_new() {
        let p1 = Proto::new();
        let p2 = Proto::default();
        assert_eq!(p1.num_params(), p2.num_params());
        assert_eq!(p1.instruction_count(), p2.instruction_count());
        assert_eq!(p1.constant_count(), p2.constant_count());
    }

    // ── gc_list 管理 ──────────────────────────────────────────────

    #[test]
    fn test_proto_gc_list() {
        let mut gc = GarbageCollector::new();
        let mut p = Proto::new();

        assert!(p.gc_list().is_none());

        let sub = gc.create(Proto::new());
        p.set_gc_list(Some(sub));
        assert_eq!(p.gc_list(), Some(sub));
    }
}
