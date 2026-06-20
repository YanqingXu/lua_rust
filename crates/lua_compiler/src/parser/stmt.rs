//! 语句与语句块解析
//!
//! 实现语句块、if/while/repeat/for/do 控制流、
//! 局部声明、return、break、赋值和函数调用语句。
//!
//! C++ 参考: `lua_cpp/src/compiler/parser/parser_stmt.cpp`

use crate::ast::SourceLocation;
use crate::ast::expr::{Expr, NumberExpr};
use crate::ast::stmt::{
    AssignStmt, BreakStmt, CallStmt, DoStmt, ForInStmt, ForNumStmt, IfBranch, IfStmt, LocalStmt,
    RepeatStmt, ReturnStmt, Stmt, WhileStmt,
};
use crate::parser::ParseError;
use crate::parser::Parser;
use crate::token::TokenType;

impl<'source> Parser<'source> {
    // ── 语句块 ─────────────────────────────────────────────────────

    /// 解析语句块（直到 end/eof/else/elseif/until）
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseBlock()`
    pub fn parse_block(&mut self) -> Result<Vec<Box<Stmt>>, ParseError> {
        let _guard = self.recursion_guard(Self::MAX_BLOCK_RECURSION_DEPTH)?;

        let mut statements = Vec::new();
        let mut may_consume_separator = false;

        while !self.check(TokenType::Eos)
            && !self.check(TokenType::End)
            && !self.check(TokenType::Else)
            && !self.check(TokenType::Elseif)
            && !self.check(TokenType::Until)
        {
            // Statement-boundary error recovery
            if self.check(TokenType::Semicolon) {
                if !may_consume_separator {
                    return Err(self.make_error("unexpected symbol"));
                }
                self.advance();
                may_consume_separator = false;
                continue;
            }

            if self.check(TokenType::Return) {
                let stmt = self.parse_return_stmt()?;
                statements.push(stmt);
                break;
            }

            let stmt = self.parse_statement()?;
            statements.push(stmt);
            may_consume_separator = true;
        }

        Ok(statements)
    }

    // ── 语句分发 ───────────────────────────────────────────────────

    /// 语句分发器
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseStatement()`
    fn parse_statement(&mut self) -> Result<Box<Stmt>, ParseError> {
        match self.current().token_type {
            TokenType::If => self.parse_if_stmt(),
            TokenType::While => self.parse_while_stmt(),
            TokenType::Do => self.parse_do_stmt(),
            TokenType::For => self.parse_for_stmt(),
            TokenType::Repeat => self.parse_repeat_stmt(),
            TokenType::Function => self.parse_function_stmt(),
            TokenType::Local => self.parse_local_stmt(),
            TokenType::Break => self.parse_break_stmt(),
            _ => self.parse_expr_stmt(),
        }
    }

    // ── if 语句 ────────────────────────────────────────────────────

