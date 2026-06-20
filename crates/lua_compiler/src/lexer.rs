//! Lua 词法分析器
//!
//! 将 Lua 源代码文本转换为 Token 流。支持：
//! - 所有 Lua 5.1 关键字、运算符和字面量
//! - 单行注释（`--`）和多行注释（`--[[ ]]`）
//! - 长字符串（`[[ ]]` 和 `[=[ ]=]`）
//! - 行号和列号跟踪
//! - Token 预读机制（LL(1) 前瞻）
//!
//! C++ 参考: `lua_cpp/src/compiler/lexer/lexer.hpp`

use std::collections::HashMap;

use crate::token::{Token, TokenType, TokenValue};

// =====================================================================
// 关键字表
// =====================================================================

/// 构建 Lua 5.1 关键字映射表（懒初始化）
fn keyword_table() -> &'static HashMap<&'static str, TokenType> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<HashMap<&str, TokenType>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut m = HashMap::new();
        m.insert("and", TokenType::And);
        m.insert("break", TokenType::Break);
        m.insert("do", TokenType::Do);
        m.insert("else", TokenType::Else);
        m.insert("elseif", TokenType::Elseif);
        m.insert("end", TokenType::End);
        m.insert("false", TokenType::False);
        m.insert("for", TokenType::For);
        m.insert("function", TokenType::Function);
        m.insert("if", TokenType::If);
        m.insert("in", TokenType::In);
        m.insert("local", TokenType::Local);
        m.insert("nil", TokenType::Nil);
        m.insert("not", TokenType::Not);
        m.insert("or", TokenType::Or);
        m.insert("repeat", TokenType::Repeat);
        m.insert("return", TokenType::Return);
        m.insert("then", TokenType::Then);
        m.insert("true", TokenType::True);
        m.insert("until", TokenType::Until);
        m.insert("while", TokenType::While);
        m
    })
}

/// 检查标识符是否为关键字，返回对应的 TokenType
fn lookup_keyword(ident: &str) -> Option<TokenType> {
    keyword_table().get(ident).copied()
}

// =====================================================================
// Lexer 结构体
// =====================================================================

/// Lua 词法分析器
///
/// 从源代码字符串中提取 Token 流。
pub struct Lexer<'source> {
    /// 完整的源代码（生命周期锚点）
    _source: &'source str,
    /// 剩余待扫描的源代码（字节切片）
    rest: &'source str,
    /// 当前行号（1-based）
    line: i32,
    /// 当前列号（1-based）
    column: i32,
    /// Token 预读缓存
    lookahead: Option<Token>,
}

impl<'source> Lexer<'source> {
    /// 从源代码字符串创建 Lexer
    pub fn new(source: &'source str) -> Self {
        Self {
            _source: source,
            rest: source,
            line: 1,
            column: 1,
            lookahead: None,
        }
    }

    // ── 公共接口 ──────────────────────────────────────────────────

    /// 获取下一个 Token（消费）
    pub fn next_token(&mut self) -> Token {
        if let Some(tok) = self.lookahead.take() {
            return tok;
        }
        self.scan_token()
    }

    /// 预读下一个 Token（不消费）
    pub fn peek_token(&mut self) -> Token {
        if self.lookahead.is_none() {
            let tok = self.scan_token();
            self.lookahead = Some(tok);
        }
        self.lookahead.clone().unwrap()
    }

    /// 检查是否到达源代码末尾
    #[inline]
    pub fn is_at_end(&self) -> bool {
        self.rest.is_empty()
    }

    /// 获取当前行号
    #[inline]
    pub fn current_line(&self) -> i32 {
        self.line
    }

    /// 获取当前列号
    #[inline]
    pub fn current_column(&self) -> i32 {
        self.column
    }

    // ── 字符操作 ──────────────────────────────────────────────────

    /// 前进一个字符并返回
    fn advance(&mut self) -> Option<char> {
        let c = self.rest.chars().next()?;
        let len = c.len_utf8();
        self.rest = &self.rest[len..];
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(c)
    }

    /// 查看当前字符（不前进）
    fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    /// 查看下一个字符（不前进）
    fn peek_next(&self) -> Option<char> {
        let mut chars = self.rest.chars();
        chars.next(); // skip current
        chars.next()
    }

