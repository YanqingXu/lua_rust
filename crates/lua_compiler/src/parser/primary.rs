//! 基础表达式（Primary）与后缀表达式（Postfix）解析
//!
//! 实现字面量、标识符、括号、表/函数表达式入口，
//! 以及函数调用 `()`、索引 `[]`、成员访问 `.`、方法调用 `:` 等后缀链。
//!
//! C++ 参考: `lua_cpp/src/compiler/parser/parser_primary.cpp`

use crate::ast::SourceLocation;
use crate::ast::expr::{
    BoolExpr, CallExpr, Expr, IndexExpr, MemberExpr, NameExpr, NilExpr, NumberExpr, ParenExpr,
    StringExpr, VarargExpr,
};
use crate::parser::ParseError;
use crate::parser::Parser;
use crate::token::{TokenType, TokenValue};

impl<'source> Parser<'source> {
    /// 基础表达式解析
    ///
    /// C++ 对应: `Lua::Parser::Impl::parsePrimaryExpr()`
    pub fn parse_primary_expr(&mut self) -> Result<Box<Expr>, ParseError> {
        let line = self.current().line;
        let column = self.current().column;

        // nil
        if self.match_token(TokenType::Nil) {
            return Ok(Box::new(Expr::Nil(NilExpr {
                location: SourceLocation::new(line, column),
            })));
        }

        // true
        if self.match_token(TokenType::True) {
            return Ok(Box::new(Expr::Boolean(BoolExpr {
                location: SourceLocation::new(line, column),
                value: true,
            })));
        }

        // false
        if self.match_token(TokenType::False) {
            return Ok(Box::new(Expr::Boolean(BoolExpr {
                location: SourceLocation::new(line, column),
                value: false,
            })));
        }

        // 数字
        if self.current().is_number() {
            let value = match &self.current().value {
                TokenValue::Number(n) => *n,
                _ => 0.0,
            };
            let num_expr = NumberExpr {
                location: SourceLocation::new(line, column),
                value,
            };
            self.advance();
            return Ok(Box::new(Expr::Number(num_expr)));
        }

        // 字符串
        if self.current().is_string() {
            let value = Self::token_string(self.current()).to_string();
            let str_expr = StringExpr {
                location: SourceLocation::new(line, column),
                value,
            };
            self.advance();
            return Ok(Box::new(Expr::String(str_expr)));
        }

        // 变长参数 ...
        if self.match_token(TokenType::Dots) {
            return Ok(Box::new(Expr::Vararg(VarargExpr {
                location: SourceLocation::new(line, column),
            })));
        }

        // 表构造器 { ... }
        if self.check(TokenType::LBrace) {
            return self.parse_table_constructor();
        }

        // 函数表达式 function (...) ... end
        if self.check(TokenType::Function) {
            return self.parse_function_expr();
        }

        // 括号表达式 ( ... )
        if self.match_token(TokenType::LParen) {
            let expr = self.parse_expression()?;
            self.expect(TokenType::RParen, "Expected ')' after expression")?;

            let paren_expr = ParenExpr {
                location: SourceLocation::new(line, column),
                expression: expr,
            };
            return self.parse_postfix_expr(Box::new(Expr::Paren(paren_expr)));
        }

        // 标识符
        if crate::parser::is_name(self.current()) {
            let name_token = self.current().clone();
            let name = Self::token_string(&name_token).to_string();
            self.note_name_use(&name, &name_token)?;
            self.advance();

            let name_expr = NameExpr {
                location: SourceLocation::new(line, column),
                name,
            };
            return self.parse_postfix_expr(Box::new(Expr::Name(name_expr)));
        }

        // 无法识别的符号
        Err(self.make_error("unexpected symbol"))
    }

