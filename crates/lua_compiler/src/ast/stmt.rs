//! Lua 抽象语法树 — 语句节点定义
//!
//! 定义所有 Lua 5.1 语句类型（13 种），用于表示解析后的语句结构。
//!
//! C++ 参考: `lua_cpp/src/compiler/ast.hpp`

use crate::ast::expr::{Expr, SourceLocation};

// =====================================================================
// If 语句分支
// =====================================================================

/// if/elseif 分支
///
/// 包含条件表达式和对应的语句体。
///
/// C++ 对应: `Lua::IfStmt::Branch`
#[derive(Debug, Clone)]
pub struct IfBranch {
    pub condition: Box<Expr>,
    pub body: Vec<Box<Stmt>>,
}

// =====================================================================
// 语句节点类型
// =====================================================================

/// 空语句
///
/// C++ 对应: `Lua::EmptyStmt`
#[derive(Debug, Clone)]
pub struct EmptyStmt {
    pub location: SourceLocation,
}

/// 赋值语句 `targets = values`
///
/// C++ 对应: `Lua::AssignStmt`
#[derive(Debug, Clone)]
pub struct AssignStmt {
    pub location: SourceLocation,
    /// 左值列表
    pub targets: Vec<Box<Expr>>,
    /// 右值列表
    pub values: Vec<Box<Expr>>,
}

/// 局部变量声明 `local names = values`
///
/// C++ 对应: `Lua::LocalStmt`
#[derive(Debug, Clone)]
pub struct LocalStmt {
    pub location: SourceLocation,
    pub names: Vec<String>,
    pub values: Vec<Box<Expr>>,
}

/// 函数调用语句
///
/// C++ 对应: `Lua::CallStmt`
#[derive(Debug, Clone)]
pub struct CallStmt {
    pub location: SourceLocation,
    pub call: Box<Expr>,
}

/// if 语句
///
/// C++ 对应: `Lua::IfStmt`
#[derive(Debug, Clone)]
pub struct IfStmt {
    pub location: SourceLocation,
    /// if 和 elseif 分支（至少有一个）
    pub branches: Vec<IfBranch>,
    /// else 分支（可选）
    pub else_branch: Vec<Box<Stmt>>,
    /// 结束行号
    pub end_line: i32,
}

/// while 循环
///
/// C++ 对应: `Lua::WhileStmt`
#[derive(Debug, Clone)]
pub struct WhileStmt {
    pub location: SourceLocation,
    pub condition: Box<Expr>,
    pub body: Vec<Box<Stmt>>,
    /// 结束行号
    pub end_line: i32,
}

/// repeat-until 循环
///
/// C++ 对应: `Lua::RepeatStmt`
#[derive(Debug, Clone)]
pub struct RepeatStmt {
    pub location: SourceLocation,
    pub body: Vec<Box<Stmt>>,
    pub condition: Box<Expr>,
    /// 结束行号
    pub end_line: i32,
}

/// 数值 for 循环 `for name = init, limit[, step] do body end`
///
/// C++ 对应: `Lua::ForNumStmt`
#[derive(Debug, Clone)]
pub struct ForNumStmt {
    pub location: SourceLocation,
    pub var: String,
    pub init: Box<Expr>,
    pub limit: Box<Expr>,
    /// 可选，默认为 1
    pub step: Option<Box<Expr>>,
    pub body: Vec<Box<Stmt>>,
    /// 结束行号
    pub end_line: i32,
}

/// 泛型 for 循环 `for vars in iterators do body end`
///
/// C++ 对应: `Lua::ForInStmt`
#[derive(Debug, Clone)]
pub struct ForInStmt {
    pub location: SourceLocation,
    /// 变量名列表
    pub vars: Vec<String>,
    /// 迭代器表达式列表
    pub iterators: Vec<Box<Expr>>,
    pub body: Vec<Box<Stmt>>,
    /// 结束行号
    pub end_line: i32,
}

/// 函数定义语句
///
/// 支持以下形式：
/// - `function foo() end` — 简单函数
/// - `function t.a.b.c.foo() end` — 表成员函数
/// - `function t:method() end` — 方法定义（自动添加 self 参数）
///
/// C++ 对应: `Lua::FunctionStmt`
#[derive(Debug, Clone)]
pub struct FunctionStmt {
    pub location: SourceLocation,
    /// 基础函数名
    pub name: String,
    /// 表路径，例如 t.a.b.c 中的 ["t", "a", "b", "c"]
    pub table_path: Vec<String>,
    /// 是否为方法定义（使用冒号语法）
    pub is_method: bool,
    /// 参数名列表
    pub params: Vec<String>,
    /// 是否接受变长参数
    pub is_vararg: bool,
    /// 函数体
    pub body: Vec<Box<Stmt>>,
    /// 是否为局部函数
    pub is_local: bool,
    /// 结束行号
    pub end_line: i32,
}