    /// 如果当前字符匹配，则前进并返回 true
    fn match_char(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    // ── 位置记录 ──────────────────────────────────────────────────

    /// 记录当前扫描位置（用于构建 Token 位置信息）
    fn current_location(&self) -> (i32, i32) {
        (self.line, self.column)
    }

    // ── 核心扫描 ──────────────────────────────────────────────────

    /// 扫描下一个 Token（核心函数）
    fn scan_token(&mut self) -> Token {
        // 跳过空白和注释
        loop {
            let c = match self.peek() {
                Some(c) => c,
                None => return self.make_eos(),
            };

            match c {
                // 空白字符
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance();
                    continue;
                }
                // 注释
                '-' if self.peek_next() == Some('-') => {
                    self.advance(); // consume first '-'
                    self.advance(); // consume second '-'
                    if let Some(err) = self.skip_comment() {
                        return err;
                    }
                    continue;
                }
                _ => break,
            }
        }

        let (line, col) = self.current_location();
        let c = self.advance().unwrap();

        match c {
            // 单字符分隔符
            '(' => self.make_token(
                TokenType::from_char(c).unwrap_or(TokenType::Error),
                c,
                line,
                col,
            ),
            ')' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),
            '{' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),
            '}' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),
            ';' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),
            ',' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),
            ']' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),

            // [ 可能是长字符串
            '[' => self.try_long_string_or_token(line, col),

            // 单字符运算符
            '+' | '*' | '/' | '^' | '%' | '-' | '#' => self.make_token(
                TokenType::from_char(c).unwrap_or(TokenType::Error),
                c,
                line,
                col,
            ),

            ':' => self.make_token(TokenType::from_char(c).unwrap(), c, line, col),

            '<' => {
                let tt = if self.match_char('=') {
                    TokenType::Le
                } else {
                    TokenType::from_char(c).unwrap_or(TokenType::Error)
                };
                let lex = if tt == TokenType::Le {
                    "<=".to_string()
                } else {
                    c.to_string()
                };
                self.make_token(tt, lex, line, col)
            }

            '>' => {
                let tt = if self.match_char('=') {
                    TokenType::Ge
                } else {
                    TokenType::from_char(c).unwrap_or(TokenType::Error)
                };
                let lex = if tt == TokenType::Ge {
                    ">=".to_string()
                } else {
                    c.to_string()
                };
                self.make_token(tt, lex, line, col)
            }

            '=' => {
                let tt = if self.match_char('=') {
                    TokenType::Eq
                } else {
                    TokenType::from_char(c).unwrap_or(TokenType::Error)
                };
                let lex = if tt == TokenType::Eq {
                    "==".to_string()
                } else {
                    c.to_string()
                };
                self.make_token(tt, lex, line, col)
            }

            '~' => {
                if self.match_char('=') {
                    self.make_token(TokenType::Ne, "~=".to_string(), line, col)
                } else {
                    Token::new_error("unexpected symbol '~'".to_string(), line, col)
                }
            }

            '.' => {
                if self.match_char('.') {
                    if self.match_char('.') {
                        self.make_token(TokenType::Dots, "...".to_string(), line, col)
                    } else {
                        self.make_token(TokenType::Concat, "..".to_string(), line, col)
                    }
                } else if self.peek().is_some_and(|nc| nc.is_ascii_digit()) {
                    // 小数：.5 → 0.5
                    self.scan_fractional_number(line, col)
                } else {
                    self.make_token(TokenType::from_char(c).unwrap(), c, line, col)
                }
            }

            // 字符串
            '"' | '\'' => self.scan_short_string(c, line, col),

            // 数字
            '0'..='9' => self.scan_number(c, line, col),

            // 标识符或关键字
            'a'..='z' | 'A'..='Z' | '_' => self.scan_identifier(c, line, col),

            // 非法字符
            _ => Token::new_error(format!("unexpected character '{}'", c), line, col),
        }
    }

    // ── 数字扫描 ──────────────────────────────────────────────────

    fn scan_number(&mut self, first: char, line: i32, col: i32) -> Token {
        let mut lexeme = String::from(first);

        // 检查十六进制
        if first == '0' && self.peek() == Some('x') {
            lexeme.push('x');
            self.advance();
            self.scan_hex_number(lexeme, line, col)
        } else {
            self.scan_decimal_number(lexeme, line, col)
        }
    }

    fn scan_decimal_number(&mut self, mut lexeme: String, line: i32, col: i32) -> Token {
        // 整数部分
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            lexeme.push(self.advance().unwrap());
        }

        // 小数部分
        if self.peek() == Some('.') && self.peek_next().is_some_and(|c| c.is_ascii_digit()) {
            lexeme.push(self.advance().unwrap()); // '.'
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                lexeme.push(self.advance().unwrap());
            }
        }

        // 指数部分
        if self.peek() == Some('e') || self.peek() == Some('E') {
            let next = self.peek_next();
            if next.is_some_and(|c| c.is_ascii_digit())
                || ((next == Some('+') || next == Some('-'))
                    && self.peek_ahead(2).is_some_and(|c| c.is_ascii_digit()))
            {
                lexeme.push(self.advance().unwrap()); // 'e' or 'E'
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    lexeme.push(self.advance().unwrap());
                }
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    lexeme.push(self.advance().unwrap());
                }
            }
        }

        // 解析数值
        let value: f64 = lexeme.parse().unwrap_or(0.0);
        Token::new_number(lexeme, value, line, col)
    }

    fn scan_hex_number(&mut self, mut lexeme: String, line: i32, col: i32) -> Token {
        // 十六进制数字
        while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
            lexeme.push(self.advance().unwrap());
        }

        // 小数部分 (0x1.2p3)
        if self.peek() == Some('.') && self.peek_next().is_some_and(|c| c.is_ascii_hexdigit()) {
            lexeme.push(self.advance().unwrap()); // '.'
            while self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                lexeme.push(self.advance().unwrap());
            }
        }

        // 指数部分 (p+3, p-3)
        if self.peek() == Some('p') || self.peek() == Some('P') {
            lexeme.push(self.advance().unwrap());
            if self.peek() == Some('+') || self.peek() == Some('-') {
                lexeme.push(self.advance().unwrap());
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                lexeme.push(self.advance().unwrap());
            }
        }

        // 解析为 f64
        let without_prefix = lexeme.strip_prefix("0x").unwrap_or(&lexeme);
        let value: f64 = if let Ok(v) = i64::from_str_radix(without_prefix, 16) {
            v as f64
        } else {
            0.0
        };

        Token::new_number(lexeme, value, line, col)
    }

    /// 扫描以 '.' 开头的小数（如 .5）
    fn scan_fractional_number(&mut self, line: i32, col: i32) -> Token {
        let mut lexeme = String::from(".");
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            lexeme.push(self.advance().unwrap());
        }

        let value: f64 = lexeme.parse().unwrap_or(0.0);
        Token::new_number(lexeme, value, line, col)
    }

    // ── 字符串扫描 ────────────────────────────────────────────────

    fn scan_short_string(&mut self, quote: char, line: i32, col: i32) -> Token {
        let mut lexeme = String::from(quote);
        let mut value = String::new();

        loop {
            match self.advance() {
                None => {
                    return Token::new_error("unterminated string".to_string(), line, col);
                }
                Some('\n') | Some('\r') => {
                    return Token::new_error("unterminated string".to_string(), line, col);
                }
                Some(c) if c == quote => {
                    lexeme.push(c);
                    return Token::new_string(lexeme, value, line, col);
                }
                Some('\\') => {
                    lexeme.push('\\');
                    match self.advance() {
                        None => {
                            return Token::new_error("unterminated string".to_string(), line, col);
                        }
                        Some(ec) => {
                            lexeme.push(ec);
                            match ec {
                                'a' => value.push('\x07'),
                                'b' => value.push('\x08'),
                                'f' => value.push('\x0C'),
                                'n' => value.push('\n'),
                                'r' => value.push('\r'),
                                't' => value.push('\t'),
                                'v' => value.push('\x0B'),
                                '\\' => value.push('\\'),
                                '"' => value.push('"'),
                                '\'' => value.push('\''),
                                '\n' | '\r' => {
                                    // 跨行转义：跳过换行
                                    if ec == '\r' && self.peek() == Some('\n') {
                                        lexeme.push(self.advance().unwrap());
                                    }
                                }
                                '0'..='9' => {
                                    // 十进制转义 \ddd
                                    let mut digits = String::from(ec);
                                    for _ in 0..2 {
                                        if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                                            let d = self.advance().unwrap();
                                            lexeme.push(d);
                                            digits.push(d);
                                        } else {
                                            break;
                                        }
                                    }
                                    let code: u32 = digits.parse().unwrap_or(0);
                                    if let Some(ch) = char::from_u32(code) {
                                        value.push(ch);
                                    }
                                }
                                'x' => {
                                    // 十六进制转义 \xXX
                                    let mut hex = String::new();
                                    for _ in 0..2 {
                                        if self.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
                                            let d = self.advance().unwrap();
                                            lexeme.push(d);
                                            hex.push(d);
                                        } else {
                                            break;
                                        }
                                    }
                                    if let Ok(code) = u32::from_str_radix(&hex, 16)
                                        && let Some(ch) = char::from_u32(code)
                                    {
                                        value.push(ch);
                                    }
                                }
                                _ => {
                                    // 未知转义序列 → 保留原字符
                                    value.push(ec);
                                }
                            }
                        }
                    }
                }
                Some(c) => {
                    lexeme.push(c);
                    value.push(c);
                }
            }
        }
    }

    fn try_long_string_or_token(&mut self, line: i32, col: i32) -> Token {
        // 检查是否以 [ 或 [=*[ 开头
        let rest_snapshot = self.rest;
        let line_snapshot = self.line;
        let col_snapshot = self.column;

        // 读取 = 的数量
        let mut level = 0i32;
        while self.peek() == Some('=') {
            self.advance();
            level += 1;
        }

        if self.peek() == Some('[') {
            self.advance(); // consume second '['
            return self.scan_long_string(level, line, col);
        }

        // 不是长字符串 — 回退
        self.rest = rest_snapshot;
        self.line = line_snapshot;
        self.column = col_snapshot;
        self.make_token(TokenType::from_char('[').unwrap(), "[", line, col)
    }

    fn scan_long_string(&mut self, level: i32, line: i32, col: i32) -> Token {
        let mut lexeme = String::from("[");
        for _ in 0..level {
            lexeme.push('=');
        }
        lexeme.push('[');

        let mut value = String::new();

        // Lua 5.1: skip first newline after opening bracket
        if self.peek() == Some('\n') {
            lexeme.push(self.advance().unwrap());
        } else if self.peek() == Some('\r') {
            lexeme.push(self.advance().unwrap());
            if self.peek() == Some('\n') {
                lexeme.push(self.advance().unwrap());
            }
        }

        loop {
            match self.advance() {
                None => {
                    return Token::new_error("unfinished long string".to_string(), line, col);
                }
                Some(']') => {
                    lexeme.push(']');
                    // 检查是否是结束分隔符 ]=*]
                    let mut close_level = 0i32;
                    while close_level < level && self.peek() == Some('=') {
                        lexeme.push(self.advance().unwrap());
                        close_level += 1;
                    }
                    if close_level == level && self.peek() == Some(']') {
                        lexeme.push(self.advance().unwrap());
                        return Token::new_string(lexeme, value, line, col);
                    }
                    // 不是结束符，继续添加
                    value.push(']');
                    for _ in 0..close_level {
                        value.push('=');
                    }
                }
                Some('\r') => {
                    lexeme.push('\r');
                    // Lua 5.1: normalize \r\n → \n
                    if self.peek() == Some('\n') {
                        lexeme.push(self.advance().unwrap());
                    }
                    value.push('\n');
                }
                Some(c) => {
                    lexeme.push(c);
                    value.push(c);
                }
            }
        }
    }

    // ── 标识符/关键字扫描 ────────────────────────────────────────

    fn scan_identifier(&mut self, first: char, line: i32, col: i32) -> Token {
        let mut lexeme = String::from(first);

        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            lexeme.push(self.advance().unwrap());
        }

        // 检查是否为关键字
        if let Some(kw_type) = lookup_keyword(&lexeme) {
            // 特殊处理 true/false/nil
            let value = match kw_type {
                TokenType::True => TokenValue::None, // 在 Lua 中 true 是关键词，不是字面量
                TokenType::False => TokenValue::None,
                TokenType::Nil => TokenValue::None,
                _ => TokenValue::None,
            };
            let mut tok = Token::new(kw_type, lexeme, line, col);
            tok.value = value;
            tok
        } else {
            Token::new(TokenType::Name, lexeme, line, col)
        }
    }

    // ── 注释处理 ──────────────────────────────────────────────────

    /// 跳过注释。如果遇到未闭合的长注释，返回错误 Token。
    fn skip_comment(&mut self) -> Option<Token> {
        let (line, col) = (self.line, self.column);

        // 检查是否为长注释 --[[ 或 --[=[ ... ]=]
        if self.peek() == Some('[') {
            self.advance(); // consume '['

            let mut level = 0i32;
            while self.peek() == Some('=') {
                self.advance();
                level += 1;
            }

            if self.peek() == Some('[') {
                self.advance(); // consume second '['
                return self.skip_long_comment(level, line, col);
            }

            // 不是长注释，是普通的 --[identifier]... 行注释
            self.skip_line_comment();
            return None;
        }

        self.skip_line_comment();
        None
    }

    fn skip_line_comment(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' || c == '\r' {
                break;
            }
            self.advance();
        }
    }

    fn skip_long_comment(&mut self, level: i32, start_line: i32, start_col: i32) -> Option<Token> {
        // Lua 5.1: skip first newline after opening bracket
        if self.peek() == Some('\n') {
            self.advance();
        } else if self.peek() == Some('\r') {
            self.advance();
            if self.peek() == Some('\n') {
                self.advance();
            }
        }

        loop {
            match self.advance() {
                None => {
                    return Some(Token::new_error(
                        "unfinished long comment".to_string(),
                        start_line,
                        start_col,
                    ));
                }
                Some(']') => {
                    let mut close_level = 0i32;
                    while close_level < level && self.peek() == Some('=') {
                        self.advance();
                        close_level += 1;
                    }
                    if close_level == level && self.peek() == Some(']') {
                        self.advance();
                        return None; // 成功闭合
                    }
                }
                _ => {}
            }
        }
    }

    // ── Token 构造辅助 ────────────────────────────────────────────

    fn make_token(
        &self,
        token_type: TokenType,
        lexeme: impl Into<String>,
        line: i32,
        column: i32,
    ) -> Token {
        Token::new(token_type, lexeme.into(), line, column)
    }

    fn make_eos(&self) -> Token {
        Token::eos(self.line, self.column)
    }

    /// 前瞻 N 个字符
    fn peek_ahead(&self, n: usize) -> Option<char> {
        self.rest.chars().nth(n)
    }
}