    /// 后缀表达式链：处理函数调用、索引、成员访问、方法调用
    ///
    /// C++ 对应: `Lua::Parser::Impl::parsePostfixExpr()`
    fn parse_postfix_expr(&mut self, mut base: Box<Expr>) -> Result<Box<Expr>, ParseError> {
        loop {
            let line = self.current().line;
            let column = self.current().column;

            // 拒绝歧义：函数调用前出现换行
            let reject_ambiguous_newline = |parser: &Parser, base_line: i32| {
                if parser.current().line > base_line {
                    Err(parser.make_error("ambiguous syntax (function call x new statement)"))
                } else {
                    Ok(())
                }
            };

            if self.check(TokenType::LParen) {
                reject_ambiguous_newline(self, self.previous().line)?;
                self.advance(); // consume '('

                let _guard = self.recursion_guard(Self::MAX_BLOCK_RECURSION_DEPTH)?;

                let args = if !self.check(TokenType::RParen) {
                    self.parse_expr_list()?
                } else {
                    Vec::new()
                };

                self.expect(TokenType::RParen, "Expected ')' after arguments")?;

                base = Box::new(Expr::Call(CallExpr {
                    location: SourceLocation::new(line, column),
                    func: base,
                    args,
                    is_method_call: false,
                }));
            } else if self.match_token(TokenType::LBracket) {
                let index = self.parse_expression()?;
                self.expect(TokenType::RBracket, "Expected ']' after index")?;

                base = Box::new(Expr::Index(IndexExpr {
                    location: SourceLocation::new(line, column),
                    table: base,
                    index,
                }));
            } else if self.match_token(TokenType::Dot) {
                if !crate::parser::is_name(self.current()) {
                    return Err(self.make_error("Expected member name after '.'"));
                }

                let member = Self::token_string(self.current()).to_string();
                let loc = SourceLocation::new(line, column);
                self.advance();

                base = Box::new(Expr::Member(MemberExpr {
                    location: loc,
                    table: base,
                    member,
                }));
            } else if self.match_token(TokenType::Colon) {
                if !crate::parser::is_name(self.current()) {
                    return Err(self.make_error("Expected method name after ':'"));
                }

                let method_name = Self::token_string(self.current()).to_string();
                self.advance();

                let member_expr = MemberExpr {
                    location: SourceLocation::new(line, column),
                    table: base,
                    member: method_name,
                };

                let mut call_expr = CallExpr {
                    location: SourceLocation::new(line, column),
                    func: Box::new(Expr::Member(member_expr)),
                    args: Vec::new(),
                    is_method_call: true,
                };

                if self.check(TokenType::LParen) {
                    if self.current().line > line {
                        return Err(
                            self.make_error("ambiguous syntax (function call x new statement)")
                        );
                    }
                    self.advance();
                    let _guard = self.recursion_guard(Self::MAX_BLOCK_RECURSION_DEPTH)?;

                    if !self.check(TokenType::RParen) {
                        call_expr.args = self.parse_expr_list()?;
                    }
                    self.expect(TokenType::RParen, "Expected ')' after arguments")?;
                } else if self.current().is_string() {
                    if self.current().line > line {
                        return Err(
                            self.make_error("ambiguous syntax (function call x new statement)")
                        );
                    }
                    let str_val = Self::token_string(self.current()).to_string();
                    let str_line = self.current().line;
                    let str_col = self.current().column;
                    self.advance();

                    call_expr.args.push(Box::new(Expr::String(StringExpr {
                        location: SourceLocation::new(str_line, str_col),
                        value: str_val,
                    })));
                } else if self.check(TokenType::LBrace) {
                    if self.current().line > line {
                        return Err(
                            self.make_error("ambiguous syntax (function call x new statement)")
                        );
                    }
                    call_expr.args.push(self.parse_table_constructor()?);
                } else {
                    return Err(self.make_error("Expected function arguments after method name"));
                }

                base = Box::new(Expr::Call(call_expr));
            } else if self.current().is_string() {
                // 字符串参数调用：func"string" 或 func'string'
                reject_ambiguous_newline(self, self.previous().line)?;

                let str_val = Self::token_string(self.current()).to_string();
                let str_line = self.current().line;
                let str_col = self.current().column;
                self.advance();

                let call_expr = CallExpr {
                    location: SourceLocation::new(line, column),
                    func: base,
                    args: vec![Box::new(Expr::String(StringExpr {
                        location: SourceLocation::new(str_line, str_col),
                        value: str_val,
                    }))],
                    is_method_call: false,
                };

                base = Box::new(Expr::Call(call_expr));
            } else if self.check(TokenType::LBrace) {
                // 表构造器参数调用：func{...}
                reject_ambiguous_newline(self, self.previous().line)?;

                let call_expr = CallExpr {
                    location: SourceLocation::new(line, column),
                    func: base,
                    args: vec![self.parse_table_constructor()?],
                    is_method_call: false,
                };

                base = Box::new(Expr::Call(call_expr));
            } else {
                break;
            }
        }

        Ok(base)
    }
}