/// return 语句
///
/// C++ 对应: `Lua::ReturnStmt`
#[derive(Debug, Clone)]
pub struct ReturnStmt {
    pub location: SourceLocation,
    pub values: Vec<Box<Expr>>,
}

/// break 语句
///
/// C++ 对应: `Lua::BreakStmt`
#[derive(Debug, Clone)]
pub struct BreakStmt {
    pub location: SourceLocation,
}

/// do-end 块
///
/// C++ 对应: `Lua::DoStmt`
#[derive(Debug, Clone)]
pub struct DoStmt {
    pub location: SourceLocation,
    pub body: Vec<Box<Stmt>>,
    /// 结束行号
    pub end_line: i32,
}

// =====================================================================
// 语句枚举（13 种变体，对应 C++ StmtVariant）
// =====================================================================

/// Lua 5.1 语句枚举
///
/// 包含全部 13 种语句节点类型。
/// 使用 `Box<Stmt>` 实现递归所有权（如函数体内的语句）。
///
/// C++ 对应: `Lua::StmtVariant` (std::variant of 13 types)
#[derive(Debug, Clone)]
pub enum Stmt {
    Empty(EmptyStmt),
    Assign(AssignStmt),
    Local(LocalStmt),
    Call(CallStmt),
    If(IfStmt),
    While(WhileStmt),
    Repeat(RepeatStmt),
    ForNum(ForNumStmt),
    ForIn(ForInStmt),
    Function(FunctionStmt),
    Return(ReturnStmt),
    Break(BreakStmt),
    Do(DoStmt),
}

/// 语句节点数量（编译期常量，用于验证）
pub const STMT_NODE_COUNT: usize = 13;

impl Stmt {
    /// 获取语句的行号
    ///
    /// C++ 对应: `Lua::Stmt::getLine()`
    pub fn line(&self) -> i32 {
        match self {
            Stmt::Empty(s) => s.location.line,
            Stmt::Assign(s) => s.location.line,
            Stmt::Local(s) => s.location.line,
            Stmt::Call(s) => s.location.line,
            Stmt::If(s) => s.location.line,
            Stmt::While(s) => s.location.line,
            Stmt::Repeat(s) => s.location.line,
            Stmt::ForNum(s) => s.location.line,
            Stmt::ForIn(s) => s.location.line,
            Stmt::Function(s) => s.location.line,
            Stmt::Return(s) => s.location.line,
            Stmt::Break(s) => s.location.line,
            Stmt::Do(s) => s.location.line,
        }
    }

    /// 获取语句的列号
    ///
    /// C++ 对应: `Lua::Stmt::getColumn()`
    pub fn column(&self) -> i32 {
        match self {
            Stmt::Empty(s) => s.location.column,
            Stmt::Assign(s) => s.location.column,
            Stmt::Local(s) => s.location.column,
            Stmt::Call(s) => s.location.column,
            Stmt::If(s) => s.location.column,
            Stmt::While(s) => s.location.column,
            Stmt::Repeat(s) => s.location.column,
            Stmt::ForNum(s) => s.location.column,
            Stmt::ForIn(s) => s.location.column,
            Stmt::Function(s) => s.location.column,
            Stmt::Return(s) => s.location.column,
            Stmt::Break(s) => s.location.column,
            Stmt::Do(s) => s.location.column,
        }
    }