// =====================================================================
// TokenType 从字符构造的辅助实现
// =====================================================================

impl TokenType {
    /// 从单字符获取 TokenType
    fn from_char(c: char) -> Option<TokenType> {
        match c {
            '+' => Some(TokenType::Plus),
            '-' => Some(TokenType::Minus),
            '*' => Some(TokenType::Star),
            '/' => Some(TokenType::Slash),
            '^' => Some(TokenType::Caret),
            '%' => Some(TokenType::Percent),
            '=' => Some(TokenType::Assign),
            '<' => Some(TokenType::Lt),
            '>' => Some(TokenType::Gt),
            '(' => Some(TokenType::LParen),
            ')' => Some(TokenType::RParen),
            '{' => Some(TokenType::LBrace),
            '}' => Some(TokenType::RBrace),
            '[' => Some(TokenType::LBracket),
            ']' => Some(TokenType::RBracket),
            ';' => Some(TokenType::Semicolon),
            ',' => Some(TokenType::Comma),
            '.' => Some(TokenType::Dot),
            '#' => Some(TokenType::Len),
            ':' => Some(TokenType::Colon),
            _ => None,
        }
    }
}

// =====================================================================
// Lexer 综合测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 辅助函数：扫描所有 Token 并返回 Vec
    fn scan_all(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let tok = lexer.next_token();
            let is_eos = tok.is_eos();
            tokens.push(tok);
            if is_eos {
                break;
            }
        }
        tokens
    }

    /// 辅助函数：扫描并仅返回非 EOS Token
    fn scan_non_eos(source: &str) -> Vec<Token> {
        let mut tokens = scan_all(source);
        tokens.pop(); // remove EOS
        tokens
    }

    // ── 关键字测试 ────────────────────────────────────────────────

    #[test]
    fn test_keywords() {
        let keywords = [
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
            "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
        ];
        for kw in keywords {
            let tokens = scan_non_eos(kw);
            assert_eq!(tokens.len(), 1, "Failed for keyword: {}", kw);
            assert!(tokens[0].is_keyword(), "Not a keyword: {}", kw);
            assert_eq!(tokens[0].lexeme, kw);
        }
    }

    #[test]
    fn test_keyword_count() {
        assert_eq!(keyword_table().len(), 21);
    }

    // ── 标识符测试 ────────────────────────────────────────────────

    #[test]
    fn test_identifiers() {
        let source = "foo bar_123 _private X1";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 4);
        for (i, name) in ["foo", "bar_123", "_private", "X1"].iter().enumerate() {
            assert_eq!(tokens[i].token_type, TokenType::Name);
            assert_eq!(tokens[i].lexeme, *name);
        }
    }

    // ── 数字测试 ──────────────────────────────────────────────────

    #[test]
    fn test_integer_numbers() {
        let source = "0 42 123 999";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 4);
        for tok in &tokens {
            assert!(tok.is_number());
        }
        assert!(matches!(tokens[0].value, TokenValue::Number(0.0)));
        assert!(matches!(tokens[1].value, TokenValue::Number(42.0)));
        assert!(matches!(tokens[2].value, TokenValue::Number(123.0)));
    }

    #[test]
    fn test_float_numbers() {
        let source = "3.14 0.5 1.0";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 3);
        assert!(matches!(tokens[0].value, TokenValue::Number(v) if (v - 3.14).abs() < 0.001));
        assert!(matches!(tokens[1].value, TokenValue::Number(0.5)));
    }

    #[test]
    fn test_scientific_notation() {
        let source = "1e10 2.5e-3 0x1F";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 3);
        assert!(matches!(tokens[0].value, TokenValue::Number(1e10)));
        assert!(matches!(tokens[1].value, TokenValue::Number(0.0025)));
        assert!(matches!(tokens[2].value, TokenValue::Number(31.0)));
    }

    #[test]
    fn test_fractional_number() {
        let tokens = scan_non_eos(".5");
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_number());
        assert!(matches!(tokens[0].value, TokenValue::Number(0.5)));
    }

    // ── 字符串测试 ────────────────────────────────────────────────

    #[test]
    fn test_simple_strings() {
        let source = r#""hello" 'world' "#;
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 2);
        assert!(tokens[0].is_string());
        assert!(tokens[1].is_string());
        assert!(matches!(&tokens[0].value, TokenValue::String(s) if s == "hello"));
        assert!(matches!(&tokens[1].value, TokenValue::String(s) if s == "world"));
    }

    #[test]
    fn test_string_escapes() {
        let source = r#""a\nb\tc\\d\"e""#;
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 1);
        assert!(matches!(&tokens[0].value, TokenValue::String(s) if s == "a\nb\tc\\d\"e"));
    }

    #[test]
    fn test_long_string() {
        let source = "[[hello\nworld]]";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_string());
        assert!(matches!(&tokens[0].value, TokenValue::String(s) if s == "hello\nworld"));
    }

    #[test]
    fn test_long_string_with_equals() {
        let source = "[=[hello [world] foo]=]";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_string());
        assert!(matches!(&tokens[0].value, TokenValue::String(s) if s == "hello [world] foo"));
    }

    #[test]
    fn test_empty_long_string() {
        let tokens = scan_non_eos("[[]]");
        assert_eq!(tokens.len(), 1);
        assert!(tokens[0].is_string());
        assert!(matches!(&tokens[0].value, TokenValue::String(s) if s.is_empty()));
    }

    // ── 运算符测试 ────────────────────────────────────────────────

    #[test]
    fn test_arithmetic_operators() {
        let source = "+ - * / ^ %";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn test_compound_operators() {
        let source = ".. ... == >= <= ~=";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].token_type, TokenType::Concat);
        assert_eq!(tokens[1].token_type, TokenType::Dots);
        assert_eq!(tokens[2].token_type, TokenType::Eq);
        assert_eq!(tokens[3].token_type, TokenType::Ge);
        assert_eq!(tokens[4].token_type, TokenType::Le);
        assert_eq!(tokens[5].token_type, TokenType::Ne);
    }

    #[test]
    fn test_brackets_and_delimiters() {
        let source = "( ) { } [ ] ; ,";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 8);
    }

    // ── 注释测试 ──────────────────────────────────────────────────

    #[test]
    fn test_line_comment() {
        let source = "42 -- this is a comment\n43";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0].value, TokenValue::Number(42.0)));
        assert!(matches!(tokens[1].value, TokenValue::Number(43.0)));
    }

    #[test]
    fn test_long_comment() {
        let source = "1 --[[ long\ncomment ]] 2";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[0].value, TokenValue::Number(1.0)));
        assert!(matches!(tokens[1].value, TokenValue::Number(2.0)));
    }

    #[test]
    fn test_long_comment_with_equals() {
        let source = "1 --[=[ nested [comment] ]=] 2";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens.len(), 2);
    }

    // ── 综合测试 ──────────────────────────────────────────────────

    #[test]
    fn test_complete_statement() {
        let source = r#"local x = 42 + 3.14"#;
        let tokens = scan_non_eos(source);
        // local(0) x(1) =(2) 42(3) +(4) 3.14(5)
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].token_type, TokenType::Local);
        assert_eq!(tokens[1].token_type, TokenType::Name);
        assert_eq!(tokens[1].lexeme, "x");
        assert_eq!(tokens[2].token_type, TokenType::Assign);
        assert!(tokens[3].is_number());
        assert_eq!(tokens[4].token_type, TokenType::Plus);
        assert!(tokens[5].is_number());
    }

    #[test]
    fn test_function_definition() {
        let source = "function foo(a, b) return a + b end";
        let tokens = scan_non_eos(source);
        // function(0) foo(1) ((2) a(3) ,(4) b(5) )(6) return(7) a(8) +(9) b(10) end(11)
        let types: Vec<TokenType> = tokens.iter().map(|t| t.token_type).collect();
        assert_eq!(types[0], TokenType::Function);
        assert_eq!(types[1], TokenType::Name); // foo
        assert_eq!(types[7], TokenType::Return);
        assert_eq!(types[11], TokenType::End);
    }

    #[test]
    fn test_if_statement() {
        let source = "if x > 0 then return 1 else return 0 end";
        let tokens = scan_non_eos(source);
        let types: Vec<TokenType> = tokens.iter().map(|t| t.token_type).collect();
        assert_eq!(types[0], TokenType::If);
        assert_eq!(types[1], TokenType::Name);
        // '>' is ASCII char 62
        assert_eq!(types[3], TokenType::Number);
        assert_eq!(types[4], TokenType::Then);
        assert_eq!(types[5], TokenType::Return);
        assert_eq!(types[7], TokenType::Else); // fixed: 1 is not Else, 7 is
    }

    // ── 位置跟踪测试 ──────────────────────────────────────────────

    #[test]
    fn test_line_tracking() {
        let source = "local\nx\n=\n42";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens[0].line, 1); // local
        assert_eq!(tokens[1].line, 2); // x
        assert_eq!(tokens[2].line, 3); // =
        assert_eq!(tokens[3].line, 4); // 42
    }

    #[test]
    fn test_column_tracking() {
        let source = "  x";
        let tokens = scan_non_eos(source);
        assert_eq!(tokens[0].lexeme, "x");
        assert_eq!(tokens[0].column, 3); // after two spaces
    }

    // ── Peek 测试 ─────────────────────────────────────────────────

    #[test]
    fn test_peek_token() {
        let mut lexer = Lexer::new("42 x");
        let peeked = lexer.peek_token();
        assert!(peeked.is_number());
        // Peek again — should return same token
        let peeked2 = lexer.peek_token();
        assert_eq!(peeked2.token_type, peeked.token_type);
        // Consume
        let tok = lexer.next_token();
        assert_eq!(tok.token_type, peeked.token_type);
    }

    // ── 错误处理测试 ──────────────────────────────────────────────

    #[test]
    fn test_unterminated_string() {
        let tokens = scan_all("\"hello");
        assert!(tokens[0].is_error());
    }

    #[test]
    fn test_unexpected_character() {
        let tokens = scan_all("@");
        assert!(tokens[0].is_error());
    }

    #[test]
    fn test_eos() {
        let mut lexer = Lexer::new("");
        let tok = lexer.next_token();
        assert!(tok.is_eos());
    }
}
