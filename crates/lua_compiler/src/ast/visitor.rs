//! Lua AST Visitor 模式
//!
//! 为 AST 的表达式和语句提供 visitor trait，支持遍历全部节点类型。
//! Rust 编译器通过穷尽检查确保每个 visitor 覆盖所有节点变体。
//!
//! C++ 参考: `lua_cpp/src/compiler/ast_visitor.hpp`

use crate::ast::expr::{
    BinaryExpr, BoolExpr, CallExpr, Expr, FunctionExpr, IndexExpr, MemberExpr, NameExpr, NilExpr,
    NumberExpr, ParenExpr, StringExpr, TableExpr, UnaryExpr, VarargExpr,
};
use crate::ast::stmt::{
    AssignStmt, BreakStmt, CallStmt, DoStmt, EmptyStmt, ForInStmt, ForNumStmt, FunctionStmt,
    IfStmt, LocalStmt, RepeatStmt, ReturnStmt, Stmt, WhileStmt,
};

// =====================================================================
// ExprVisitor trait
// =====================================================================

/// 表达式访问器 trait
///
/// 实现者必须为全部 14 种表达式节点提供 `visit_*` 方法。
/// Rust 编译器会检查 match 穷尽性，确保所有变体都被覆盖。
///
/// C++ 对应: `Lua::ExprVisitor<Derived, R>`
pub trait ExprVisitor<R = ()> {
    /// 访问 nil 字面量
    fn visit_nil(&mut self, expr: &NilExpr) -> R;
    /// 访问布尔字面量
    fn visit_bool(&mut self, expr: &BoolExpr) -> R;
    /// 访问数字字面量
    fn visit_number(&mut self, expr: &NumberExpr) -> R;
    /// 访问字符串字面量
    fn visit_string(&mut self, expr: &StringExpr) -> R;
    /// 访问变长参数
    fn visit_vararg(&mut self, expr: &VarargExpr) -> R;
    /// 访问标识符
    fn visit_name(&mut self, expr: &NameExpr) -> R;
    /// 访问二元运算表达式
    fn visit_binary(&mut self, expr: &BinaryExpr) -> R;
    /// 访问一元运算表达式
    fn visit_unary(&mut self, expr: &UnaryExpr) -> R;
    /// 访问表构造器
    fn visit_table(&mut self, expr: &TableExpr) -> R;
    /// 访问函数调用
    fn visit_call(&mut self, expr: &CallExpr) -> R;
    /// 访问表索引
    fn visit_index(&mut self, expr: &IndexExpr) -> R;
    /// 访问成员访问
    fn visit_member(&mut self, expr: &MemberExpr) -> R;
    /// 访问函数定义表达式
    fn visit_function(&mut self, expr: &FunctionExpr) -> R;
    /// 访问括号表达式
    fn visit_paren(&mut self, expr: &ParenExpr) -> R;

    /// 分发表达式到对应的 visit 方法
    ///
    /// C++ 对应: `Lua::ExprVisitor::visit(const Expr&)`
    fn visit_expr(&mut self, expr: &Expr) -> R {
        match expr {
            Expr::Nil(e) => self.visit_nil(e),
            Expr::Boolean(e) => self.visit_bool(e),
            Expr::Number(e) => self.visit_number(e),
            Expr::String(e) => self.visit_string(e),
            Expr::Vararg(e) => self.visit_vararg(e),
            Expr::Name(e) => self.visit_name(e),
            Expr::Binary(e) => self.visit_binary(e),
            Expr::Unary(e) => self.visit_unary(e),
            Expr::Table(e) => self.visit_table(e),
            Expr::Call(e) => self.visit_call(e),
            Expr::Index(e) => self.visit_index(e),
            Expr::Member(e) => self.visit_member(e),
            Expr::Function(e) => self.visit_function(e),
            Expr::Paren(e) => self.visit_paren(e),
        }
    }
}

// =====================================================================
// StmtVisitor trait
// =====================================================================

