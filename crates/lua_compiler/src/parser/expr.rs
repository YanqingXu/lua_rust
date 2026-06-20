//! Lua 表达式优先级链解析
//!
//! 12 级表达式优先级（从低到高）：
//! 1. parse_or_expr       — `or`
//! 2. parse_and_expr       — `and`
//! 3. parse_relational_expr — `<` `>` `<=` `>=` `==` `~=`
//! 4. parse_concat_expr    — `..` (right-assoc)
//! 5. parse_additive_expr  — `+` `-`
//! 6. parse_multiplicative — `*` `/` `%`
//! 7. parse_unary_expr     — `not` `-` `#`
//! 8. parse_power_expr     — `^` (right-assoc)
//! 9. parse_primary_expr   — literals, names, `(`, `{`, `function`
//!
//! C++ 参考: `lua_cpp/src/compiler/parser/parser_expr.cpp`

use crate::ast::SourceLocation;
use crate::ast::expr::{BinaryOp, Expr, UnaryOp};
use crate::parser::ParseError;
use crate::parser::Parser;
use crate::token::TokenType;

impl<'source> Parser<'source> {
    // ── 顶级表达式入口 ─────────────────────────────────────────────

    /// parseExpression() — 表达式入口
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseExpression()`
    pub fn parse_expression(&mut self) -> Result<Box<Expr>, ParseError> {
        let _guard = self.recursion_guard(Self::MAX_RECURSION_DEPTH)?;
        self.parse_or_expr()
    }

    // ── 表达式优先级链 ─────────────────────────────────────────────

    /// `or` 表达式 (优先级最低)
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseOrExpr()`
    fn parse_or_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_and_expr()?;

        while self.check(TokenType::Or) {
            let op_token = self.current().clone();
            self.advance();
            let right = self.parse_and_expr()?;
            left = self.make_binary_expr(BinaryOp::Or, &op_token, left, right);
        }

        Ok(left)
    }

    /// `and` 表达式
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseAndExpr()`
    fn parse_and_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_relational_expr()?;

        while self.check(TokenType::And) {
            let op_token = self.current().clone();
            self.advance();
            let right = self.parse_relational_expr()?;
            left = self.make_binary_expr(BinaryOp::And, &op_token, left, right);
        }

        Ok(left)
    }

    /// 关系表达式 `<` `>` `<=` `>=` `==` `~=`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseRelationalExpr()`
    fn parse_relational_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_concat_expr()?;

        loop {
            let op = self.current().token_type;
            let binary_op = match op {
                TokenType::Lt => BinaryOp::Lt,
                TokenType::Gt => BinaryOp::Gt,
                TokenType::Le => BinaryOp::Le,
                TokenType::Ge => BinaryOp::Ge,
                TokenType::Eq => BinaryOp::Eq,
                TokenType::Ne => BinaryOp::Ne,
                _ => break,
            };

            let op_token = self.current().clone();
            self.advance();
            let right = self.parse_concat_expr()?;
            left = self.make_binary_expr(binary_op, &op_token, left, right);
        }

        Ok(left)
    }

    /// 字符串连接 `..` （右结合）
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseConcatExpr()`
    fn parse_concat_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_additive_expr()?;

        if self.check(TokenType::Concat) {
            let op_token = self.current().clone();
            self.advance();

            let _guard = self.recursion_guard(Self::MAX_RIGHT_ASSOC_RECURSION_DEPTH)?;
            let right = self.parse_concat_expr()?;
            left = self.make_binary_expr(BinaryOp::Concat, &op_token, left, right);
        }

        Ok(left)
    }

    /// 加减表达式 `+` `-`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseAdditiveExpr()`
    fn parse_additive_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_multiplicative_expr()?;

        while self.check(TokenType::Plus) || self.check(TokenType::Minus) {
            let op_token = self.current().clone();
            let is_add = self.check(TokenType::Plus);
            self.advance();

            let binary_op = if is_add { BinaryOp::Add } else { BinaryOp::Sub };
            let right = self.parse_multiplicative_expr()?;
            left = self.make_binary_expr(binary_op, &op_token, left, right);
        }

        Ok(left)
    }

    /// 乘除取模表达式 `*` `/` `%`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseMultiplicativeExpr()`
    fn parse_multiplicative_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_unary_expr()?;

        while self.check(TokenType::Star)
            || self.check(TokenType::Slash)
            || self.check(TokenType::Percent)
        {
            let op_token = self.current().clone();
            let op = op_token.token_type;
            self.advance();

            let binary_op = match op {
                TokenType::Star => BinaryOp::Mul,
                TokenType::Slash => BinaryOp::Div,
                _ => BinaryOp::Mod,
            };
            let right = self.parse_unary_expr()?;
            left = self.make_binary_expr(binary_op, &op_token, left, right);
        }

        Ok(left)
    }

    /// 一元表达式 `not` `-` `#`
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseUnaryExpr()`
    fn parse_unary_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        if self.check(TokenType::Not) {
            let op_token = self.current().clone();
            self.advance();
            let operand = self.parse_unary_expr()?;
            return self.make_unary_expr(UnaryOp::Not, &op_token, operand);
        }

        if self.check(TokenType::Minus) {
            let op_token = self.current().clone();
            self.advance();
            let operand = self.parse_unary_expr()?;
            return self.make_unary_expr(UnaryOp::Neg, &op_token, operand);
        }

        if self.check(TokenType::Len) {
            let op_token = self.current().clone();
            self.advance();
            let operand = self.parse_unary_expr()?;
            return self.make_unary_expr(UnaryOp::Len, &op_token, operand);
        }

        self.parse_power_expr()
    }

    /// 幂表达式 `^` （右结合）
    ///
    /// C++ 对应: `Lua::Parser::Impl::parsePowerExpr()`
    fn parse_power_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let mut left = self.parse_primary_expr()?;

        if self.check(TokenType::Caret) {
            let op_token = self.current().clone();
            self.advance();

            let _guard = self.recursion_guard(Self::MAX_RIGHT_ASSOC_RECURSION_DEPTH)?;
            let right = self.parse_unary_expr()?;
            left = self.make_binary_expr(BinaryOp::Pow, &op_token, left, right);
        }

        Ok(left)
    }

    // ── 表达式列表 ─────────────────────────────────────────────────

    /// 解析逗号分隔的表达式列表
    ///
    /// C++ 对应: `Lua::Parser::Impl::parseExprList()`
    pub fn parse_expr_list(&mut self) -> Result<Vec<Box<Expr>>, ParseError> {
        let mut exprs = Vec::new();

        loop {
            exprs.push(self.parse_expression()?);
            if !self.match_token(TokenType::Comma) {
                break;
            }
        }

        Ok(exprs)
    }

    // ── AST 构造辅助 ───────────────────────────────────────────────

    /// 创建二元表达式
    fn make_binary_expr(
        &self,
        op: BinaryOp,
        op_token: &crate::token::Token,
        left: Box<Expr>,
        right: Box<Expr>,
    ) -> Box<Expr> {
        Box::new(Expr::Binary(crate::ast::expr::BinaryExpr {
            location: SourceLocation::new(op_token.line, op_token.column),
            op,
            left,
            right,
        }))
    }

    /// 创建一元表达式
    fn make_unary_expr(
        &self,
        op: UnaryOp,
        op_token: &crate::token::Token,
        operand: Box<Expr>,
    ) -> Result<Box<Expr>, ParseError> {
        Ok(Box::new(Expr::Unary(crate::ast::expr::UnaryExpr {
            location: SourceLocation::new(op_token.line, op_token.column),
            op,
            operand,
        })))
    }
}
