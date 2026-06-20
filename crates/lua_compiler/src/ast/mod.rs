//! Lua 抽象语法树（AST）模块
//!
//! 定义 Lua 5.1 的抽象语法树节点类型，用于表示解析后的程序结构。
//!
//! ## 模块结构
//! - `expr` — 表达式节点（14 种变体）
//! - `stmt` — 语句节点（13 种变体）
//! - `visitor` — Visitor 模式（`ExprVisitor`、`StmtVisitor`、`AstVisitor` trait）
//!
//! ## 设计原则
//! - 使用 Rust `enum` 实现类型安全的多态（对标 C++ `std::variant`）
//! - 使用 `Box<Expr>` / `Box<Stmt>` 管理递归节点生命周期
//! - 清晰的节点层次结构
//! - 完整的位置信息（行号、列号）
//!
//! C++ 参考: `lua_cpp/src/compiler/ast.hpp`

pub mod expr;
pub mod stmt;
pub mod visitor;

// 常用类型的便捷重导出
pub use expr::{
    BinaryOp, BoolExpr, CallExpr, EXPR_NODE_COUNT, Expr, FunctionExpr, IndexExpr, MemberExpr,
    NameExpr, NilExpr, NumberExpr, ParenExpr, SourceLocation, StringExpr, TableExpr, TableField,
    UnaryExpr, UnaryOp, VarargExpr,
};
pub use stmt::{
    AssignStmt, BreakStmt, CallStmt, Chunk, DoStmt, EmptyStmt, ForInStmt, ForNumStmt, FunctionStmt,
    IfBranch, IfStmt, LocalStmt, RepeatStmt, ReturnStmt, STMT_NODE_COUNT, Stmt, WhileStmt,
};
pub use visitor::{AstVisitor, ExprVisitor, StmtVisitor};