/// 语句访问器 trait
///
/// 实现者必须为全部 13 种语句节点提供 `visit_*` 方法。
/// Rust 编译器会检查 match 穷尽性，确保所有变体都被覆盖。
///
/// C++ 对应: `Lua::StmtVisitor<Derived, R>`
pub trait StmtVisitor<R = ()> {
    /// 访问空语句
    fn visit_empty(&mut self, stmt: &EmptyStmt) -> R;
    /// 访问赋值语句
    fn visit_assign(&mut self, stmt: &AssignStmt) -> R;
    /// 访问局部变量声明
    fn visit_local(&mut self, stmt: &LocalStmt) -> R;
    /// 访问函数调用语句
    fn visit_call(&mut self, stmt: &CallStmt) -> R;
    /// 访问 if 语句
    fn visit_if(&mut self, stmt: &IfStmt) -> R;
    /// 访问 while 循环
    fn visit_while(&mut self, stmt: &WhileStmt) -> R;
    /// 访问 repeat-until 循环
    fn visit_repeat(&mut self, stmt: &RepeatStmt) -> R;
    /// 访问数值 for 循环
    fn visit_for_num(&mut self, stmt: &ForNumStmt) -> R;
    /// 访问泛型 for 循环
    fn visit_for_in(&mut self, stmt: &ForInStmt) -> R;
    /// 访问函数定义语句
    fn visit_function(&mut self, stmt: &FunctionStmt) -> R;
    /// 访问 return 语句
    fn visit_return(&mut self, stmt: &ReturnStmt) -> R;
    /// 访问 break 语句
    fn visit_break(&mut self, stmt: &BreakStmt) -> R;
    /// 访问 do 块
    fn visit_do(&mut self, stmt: &DoStmt) -> R;

    /// 分发语句到对应的 visit 方法
    ///
    /// C++ 对应: `Lua::StmtVisitor::visit(const Stmt&)`
    fn visit_stmt(&mut self, stmt: &Stmt) -> R {
        match stmt {
            Stmt::Empty(s) => self.visit_empty(s),
            Stmt::Assign(s) => self.visit_assign(s),
            Stmt::Local(s) => self.visit_local(s),
            Stmt::Call(s) => self.visit_call(s),
            Stmt::If(s) => self.visit_if(s),
            Stmt::While(s) => self.visit_while(s),
            Stmt::Repeat(s) => self.visit_repeat(s),
            Stmt::ForNum(s) => self.visit_for_num(s),
            Stmt::ForIn(s) => self.visit_for_in(s),
            Stmt::Function(s) => self.visit_function(s),
            Stmt::Return(s) => self.visit_return(s),
            Stmt::Break(s) => self.visit_break(s),
            Stmt::Do(s) => self.visit_do(s),
        }
    }
}

// =====================================================================
// AstVisitor trait（组合 ExprVisitor + StmtVisitor）
// =====================================================================

/// 完整的 AST 访问器 trait
///
/// 同时提供表达式和语句的访问能力。
/// 实现此 trait 的类型必须同时实现 `ExprVisitor` 和 `StmtVisitor`。
///
/// C++ 对应: `Lua::AstVisitor<Derived, R>`
pub trait AstVisitor<R = ()>: ExprVisitor<R> + StmtVisitor<R> {
    /// 访问程序块
    fn visit_chunk(&mut self, chunk: &crate::ast::stmt::Chunk) -> R {
        // 默认实现：遍历所有语句
        for stmt in &chunk.statements {
            self.visit_stmt(stmt);
        }
        // 返回默认值；实际实现应覆盖此方法
        self.default_result()
    }

    /// 为 R = () 的默认实现提供返回值
    fn default_result(&self) -> R {
        // 对于 R = () 类型，这不会产生实际代码
        // 对于其他类型，实现者应覆盖此方法
        panic!("AstVisitor::default_result() must be overridden for non-unit return types")
    }
}