    /// 获取语句覆盖的结束行号
    ///
    /// 对于 if/while/for/function/do 等块语句，返回 end 关键字的行号；
    /// 对于单行语句，返回其自身的行号。
    ///
    /// C++ 对应: `Lua::Stmt::getEndLine()`
    pub fn end_line(&self) -> i32 {
        match self {
            Stmt::Empty(s) => s.location.line,
            Stmt::Assign(s) => s.location.line,
            Stmt::Local(s) => s.location.line,
            Stmt::Call(s) => s.location.line,
            Stmt::If(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::While(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::Repeat(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::ForNum(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::ForIn(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::Function(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
            Stmt::Return(s) => s.location.line,
            Stmt::Break(s) => s.location.line,
            Stmt::Do(s) => {
                if s.end_line > 0 {
                    s.end_line
                } else {
                    s.location.line
                }
            }
        }
    }
}

// =====================================================================
// 程序块（Chunk）
// =====================================================================

/// 顶层程序块
///
/// 表示一个完整的 Lua 源文件或函数体。
///
/// C++ 对应: `Lua::Chunk`
#[derive(Debug, Clone)]
pub struct Chunk {
    pub statements: Vec<Box<Stmt>>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            statements: Vec::new(),
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
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
    fn test_stmt_node_count() {
        assert_eq!(STMT_NODE_COUNT, 13);
    }

    #[test]
    fn test_empty_stmt() {
        let s = Stmt::Empty(EmptyStmt {
            location: loc(1, 1),
        });
        assert_eq!(s.line(), 1);
        assert_eq!(s.column(), 1);
        assert_eq!(s.end_line(), 1);
    }

    #[test]
    fn test_assign_stmt() {
        let target = Box::new(Expr::Name(crate::ast::expr::NameExpr {
            location: loc(1, 1),
            name: "x".to_string(),
        }));
        let value = Box::new(Expr::Number(crate::ast::expr::NumberExpr {
            location: loc(1, 5),
            value: 42.0,
        }));
        let s = Stmt::Assign(AssignStmt {
            location: loc(1, 1),
            targets: vec![target],
            values: vec![value],
        });
        assert_eq!(s.line(), 1);
        assert!(matches!(s, Stmt::Assign(ref a) if a.targets.len() == 1));
    }

    #[test]
    fn test_local_stmt() {
        let s = Stmt::Local(LocalStmt {
            location: loc(1, 1),
            names: vec!["x".to_string(), "y".to_string()],
            values: vec![],
        });
        assert!(matches!(s, Stmt::Local(ref l) if l.names.len() == 2));
    }

    #[test]
    fn test_call_stmt() {
        let call = Box::new(Expr::Call(crate::ast::expr::CallExpr {
            location: loc(1, 1),
            func: Box::new(Expr::Name(crate::ast::expr::NameExpr {
                location: loc(1, 1),
                name: "print".to_string(),
            })),
            args: vec![],
            is_method_call: false,
        }));
        let s = Stmt::Call(CallStmt {
            location: loc(1, 1),
            call,
        });
        assert_eq!(s.line(), 1);
    }

    #[test]
    fn test_if_stmt() {
        let cond = Box::new(Expr::Boolean(crate::ast::expr::BoolExpr {
            location: loc(1, 4),
            value: true,
        }));
        let s = Stmt::If(IfStmt {
            location: loc(1, 1),
            branches: vec![IfBranch {
                condition: cond,
                body: vec![],
            }],
            else_branch: vec![],
            end_line: 3,
        });
        assert_eq!(s.line(), 1);
        assert_eq!(s.end_line(), 3);
    }

    #[test]
    fn test_while_stmt() {
        let cond = Box::new(Expr::Boolean(crate::ast::expr::BoolExpr {
            location: loc(1, 7),
            value: true,
        }));
        let s = Stmt::While(WhileStmt {
            location: loc(1, 1),
            condition: cond,
            body: vec![],
            end_line: 2,
        });
        assert_eq!(s.end_line(), 2);
    }

    #[test]
    fn test_repeat_stmt() {
        let cond = Box::new(Expr::Boolean(crate::ast::expr::BoolExpr {
            location: loc(3, 7),
            value: false,
        }));
        let s = Stmt::Repeat(RepeatStmt {
            location: loc(1, 1),
            body: vec![],
            condition: cond,
            end_line: 3,
        });
        assert_eq!(s.end_line(), 3);
    }

    #[test]
    fn test_for_num_stmt() {
        let init = Box::new(Expr::Number(crate::ast::expr::NumberExpr {
            location: loc(1, 10),
            value: 1.0,
        }));
        let limit = Box::new(Expr::Number(crate::ast::expr::NumberExpr {
            location: loc(1, 13),
            value: 10.0,
        }));
        let s = Stmt::ForNum(ForNumStmt {
            location: loc(1, 1),
            var: "i".to_string(),
            init,
            limit,
            step: None,
            body: vec![],
            end_line: 2,
        });
        assert_eq!(s.end_line(), 2);
    }

    #[test]
    fn test_for_in_stmt() {
        let iter = Box::new(Expr::Name(crate::ast::expr::NameExpr {
            location: loc(1, 14),
            name: "pairs".to_string(),
        }));
        let s = Stmt::ForIn(ForInStmt {
            location: loc(1, 1),
            vars: vec!["k".to_string(), "v".to_string()],
            iterators: vec![iter],
            body: vec![],
            end_line: 2,
        });
        assert!(matches!(s, Stmt::ForIn(ref f) if f.vars.len() == 2));
    }

    #[test]
    fn test_function_stmt() {
        let s = Stmt::Function(FunctionStmt {
            location: loc(1, 1),
            name: "foo".to_string(),
            table_path: vec![],
            is_method: false,
            params: vec!["a".to_string(), "b".to_string()],
            is_vararg: false,
            body: vec![],
            is_local: false,
            end_line: 3,
        });
        assert_eq!(s.end_line(), 3);
    }

    #[test]
    fn test_function_stmt_method() {
        let s = Stmt::Function(FunctionStmt {
            location: loc(1, 1),
            name: "method".to_string(),
            table_path: vec!["t".to_string()],
            is_method: true,
            params: vec!["x".to_string()],
            is_vararg: false,
            body: vec![],
            is_local: false,
            end_line: 3,
        });
        assert!(matches!(s, Stmt::Function(ref f) if f.is_method));
        assert!(matches!(s, Stmt::Function(ref f) if f.table_path == vec!["t"]));
    }

    #[test]
    fn test_return_stmt() {
        let value = Box::new(Expr::Number(crate::ast::expr::NumberExpr {
            location: loc(1, 8),
            value: 42.0,
        }));
        let s = Stmt::Return(ReturnStmt {
            location: loc(1, 1),
            values: vec![value],
        });
        assert_eq!(s.line(), 1);
    }

    #[test]
    fn test_break_stmt() {
        let s = Stmt::Break(BreakStmt {
            location: loc(1, 1),
        });
        assert_eq!(s.line(), 1);
    }

    #[test]
    fn test_do_stmt() {
        let s = Stmt::Do(DoStmt {
            location: loc(1, 1),
            body: vec![],
            end_line: 2,
        });
        assert_eq!(s.end_line(), 2);
    }

    #[test]
    fn test_chunk() {
        let mut chunk = Chunk::new();
        assert!(chunk.statements.is_empty());

        chunk.statements.push(Box::new(Stmt::Empty(EmptyStmt {
            location: loc(1, 1),
        })));
        assert_eq!(chunk.statements.len(), 1);
    }

    #[test]
    fn test_all_stmt_node_types_coverage() {
        // 确保所有 13 种变体都可以构造
        let _ = Stmt::Empty(EmptyStmt {
            location: loc(0, 0),
        });
        let _ = Stmt::Assign(AssignStmt {
            location: loc(0, 0),
            targets: vec![],
            values: vec![],
        });
        let _ = Stmt::Local(LocalStmt {
            location: loc(0, 0),
            names: vec![],
            values: vec![],
        });
        let _ = Stmt::Call(CallStmt {
            location: loc(0, 0),
            call: Box::new(Expr::Nil(crate::ast::expr::NilExpr {
                location: loc(0, 0),
            })),
        });
        let _ = Stmt::If(IfStmt {
            location: loc(0, 0),
            branches: vec![],
            else_branch: vec![],
            end_line: 0,
        });
        let _ = Stmt::While(WhileStmt {
            location: loc(0, 0),
            condition: Box::new(Expr::Nil(crate::ast::expr::NilExpr {
                location: loc(0, 0),
            })),
            body: vec![],
            end_line: 0,
        });
        let _ = Stmt::Repeat(RepeatStmt {
            location: loc(0, 0),
            body: vec![],
            condition: Box::new(Expr::Nil(crate::ast::expr::NilExpr {
                location: loc(0, 0),
            })),
            end_line: 0,
        });
        let _ = Stmt::ForNum(ForNumStmt {
            location: loc(0, 0),
            var: String::new(),
            init: Box::new(Expr::Nil(crate::ast::expr::NilExpr {
                location: loc(0, 0),
            })),
            limit: Box::new(Expr::Nil(crate::ast::expr::NilExpr {
                location: loc(0, 0),
            })),
            step: None,
            body: vec![],
            end_line: 0,
        });
        let _ = Stmt::ForIn(ForInStmt {
            location: loc(0, 0),
            vars: vec![],
            iterators: vec![],
            body: vec![],
            end_line: 0,
        });
        let _ = Stmt::Function(FunctionStmt {
            location: loc(0, 0),
            name: String::new(),
            table_path: vec![],
            is_method: false,
            params: vec![],
            is_vararg: false,
            body: vec![],
            is_local: false,
            end_line: 0,
        });
        let _ = Stmt::Return(ReturnStmt {
            location: loc(0, 0),
            values: vec![],
        });
        let _ = Stmt::Break(BreakStmt {
            location: loc(0, 0),
        });
        let _ = Stmt::Do(DoStmt {
            location: loc(0, 0),
            body: vec![],
            end_line: 0,
        });
    }
}
