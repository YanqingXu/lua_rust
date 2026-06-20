//! 函数声明与函数表达式解析
//!
//! 实现全局/局部函数声明、函数表达式和参数列表解析。
//!
//! C++ 参考: `lua_cpp/src/compiler/parser/parser_func.cpp`

use crate::ast::SourceLocation;
use crate::ast::expr::{Expr, FunctionExpr};
use crate::ast::stmt::{FunctionStmt, Stmt};
use crate::parser::ParseError;
use crate::parser::Parser;
use crate::token::TokenType;

impl<'source> Parser<'source> {
    // ── 函数声明语句 ───────────────────────────────────────────────

    /// 解析 function 语句（全局/局部函数声明入口由 stmt.rs 调用）
    ///
    /// 支持形式：
    /// - `function foo() end` — 简单函数
    /// - `function t.a.b.c.foo() end` — 表成员函数
    /// - `function t:method() end` — 方法定义
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseFunctionStmt()`
    pub fn parse_function_stmt(&mut self) -> Result<Box<Stmt>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Function, "Expected 'function'")?;

        if !crate::parser::is_name(self.current()) {
            return Err(self.make_error("Expected function name"));
        }

        let mut name = Self::token_string(self.current()).to_string();
        self.advance();

        let mut table_path = Vec::new();
        let mut is_method = false;

        // 解析表路径: t.a.b:method 或 t.a.b.func
        while self.check(TokenType::Dot) || self.check(TokenType::Colon) {
            if self.match_token(TokenType::Dot) {
                table_path.push(std::mem::take(&mut name));

                if !crate::parser::is_name(self.current()) {
                    return Err(self.make_error("Expected field name after '.'"));
                }
                name = Self::token_string(self.current()).to_string();
                self.advance();
            } else if self.match_token(TokenType::Colon) {
                table_path.push(std::mem::take(&mut name));
                is_method = true;

                if !crate::parser::is_name(self.current()) {
                    return Err(self.make_error("Expected method name after ':'"));
                }
                name = Self::token_string(self.current()).to_string();
                self.advance();
                break;
            }
        }

        self.expect(TokenType::LParen, "Expected '(' after function name")?;
        let mut params = self.parse_param_list()?;
        self.expect(TokenType::RParen, "Expected ')' after parameters")?;

        let is_vararg = !params.is_empty() && params.last().map(|s| s.as_str()) == Some("...");
        if is_vararg {
            params.pop();
        }

        // 方法定义自动添加 self 参数
        if is_method {
            params.insert(0, "self".to_string());
        }

        let scope_line = line;
        self.enter_function_syntax_scope(scope_line, &params);
        let body = self.parse_block()?;
        self.leave_function_syntax_scope();
        let end_line = self.current().line;
        self.expect(TokenType::End, "Expected 'end' to close function")?;

        Ok(Box::new(Stmt::Function(FunctionStmt {
            location: SourceLocation::new(line, column),
            name,
            table_path,
            is_method,
            params,
            is_vararg,
            body,
            is_local: false,
            end_line,
        })))
    }

    // ── 函数表达式 ─────────────────────────────────────────────────

    /// 解析 function 表达式 `function(...) body end`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseFunctionExpr()`
    pub fn parse_function_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let _guard = self.recursion_guard(Self::MAX_RECURSION_DEPTH)?;

        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::Function, "Expected 'function'")?;

        self.expect(TokenType::LParen, "Expected '(' after 'function'")?;
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

        Ok(Box::new(Expr::Function(FunctionExpr {
            location: SourceLocation::new(line, column),
            params,
            is_vararg,
            body,
            end_line,
        })))
    }

    // ── 参数列表 ───────────────────────────────────────────────────

    /// 解析函数参数列表 `(name, ...)`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseParamList()`
    pub fn parse_param_list(&mut self) -> Result<Vec<String>, ParseError> {
        let mut params = Vec::new();

        // 空参数列表
        if self.check(TokenType::RParen) {
            return Ok(params);
        }

        // 仅有 ... 参数
        if self.match_token(TokenType::Dots) {
            params.push("...".to_string());
            return Ok(params);
        }

        loop {
            if crate::parser::is_name(self.current()) {
                params.push(Self::token_string(self.current()).to_string());
                self.advance();
            } else if self.match_token(TokenType::Dots) {
                params.push("...".to_string());
                break;
            } else {
                return Err(self.make_error("Expected parameter name"));
            }

            if !self.match_token(TokenType::Comma) {
                break;
            }
        }

        Ok(params)
    }
}