// 为 () 返回类型提供特殊的默认实现
impl<T: ExprVisitor + StmtVisitor> AstVisitor for T {
    fn default_result(&self) {
        // unit type — nothing to return
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::expr::SourceLocation;

    fn loc(line: i32, col: i32) -> SourceLocation {
        SourceLocation::new(line, col)
    }

    // ── 测试辅助 visitor ──────────────────────────────────────────

    /// 计数 visitor：统计访问的节点数
    struct CountingVisitor {
        expr_count: usize,
        stmt_count: usize,
    }

    impl ExprVisitor for CountingVisitor {
        fn visit_nil(&mut self, _: &NilExpr) {
            self.expr_count += 1;
        }
        fn visit_bool(&mut self, _: &BoolExpr) {
            self.expr_count += 1;
        }
        fn visit_number(&mut self, _: &NumberExpr) {
            self.expr_count += 1;
        }
        fn visit_string(&mut self, _: &StringExpr) {
            self.expr_count += 1;
        }
        fn visit_vararg(&mut self, _: &VarargExpr) {
            self.expr_count += 1;
        }
        fn visit_name(&mut self, _: &NameExpr) {
            self.expr_count += 1;
        }
        fn visit_binary(&mut self, _: &BinaryExpr) {
            self.expr_count += 1;
        }
        fn visit_unary(&mut self, _: &UnaryExpr) {
            self.expr_count += 1;
        }
        fn visit_table(&mut self, _: &TableExpr) {
            self.expr_count += 1;
        }
        fn visit_call(&mut self, _: &CallExpr) {
            self.expr_count += 1;
        }
        fn visit_index(&mut self, _: &IndexExpr) {
            self.expr_count += 1;
        }
        fn visit_member(&mut self, _: &MemberExpr) {
            self.expr_count += 1;
        }
        fn visit_function(&mut self, _: &FunctionExpr) {
            self.expr_count += 1;
        }
        fn visit_paren(&mut self, _: &ParenExpr) {
            self.expr_count += 1;
        }
    }

    impl StmtVisitor for CountingVisitor {
        fn visit_empty(&mut self, _: &EmptyStmt) {
            self.stmt_count += 1;
        }
        fn visit_assign(&mut self, _: &AssignStmt) {
            self.stmt_count += 1;
        }
        fn visit_local(&mut self, _: &LocalStmt) {
            self.stmt_count += 1;
        }
        fn visit_call(&mut self, _: &CallStmt) {
            self.stmt_count += 1;
        }
        fn visit_if(&mut self, _: &IfStmt) {
            self.stmt_count += 1;
        }
        fn visit_while(&mut self, _: &WhileStmt) {
            self.stmt_count += 1;
        }
        fn visit_repeat(&mut self, _: &RepeatStmt) {
            self.stmt_count += 1;
        }
        fn visit_for_num(&mut self, _: &ForNumStmt) {
            self.stmt_count += 1;
        }
        fn visit_for_in(&mut self, _: &ForInStmt) {
            self.stmt_count += 1;
        }
        fn visit_function(&mut self, _: &FunctionStmt) {
            self.stmt_count += 1;
        }
        fn visit_return(&mut self, _: &ReturnStmt) {
            self.stmt_count += 1;
        }
        fn visit_break(&mut self, _: &BreakStmt) {
            self.stmt_count += 1;
        }
        fn visit_do(&mut self, _: &DoStmt) {
            self.stmt_count += 1;
        }
    }

    // ── Visitor 测试 ──────────────────────────────────────────────

    #[test]
    fn test_count_visitor_expr() {
        let mut v = CountingVisitor {
            expr_count: 0,
            stmt_count: 0,
        };
        let e = Expr::Number(NumberExpr {
            location: loc(1, 1),
            value: 42.0,
        });
        v.visit_expr(&e);
        assert_eq!(v.expr_count, 1);
        assert_eq!(v.stmt_count, 0);
    }

    #[test]
    fn test_count_visitor_stmt() {
        let mut v = CountingVisitor {
            expr_count: 0,
            stmt_count: 0,
        };
        let s = Stmt::Return(ReturnStmt {
            location: loc(1, 1),
            values: vec![],
        });
        v.visit_stmt(&s);
        assert_eq!(v.expr_count, 0);
        assert_eq!(v.stmt_count, 1);
    }

    #[test]
    fn test_count_visitor_both() {
        let mut v = CountingVisitor {
            expr_count: 0,
            stmt_count: 0,
        };
        // 访问一个表达式
        let e = Expr::Boolean(BoolExpr {
            location: loc(1, 1),
            value: true,
        });
        v.visit_expr(&e);
        // 访问一个语句
        let s = Stmt::Break(BreakStmt {
            location: loc(2, 1),
        });
        v.visit_stmt(&s);
        assert_eq!(v.expr_count, 1);
        assert_eq!(v.stmt_count, 1);
    }

    #[test]
    fn test_visitor_covers_all_expr_variants() {
        // 编译期保证：如果新增 Expr 变体，match 会编译失败
        let mut v = CountingVisitor {
            expr_count: 0,
            stmt_count: 0,
        };

        // Nil
        v.visit_expr(&Expr::Nil(NilExpr {
            location: loc(0, 0),
        }));
        // Boolean
        v.visit_expr(&Expr::Boolean(BoolExpr {
            location: loc(0, 0),
            value: false,
        }));
        // Number
        v.visit_expr(&Expr::Number(NumberExpr {
            location: loc(0, 0),
            value: 0.0,
        }));
        // String
        v.visit_expr(&Expr::String(StringExpr {
            location: loc(0, 0),
            value: String::new(),
        }));
        // Vararg
        v.visit_expr(&Expr::Vararg(VarargExpr {
            location: loc(0, 0),
        }));
        // Name
        v.visit_expr(&Expr::Name(NameExpr {
            location: loc(0, 0),
            name: String::new(),
        }));
        // Binary
        v.visit_expr(&Expr::Binary(BinaryExpr {
            location: loc(0, 0),
            op: crate::ast::expr::BinaryOp::Add,
            left: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            right: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        }));
        // Unary
        v.visit_expr(&Expr::Unary(UnaryExpr {
            location: loc(0, 0),
            op: crate::ast::expr::UnaryOp::Not,
            operand: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        }));
        // Table
        v.visit_expr(&Expr::Table(TableExpr {
            location: loc(0, 0),
            fields: vec![],
        }));
        // Call
        v.visit_expr(&Expr::Call(CallExpr {
            location: loc(0, 0),
            func: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            args: vec![],
            is_method_call: false,
        }));
        // Index
        v.visit_expr(&Expr::Index(IndexExpr {
            location: loc(0, 0),
            table: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            index: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        }));
        // Member
        v.visit_expr(&Expr::Member(MemberExpr {
            location: loc(0, 0),
            table: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            member: String::new(),
        }));
        // Function
        v.visit_expr(&Expr::Function(FunctionExpr {
            location: loc(0, 0),
            params: vec![],
            is_vararg: false,
            body: vec![],
            end_line: 0,
        }));
        // Paren
        v.visit_expr(&Expr::Paren(ParenExpr {
            location: loc(0, 0),
            expression: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        }));

        assert_eq!(v.expr_count, 14); // 全部 14 种变体
    }

    #[test]
    fn test_visitor_covers_all_stmt_variants() {
        let mut v = CountingVisitor {
            expr_count: 0,
            stmt_count: 0,
        };

        // Empty
        v.visit_stmt(&Stmt::Empty(EmptyStmt {
            location: loc(0, 0),
        }));
        // Assign
        v.visit_stmt(&Stmt::Assign(AssignStmt {
            location: loc(0, 0),
            targets: vec![],
            values: vec![],
        }));
        // Local
        v.visit_stmt(&Stmt::Local(LocalStmt {
            location: loc(0, 0),
            names: vec![],
            values: vec![],
        }));
        // Call
        v.visit_stmt(&Stmt::Call(CallStmt {
            location: loc(0, 0),
            call: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
        }));
        // If
        v.visit_stmt(&Stmt::If(IfStmt {
            location: loc(0, 0),
            branches: vec![],
            else_branch: vec![],
            end_line: 0,
        }));
        // While
        v.visit_stmt(&Stmt::While(WhileStmt {
            location: loc(0, 0),
            condition: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            body: vec![],
            end_line: 0,
        }));
        // Repeat
        v.visit_stmt(&Stmt::Repeat(RepeatStmt {
            location: loc(0, 0),
            body: vec![],
            condition: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            end_line: 0,
        }));
        // ForNum
        v.visit_stmt(&Stmt::ForNum(ForNumStmt {
            location: loc(0, 0),
            var: String::new(),
            init: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            limit: Box::new(Expr::Nil(NilExpr {
                location: loc(0, 0),
            })),
            step: None,
            body: vec![],
            end_line: 0,
        }));
        // ForIn
        v.visit_stmt(&Stmt::ForIn(ForInStmt {
            location: loc(0, 0),
            vars: vec![],
            iterators: vec![],
            body: vec![],
            end_line: 0,
        }));
        // Function
        v.visit_stmt(&Stmt::Function(FunctionStmt {
            location: loc(0, 0),
            name: String::new(),
            table_path: vec![],
            is_method: false,
            params: vec![],
            is_vararg: false,
            body: vec![],
            is_local: false,
            end_line: 0,
        }));
        // Return
        v.visit_stmt(&Stmt::Return(ReturnStmt {
            location: loc(0, 0),
            values: vec![],
        }));
        // Break
        v.visit_stmt(&Stmt::Break(BreakStmt {
            location: loc(0, 0),
        }));
        // Do
        v.visit_stmt(&Stmt::Do(DoStmt {
            location: loc(0, 0),
            body: vec![],
            end_line: 0,
        }));

        assert_eq!(v.stmt_count, 13); // 全部 13 种变体
    }
}
