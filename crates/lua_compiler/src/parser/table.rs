//! 表构造器解析
//!
//! 解析 Lua 表字面量 `{field1, field2, ...}`。
//! 支持：
//! - 数组元素 `{value1, value2}`
//! - 键值对 `{[expr] = value}`
//! - 命名键 `{name = value}`
//! - 尾随分隔符 `{a, b,}`
//!

use crate::ast::SourceLocation;
use crate::ast::expr::{Expr, StringExpr, TableExpr, TableField};
use crate::parser::ParseError;
use crate::parser::Parser;
use crate::token::TokenType;

impl<'source> Parser<'source> {
    /// 解析表构造器 `{ [key =] value [, ...] }`
    ///
    pub fn parse_table_constructor(&mut self) -> Result<Box<Expr>, ParseError> {
        let _guard = self.recursion_guard(Self::MAX_RECURSION_DEPTH)?;

        let line = self.current().line;
        let column = self.current().column;

        self.expect(TokenType::LBrace, "Expected '{'")?;

        let mut fields = Vec::new();

        while !self.check(TokenType::RBrace) {
            let field = self.parse_table_field()?;
            fields.push(field);

            // 分隔符：逗号或分号
            if !self.match_token(TokenType::Comma) {
                self.match_token(TokenType::Semicolon);
            }

            // 允许尾随分隔符
            if self.check(TokenType::RBrace) {
                break;
            }
        }

        self.expect(TokenType::RBrace, "Expected '}' to close table constructor")?;

        Ok(Box::new(Expr::Table(TableExpr {
            location: SourceLocation::new(line, column),
            fields,
        })))
    }

    /// 解析单个表字段
    fn parse_table_field(&mut self) -> Result<TableField, ParseError> {
        // `[key] = value`
        if self.match_token(TokenType::LBracket) {
            let key = self.parse_expression()?;
            self.expect(TokenType::RBracket, "Expected ']' after table key")?;
            self.expect(TokenType::Assign, "Expected '=' after table key")?;
            let value = self.parse_expression()?;

            return Ok(TableField {
                key: Some(key),
                value,
            });
        }

        // `Name = value` 或 `value`（数组元素）
        if crate::parser::is_name(self.current()) {
            let next = self.peek();

            if next.token_type == TokenType::Assign {
                // 命名键：`name = value`
                let name = Self::token_string(self.current()).to_string();
                let name_line = self.current().line;
                let name_col = self.current().column;
                self.advance(); // consume name
                self.advance(); // consume '='

                let key = Box::new(Expr::String(StringExpr {
                    location: SourceLocation::new(name_line, name_col),
                    value: name,
                }));
                let value = self.parse_expression()?;

                return Ok(TableField {
                    key: Some(key),
                    value,
                });
            }
        }

        // 数组元素（无 key）
        let value = self.parse_expression()?;
        Ok(TableField { key: None, value })
    }
}
