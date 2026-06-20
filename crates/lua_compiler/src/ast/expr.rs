//! Lua 抽象语法树 — 表达式节点定义
//!
//! 定义所有 Lua 5.1 表达式类型（14 种），用于表示解析后的表达式结构。
//!
//! C++ 参考: `lua_cpp/src/compiler/ast.hpp`

use crate::ast::stmt::Stmt;

// =====================================================================
// 源代码位置信息
// =====================================================================

/// 源代码位置信息
///
/// 所有 AST 节点共享的位置信息基类。
/// 用于错误报告、调试和代码生成时的位置追踪。
///
/// C++ 对应: `Lua::SourceLocation`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// 行号（1-based）
    pub line: i32,
    /// 列号（1-based）
    pub column: i32,
}

impl SourceLocation {
    pub const fn new(line: i32, column: i32) -> Self {
        Self { line, column }
    }
}

// =====================================================================
// 二元运算符
// =====================================================================

/// 二元运算类型
///
/// C++ 对应: `Lua::BinaryExpr::Op`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // 算术运算
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    // 比较运算
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    // 逻辑运算
    And,
    Or,
    // 字符串连接
    Concat,
}

// =====================================================================
// 一元运算符
// =====================================================================

/// 一元运算类型
///
/// C++ 对应: `Lua::UnaryExpr::Op`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// `not`
    Not,
    /// `-` 取负
    Neg,
    /// `#` 取长度
    Len,
}

// =====================================================================
// 表达式节点类型
// =====================================================================

/// nil 字面量
///
/// C++ 对应: `Lua::NilExpr`
#[derive(Debug, Clone)]
pub struct NilExpr {
    pub location: SourceLocation,
}

/// 布尔字面量
///
/// C++ 对应: `Lua::BoolExpr`
#[derive(Debug, Clone)]
pub struct BoolExpr {
    pub location: SourceLocation,
    pub value: bool,
}

/// 数字字面量
///
/// C++ 对应: `Lua::NumberExpr`
#[derive(Debug, Clone)]
pub struct NumberExpr {
    pub location: SourceLocation,
    pub value: f64,
}

/// 字符串字面量
///
/// C++ 对应: `Lua::StringExpr`
#[derive(Debug, Clone)]
pub struct StringExpr {
    pub location: SourceLocation,
    pub value: String,
}

/// 变长参数 `...`
///
/// C++ 对应: `Lua::VarargExpr`
#[derive(Debug, Clone)]
pub struct VarargExpr {
    pub location: SourceLocation,
}

/// 标识符（变量名）
///
/// C++ 对应: `Lua::NameExpr`
#[derive(Debug, Clone)]
pub struct NameExpr {
    pub location: SourceLocation,
    pub name: String,
}

/// 二元运算表达式
///
/// C++ 对应: `Lua::BinaryExpr`
#[derive(Debug, Clone)]
pub struct BinaryExpr {
    pub location: SourceLocation,
    pub op: BinaryOp,
    pub left: Box<Expr>,
    pub right: Box<Expr>,
}

/// 一元运算表达式
///
/// C++ 对应: `Lua::UnaryExpr`
#[derive(Debug, Clone)]
pub struct UnaryExpr {
    pub location: SourceLocation,
    pub op: UnaryOp,
    pub operand: Box<Expr>,
}

/// 表构造器字段
///
/// C++ 对应: `Lua::TableField`
#[derive(Debug, Clone)]
pub struct TableField {
    /// key = nil 表示数组部分（无键）
    pub key: Option<Box<Expr>>,
    pub value: Box<Expr>,
}

/// 表构造器表达式
///
/// C++ 对应: `Lua::TableExpr`
#[derive(Debug, Clone)]
pub struct TableExpr {
    pub location: SourceLocation,
    pub fields: Vec<TableField>,
}