    /// if then (elseif then)* (else)? end
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseIfStmt()`
    fn parse_if_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::If, "Expected 'if'")?;

        let mut branches = Vec::new();
        let mut else_branch = Vec::new();

        // if condition then body
        let condition = self.parse_expression()?;
        self.expect(TokenType::Then, "Expected 'then' after if condition")?;
        let body = self.parse_block()?;
        branches.push(IfBranch { condition, body });

        // elseif condition then body
        while self.match_token(TokenType::Elseif) {
            let condition = self.parse_expression()?;
            self.expect(TokenType::Then, "Expected 'then' after elseif condition")?;
            let body = self.parse_block()?;
            branches.push(IfBranch { condition, body });
        }

        // else body
        if self.match_token(TokenType::Else) {
            else_branch = self.parse_block()?;
        }

        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close if statement")?;

        Ok(Box::new(Stmt::If(IfStmt {
            location: SourceLocation::new(line, column),
            branches,
            else_branch,
            end_line,
        })))
    }

    // ── while 循环 ─────────────────────────────────────────────────

    fn parse_while_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::While, "Expected 'while'")?;

        let condition = self.parse_expression()?;
        self.expect(TokenType::Do, "Expected 'do' after while condition")?;
        let body = self.parse_block()?;
        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close while loop")?;

        Ok(Box::new(Stmt::While(WhileStmt {
            location: SourceLocation::new(line, column),
            condition,
            body,
            end_line,
        })))
    }

    // ── do 块 ──────────────────────────────────────────────────────

    fn parse_do_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Do, "Expected 'do'")?;

        let body = self.parse_block()?;
        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close do block")?;

        Ok(Box::new(Stmt::Do(DoStmt {
            location: SourceLocation::new(line, column),
            body,
            end_line,
        })))
    }

    // ── repeat 循环 ────────────────────────────────────────────────

    fn parse_repeat_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Repeat, "Expected 'repeat'")?;

        let body = self.parse_block()?;
        let end_line = self.current().line;
        self.expect(TokenType::Until, "Expected 'until' to close repeat loop")?;
        let condition = self.parse_expression()?;

        Ok(Box::new(Stmt::Repeat(RepeatStmt {
            location: SourceLocation::new(line, column),
            body,
            condition,
            end_line,
        })))
    }

    // ── for 循环 ───────────────────────────────────────────────────

    /// 解析 for 语句（数值 for 或 泛型 for）
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseForStmt()`
    fn parse_for_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::For, "Expected 'for'")?;

        if !crate::parser::is_name(self.current()) {
            return Err(self.make_error("Expected variable name after 'for'"));
        }
        let var_name = Self::token_string(self.current()).to_string();
        self.advance();

        // 数值 for: for name = init, limit[, step] do body end
        if self.match_token(TokenType::Assign) {
            let init = self.parse_expression()?;
            self.expect(TokenType::Comma, "Expected ',' after for init value")?;
            let limit = self.parse_expression()?;

            let step = if self.match_token(TokenType::Comma) {
                Some(self.parse_expression()?)
            } else {
                Some(Box::new(Expr::Number(NumberExpr {
                    location: SourceLocation::new(self.current().line, self.current().column),
                    value: 1.0,
                })))
            };

            self.expect(TokenType::Do, "Expected 'do' after for header")?;
            let body = self.parse_block()?;
            let end_line = self.current().line;
            self.expect(TokenType::End, "Expected 'end' to close for loop")?;

            return Ok(Box::new(Stmt::ForNum(ForNumStmt {
                location: SourceLocation::new(line, column),
                var: var_name,
                init,
                limit,
                step,
                body,
                end_line,
            })));
        }

        // 泛型 for: for name[, name...] in iterators do body end
        // 或: for name in iterators do body end
        let mut vars = vec![var_name];

        // 消费 name-list 中的逗号分隔变量
        if self.match_token(TokenType::Comma) {
            loop {
                if !crate::parser::is_name(self.current()) {
                    return Err(self.make_error("Expected variable name in for-in loop"));
                }
                vars.push(Self::token_string(self.current()).to_string());
                self.advance();
                if !self.match_token(TokenType::Comma) {
                    break;
                }
            }
        }

        self.expect(TokenType::In, "Expected 'in' in for-in loop")?;
        let iterators = self.parse_expr_list()?;
        self.expect(TokenType::Do, "Expected 'do' after for-in header")?;
        let body = self.parse_block()?;
        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close for-in loop")?;

        Ok(Box::new(Stmt::ForIn(ForInStmt {
            location: SourceLocation::new(line, column),
            vars,
            iterators,
            body,
            end_line,
        })))
    }

    // ── local 语句 ─────────────────────────────────────────────────

    fn parse_local_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Local, "Expected 'local'")?;

        // local function name(...) ... end
        if self.check(TokenType::Function) {
            self.advance();
            return self.parse_local_function_stmt(line, column);
        }

        // local name[, name...] [= expr[, expr...]]
        let mut names = Vec::new();

        loop {
            if !crate::parser::is_name(self.current()) {
                return Err(self.make_error("Expected variable name in local statement"));
            }
            let name = Self::token_string(self.current()).to_string();
            let tok = self.current().clone();
            self.declare_local_name(&name, &tok)?;
            names.push(name);
            self.advance();

            if !self.match_token(TokenType::Comma) {
                break;
            }
        }

        let values = if self.match_token(TokenType::Assign) {
            self.parse_expr_list()?
        } else {
            Vec::new()
        };

        Ok(Box::new(Stmt::Local(LocalStmt {
            location: SourceLocation::new(line, column),
            names,
            values,
        })))
    }

    /// 解析 local function 声明
    fn parse_local_function_stmt(
        &mut self,
        line: i32,
        column: i32,
    ) -> Result<Box<Stmt>, ParseError> {
        if !crate::parser::is_name(self.current()) {
            return Err(self.make_error("Expected function name after 'local function'"));
        }
        let name = Self::token_string(self.current()).to_string();
        let name_tok = self.current().clone();
        self.declare_local_name(&name, &name_tok)?;
        self.advance();

        self.expect(TokenType::LParen, "Expected '(' after function name")?;
        let mut params = self.parse_param_list()?;
        self.expect(TokenType::RParen, "Expected ')' after parameters")?;

        let is_vararg = !params.is_empty() && params.last().map(|s| s.as_str()) == Some("...");
        if is_vararg {
            params.pop();
        }

        let scope_line = line;
        self.enter_function_syntax_scope(scope_line, &params);
        let body = self.parse_block()?;
        self.leave_function_syntax_scope();
        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close function")?;

        Ok(Box::new(Stmt::Function(crate::ast::stmt::FunctionStmt {
            location: SourceLocation::new(line, column),
            name,
            table_path: Vec::new(),
            is_method: false,
            params,
            is_vararg,
            body,
            is_local: true,
            end_line,
        })))
    }

    // ── return 语句 ────────────────────────────────────────────────

    fn parse_return_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Return, "Expected 'return'")?;

        let values = if !self.check(TokenType::End)
            && !self.check(TokenType::Eos)
            && !self.check(TokenType::Else)
            && !self.check(TokenType::Elseif)
            && !self.check(TokenType::Until)
            && !self.check(TokenType::Semicolon)
        {
            self.parse_expr_list()?
        } else {
            Vec::new()
        };

        let _ = self.match_token(TokenType::Semicolon);

        Ok(Box::new(Stmt::Return(ReturnStmt {
            location: SourceLocation::new(line, column),
            values,
        })))
    }

    // ── break 语句 ─────────────────────────────────────────────────

    fn parse_break_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Break, "Expected 'break'")?;

        Ok(Box::new(Stmt::Break(BreakStmt {
            location: SourceLocation::new(line, column),
        })))
    }

    // ── 表达式语句（赋值 / 函数调用）───────────────────────────────

    /// 表达式语句：赋值语句或独立函数调用
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseExprStmt()`
    fn parse_expr_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let first_token = self.current().clone();

        let expr = self.parse_expression()?;

        // 多变量赋值: targets, targets... = values...
        if self.check(TokenType::Comma) {
            let mut targets = vec![expr];

            while self.match_token(TokenType::Comma) {
                targets.push(self.parse_expression()?);
            }

            self.expect(TokenType::Assign, "Expected '=' in assignment")?;
            let values = self.parse_expr_list()?;

            let loc = SourceLocation::new(targets[0].line(), targets[0].column());
            return Ok(Box::new(Stmt::Assign(AssignStmt {
                location: loc,
                targets,
                values,
            })));
        }

        // 单变量赋值: target = values...
        if self.match_token(TokenType::Assign) {
            let targets = vec![expr];
            let values = self.parse_expr_list()?;

            let loc = SourceLocation::new(targets[0].line(), targets[0].column());
            return Ok(Box::new(Stmt::Assign(AssignStmt {
                location: loc,
                targets,
                values,
            })));
        }

        // 独立函数调用
        #[allow(irrefutable_let_patterns)]
        if let Expr::Call(_) = *expr {
            let loc = SourceLocation::new(expr.line(), expr.column());
            Ok(Box::new(Stmt::Call(CallStmt {
                location: loc,
                call: expr,
            })))
        } else {
            let error_token = if self.check(TokenType::Eos) {
                &first_token
            } else {
                self.current()
            };
            Err(self.make_error_at(error_token, "unexpected symbol"))
        }
    }
}