/// 函数调用表达式
///
/// 支持两种调用方式：
/// - 普通调用：`func(args)`
/// - 方法调用：`obj:method(args)` - 等价于 `obj.method(obj, args)`
///
/// C++ 对应: `Lua::CallExpr`
#[derive(Debug, Clone)]
pub struct CallExpr {
    pub location: SourceLocation,
    pub func: Box<Expr>,
    pub args: Vec<Box<Expr>>,
    /// 是否为方法调用（使用冒号语法）
    pub is_method_call: bool,
}

/// 表索引访问 `table[key]`
///
/// C++ 对应: `Lua::IndexExpr`
#[derive(Debug, Clone)]
pub struct IndexExpr {
    pub location: SourceLocation,
    pub table: Box<Expr>,
    pub index: Box<Expr>,
}

/// 成员访问 `table.member`
///
/// C++ 对应: `Lua::MemberExpr`
#[derive(Debug, Clone)]
pub struct MemberExpr {
    pub location: SourceLocation,
    pub table: Box<Expr>,
    pub member: String,
}

/// 函数定义表达式
///
/// C++ 对应: `Lua::FunctionExpr`
#[derive(Debug, Clone)]
pub struct FunctionExpr {
    pub location: SourceLocation,
    pub params: Vec<String>,
    pub is_vararg: bool,
    pub body: Vec<Box<Stmt>>,
    /// 结束行号（0 表示未设置）
    pub end_line: i32,
}

/// 括号表达式
///
/// 需要保留括号语义以对齐 Lua 5.1：
/// `(exp)` 会将函数调用/vararg 的多返回值收敛为单值。
///
/// C++ 对应: `Lua::ParenExpr`
#[derive(Debug, Clone)]
pub struct ParenExpr {
    pub location: SourceLocation,
    pub expression: Box<Expr>,
}

// =====================================================================
// 表达式枚举（14 种变体，对应 C++ ExprVariant）
// =====================================================================

/// Lua 5.1 表达式枚举
///
/// 包含全部 14 种表达式节点类型。
/// 使用 `Box<Expr>` 实现递归所有权。
///
/// C++ 对应: `Lua::ExprVariant` (std::variant of 14 types)
#[derive(Debug, Clone)]
pub enum Expr {
    Nil(NilExpr),
    Boolean(BoolExpr),
    Number(NumberExpr),
    String(StringExpr),
    Vararg(VarargExpr),
    Name(NameExpr),
    Binary(BinaryExpr),
    Unary(UnaryExpr),
    Table(TableExpr),
    Call(CallExpr),
    Index(IndexExpr),
    Member(MemberExpr),
    Function(FunctionExpr),
    Paren(ParenExpr),
}

/// 表达式节点数量（编译期常量，用于验证）
pub const EXPR_NODE_COUNT: usize = 14;

impl Expr {
    /// 获取表达式的行号
    ///
    /// C++ 对应: `Lua::Expr::getLine()`
    pub fn line(&self) -> i32 {
        match self {
            Expr::Nil(e) => e.location.line,
            Expr::Boolean(e) => e.location.line,
            Expr::Number(e) => e.location.line,
            Expr::String(e) => e.location.line,
            Expr::Vararg(e) => e.location.line,
            Expr::Name(e) => e.location.line,
            Expr::Binary(e) => e.location.line,
            Expr::Unary(e) => e.location.line,
            Expr::Table(e) => e.location.line,
            Expr::Call(e) => e.location.line,
            Expr::Index(e) => e.location.line,
            Expr::Member(e) => e.location.line,
            Expr::Function(e) => e.location.line,
            Expr::Paren(e) => e.location.line,
        }
    }

    /// 获取表达式的列号
    ///
    /// C++ 对应: `Lua::Expr::getColumn()`
    pub fn column(&self) -> i32 {
        match self {
            Expr::Nil(e) => e.location.column,
            Expr::Boolean(e) => e.location.column,
            Expr::Number(e) => e.location.column,
            Expr::String(e) => e.location.column,
            Expr::Vararg(e) => e.location.column,
            Expr::Name(e) => e.location.column,
            Expr::Binary(e) => e.location.column,
            Expr::Unary(e) => e.location.column,
            Expr::Table(e) => e.location.column,
            Expr::Call(e) => e.location.column,
            Expr::Index(e) => e.location.column,
            Expr::Member(e) => e.location.column,
            Expr::Function(e) => e.location.column,
            Expr::Paren(e) => e.location.column,
        }
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(line: i32, col: i32) -> SourceLocation {
        SourceLocation::new(line, col)
    }

    #[test]
    fn test_expr_node_count() {
        // 确保表达式节点数量为 14
        assert_eq!(EXPR_NODE_COUNT, 14);
    }

    #[test]
    fn test_nil_expr() {
        let e = Expr::Nil(NilExpr {
            location: loc(1, 1),
        });
        assert_eq!(e.line(), 1);
        assert_eq!(e.column(), 1);
    }

    #[test]
    fn test_bool_expr() {
        let e = Expr::Boolean(BoolExpr {
            location: loc(2, 5),
            value: true,
        });
        assert_eq!(e.line(), 2);
        assert!(matches!(e, Expr::Boolean(ref b) if b.value));
    }

    #[test]
    fn test_number_expr() {
        let e = Expr::Number(NumberExpr {
            location: loc(1, 1),
            value: 42.0,
        });
        assert!(matches!(e, Expr::Number(ref n) if (n.value - 42.0).abs() < f64::EPSILON));
    }

    #[test]
    fn test_string_expr() {
        let e = Expr::String(StringExpr {
            location: loc(1, 1),
            value: "hello".to_string(),
        });
        assert!(matches!(e, Expr::String(ref s) if s.value == "hello"));
    }

    #[test]
    fn test_vararg_expr() {
        let e = Expr::Vararg(VarargExpr {
            location: loc(1, 1),
        });
        assert!(matches!(e, Expr::Vararg(_)));
    }

    #[test]
    fn test_name_expr() {
        let e = Expr::Name(NameExpr {
            location: loc(1, 1),
            name: "foo".to_string(),
        });
        assert!(matches!(e, Expr::Name(ref n) if n.name == "foo"));
    }

    #[test]
    fn test_binary_expr() {
        let left = Box::new(Expr::Number(NumberExpr {
            location: loc(1, 1),
            value: 1.0,
        }));
        let right = Box::new(Expr::Number(NumberExpr {
            location: loc(1, 5),
            value: 2.0,
        }));
        let e = Expr::Binary(BinaryExpr {
            location: loc(1, 3),
            op: BinaryOp::Add,
            left,
            right,
        });
        assert_eq!(e.line(), 1);
        assert_eq!(e.column(), 3);
        assert!(matches!(e, Expr::Binary(ref b) if b.op == BinaryOp::Add));
    }

    #[test]
    fn test_unary_expr() {
        let operand = Box::new(Expr::Number(NumberExpr {
            location: loc(1, 2),
            value: 42.0,
        }));
        let e = Expr::Unary(UnaryExpr {
            location: loc(1, 1),
            op: UnaryOp::Neg,
            operand,
        });
        assert!(matches!(e, Expr::Unary(ref u) if u.op == UnaryOp::Neg));
    }

    #[test]
    fn test_table_expr_empty() {
        let e = Expr::Table(TableExpr {
            location: loc(1, 1),
            fields: vec![],
        });
        assert!(matches!(e, Expr::Table(ref t) if t.fields.is_empty()));
    }

    #[test]
    fn test_table_expr_with_fields() {
        let value = Box::new(Expr::Number(NumberExpr {
            location: loc(1, 5),
            value: 42.0,
        }));
        let e = Expr::Table(TableExpr {
            location: loc(1, 1),
            fields: vec![TableField { key: None, value }],
        });
        assert!(matches!(e, Expr::Table(ref t) if t.fields.len() == 1));
    }

    #[test]
    fn test_call_expr() {
        let func = Box::new(Expr::Name(NameExpr {
            location: loc(1, 1),
            name: "print".to_string(),
        }));
        let arg = Box::new(Expr::String(StringExpr {
            location: loc(1, 7),
            value: "hello".to_string(),
        }));
        let e = Expr::Call(CallExpr {
            location: loc(1, 1),
            func,
            args: vec![arg],
            is_method_call: false,
        });
        assert_eq!(e.line(), 1);
        assert!(!matches!(e, Expr::Call(ref c) if c.is_method_call));
    }

    #[test]
    fn test_index_expr() {
        let table = Box::new(Expr::Name(NameExpr {
            location: loc(1, 1),
            name: "t".to_string(),
        }));
        let index = Box::new(Expr::String(StringExpr {
            location: loc(1, 3),
            value: "key".to_string(),
        }));
        let e = Expr::Index(IndexExpr {
            location: loc(1, 1),
            table,
            index,
        });
        assert_eq!(e.line(), 1);
    }

    #[test]
    fn test_member_expr() {
        let table = Box::new(Expr::Name(NameExpr {
            location: loc(1, 1),
            name: "t".to_string(),
        }));
        let e = Expr::Member(MemberExpr {
            location: loc(1, 1),
            table,
            member: "key".to_string(),
        });
        assert!(matches!(e, Expr::Member(ref m) if m.member == "key"));
    }

    #[test]
    fn test_function_expr() {
        let e = Expr::Function(FunctionExpr {
            location: loc(1, 1),
            params: vec!["a".to_string(), "b".to_string()],
            is_vararg: false,
            body: vec![],
            end_line: 3,
        });
        assert!(matches!(e, Expr::Function(ref f) if f.params.len() == 2));
        assert!(matches!(e, Expr::Function(ref f) if !f.is_vararg));
        assert!(matches!(e, Expr::Function(ref f) if f.end_line == 3));
    }

    #[test]
    fn test_paren_expr() {
        let inner = Box::new(Expr::Number(NumberExpr {
            location: loc(1, 2),
            value: 42.0,
        }));
        let e = Expr::Paren(ParenExpr {
            location: loc(1, 1),
            expression: inner,
        });
        assert!(matches!(e, Expr::Paren(_)));
    }

    #[test]
    fn test_all_expr_node_types_coverage() {
        // 确保所有 14 种变体都可以构造
        let _ = Expr::Nil(NilExpr {
            location: loc(0, 0),
        });
        let _ = Expr::Boolean(BoolExpr {
            location: loc(0, 0),
            value: false,
        });
        let _ = Expr::Number(NumberExpr {
            location: loc(0, 0),
            value: 0.0,
        });
        let _ = Expr::String(StringExpr {
            location: loc(0, 0),
            value: String::new(),
        });
        let _ = Expr::Vararg(VarargExpr {
            location: loc(0, 0),
        });
        let _ = Expr::Name(NameExpr {
            location: loc(0, 0),
            name: String::new(),
        });
        let _ = Expr::Binary(BinaryExpr {
            location: loc(0, 0),
            op: BinaryOp::Add,
            left: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            right: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        });
        let _ = Expr::Unary(UnaryExpr {
            location: loc(0, 0),
            op: UnaryOp::Not,
            operand: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        });
        let _ = Expr::Table(TableExpr {
            location: loc(0, 0),
            fields: vec![],
        });
        let _ = Expr::Call(CallExpr {
            location: loc(0, 0),
            func: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            args: vec![],
            is_method_call: false,
        });
        let _ = Expr::Index(IndexExpr {
            location: loc(0, 0),
            table: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            index: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        });
        let _ = Expr::Member(MemberExpr {
            location: loc(0, 0),
            table: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            member: String::new(),
        });
        let _ = Expr::Function(FunctionExpr {
            location: loc(0, 0),
            params: vec![],
            is_vararg: false,
            body: vec![],
            end_line: 0,
        });
        let _ = Expr::Paren(ParenExpr {
            location: loc(0, 0),
            expression: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        });
    }
}
