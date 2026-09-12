//! Lua 语法分析器（Parser）
//!
//! 使用递归下降算法将 Token 流转换为 AST。
//!
//! ## 架构
//! - `Parser` — 主解析器，持有 TokenStream + 错误策略
//! - `TokenStream` — 对 `Lexer` 的包装，提供 current/peek/advance/match
//! - `ErrorRecoveryStrategy` — 错误恢复策略（FailFast / StatementBoundary）
//! - 表达式优先级链（12 级）→ `expr.rs`
//! - 基础表达式与后缀 → `primary.rs`
//! - 语句与块 → `stmt.rs`
//! - 函数定义 → `func.rs`
//! - 表构造器 → `table.rs`
//!

pub mod expr;
pub mod func;
pub mod primary;
pub mod stmt;
pub mod table;

use crate::ast::stmt::Chunk;
use crate::lexer::Lexer;
use crate::token::{Token, TokenType, TokenValue};

use std::fmt;

// =====================================================================
// ParseError
// =====================================================================

/// 语法解析错误
///
#[derive(Debug, Clone)]
pub struct ParseError {
    /// Host-readable view; use `message_bytes` for a Lua diagnostic value.
    pub message: String,
    message_bytes: Vec<u8>,
    pub line: i32,
    pub column: i32,
}

impl ParseError {
    pub fn new(message: impl Into<String>, line: i32, column: i32) -> Self {
        let message = message.into();
        Self {
            message_bytes: message.as_bytes().to_vec(),
            message,
            line,
            column,
        }
    }

    /// Preserve the original diagnostic bytes while providing a readable view.
    pub fn new_bytes(message_bytes: impl Into<Vec<u8>>, line: i32, column: i32) -> Self {
        let message_bytes = message_bytes.into();
        Self {
            message: diagnostic_message_text(&message_bytes),
            message_bytes,
            line,
            column,
        }
    }

    /// Original message bytes, without a source location or host escaping.
    pub fn message_bytes(&self) -> &[u8] {
        &self.message_bytes
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

pub(crate) fn diagnostic_message_text(mut bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    fn push_valid_text(text: &mut String, valid: &str) {
        for ch in valid.chars() {
            if ch.is_control() {
                text.extend(ch.escape_default());
            } else {
                text.push(ch);
            }
        }
    }

    let mut text = String::new();
    loop {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                push_valid_text(&mut text, valid);
                return text;
            }
            Err(error) => {
                let valid_len = error.valid_up_to();
                push_valid_text(
                    &mut text,
                    std::str::from_utf8(&bytes[..valid_len])
                        .expect("UTF-8 validation identified a valid prefix"),
                );
                let invalid_len = error.error_len().unwrap_or(bytes.len() - valid_len);
                for byte in &bytes[valid_len..valid_len + invalid_len] {
                    write!(text, "\\x{byte:02x}").expect("writing into a String cannot fail");
                }
                bytes = &bytes[valid_len + invalid_len..];
            }
        }
    }
}

// =====================================================================
// ParseRecoveryMode
// =====================================================================

/// 错误恢复模式
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseRecoveryMode {
    /// 首次错误即退出
    FailFast,
    /// 尝试在语句边界恢复
    StatementBoundary,
}

// =====================================================================
// ParserOptions
// =====================================================================

/// 解析器配置
///
#[derive(Debug, Clone)]
pub struct ParserOptions {
    pub recovery_mode: ParseRecoveryMode,
}

impl Default for ParserOptions {
    fn default() -> Self {
        Self {
            recovery_mode: ParseRecoveryMode::FailFast,
        }
    }
}

// =====================================================================
// FunctionSyntaxScope
// =====================================================================

/// 函数语法作用域（用于跟踪局部变量和上值）
///
#[derive(Debug, Clone)]
struct FunctionSyntaxScope {
    line: i32,
    locals: Vec<String>,
    upvalues: Vec<String>,
}

// =====================================================================
// TokenStream — 对 Lexer 的包装
// =====================================================================

/// Token 流包装器
///
/// 提供 LL(1) 前瞻和 Token 消费能力。
///
struct TokenStream<'source> {
    lexer: Lexer<'source>,
    current: Token,
    previous: Token,
}

impl<'source> TokenStream<'source> {
    fn from_bytes(source: &'source [u8]) -> Self {
        let mut lexer = Lexer::from_bytes(source);
        let first = lexer.next_token();
        let prev = first.clone();
        Self {
            lexer,
            current: first,
            previous: prev,
        }
    }

    fn current(&self) -> &Token {
        &self.current
    }

    fn previous(&self) -> &Token {
        &self.previous
    }

    fn advance(&mut self) {
        self.previous = self.current.clone();
        self.current = self.lexer.next_token();
    }

    fn peek(&mut self) -> Token {
        self.lexer.peek_token()
    }

    fn check(&self, token_type: TokenType) -> bool {
        self.current.token_type == token_type
    }

    /// 如果当前 Token 匹配，前进并返回 true
    fn match_token(&mut self, token_type: TokenType) -> bool {
        if self.check(token_type) {
            self.advance();
            true
        } else {
            false
        }
    }
}

// =====================================================================
// ParseState — 递归深度跟踪
// =====================================================================

/// 解析状态
///
#[derive(Clone)]
struct ParseState {
    recursion_depth: std::rc::Rc<std::cell::Cell<i32>>,
}

impl ParseState {
    fn new() -> Self {
        Self {
            recursion_depth: std::rc::Rc::new(std::cell::Cell::new(0)),
        }
    }

    fn enter_syntax_level(&self) -> i32 {
        let depth = self.recursion_depth.get() + 1;
        self.recursion_depth.set(depth);
        depth
    }

    fn leave_syntax_level(&self) {
        self.recursion_depth.set(self.recursion_depth.get() - 1);
    }
}

// =====================================================================
// RecursionGuard
// =====================================================================

/// 递归深度守卫（RAII 风格）
///
/// 在进入递归解析函数时增加深度计数，离开（Drop）时自动减少。
/// 如果超出最大深度则返回错误。
///
/// The counter is separately owned so recursive `&mut Parser` borrows cannot
/// invalidate a pointer into Parser storage. The guard uses no raw references.
///
struct RecursionGuard {
    state: ParseState,
}

impl RecursionGuard {
    fn new(parse_state: &ParseState, max_depth: i32, token: &Token) -> Result<Self, ParseError> {
        if parse_state.enter_syntax_level() > max_depth {
            parse_state.leave_syntax_level();
            return Err(ParseError::new(
                "chunk has too many syntax levels",
                token.line,
                token.column,
            ));
        }
        Ok(Self {
            state: parse_state.clone(),
        })
    }
}

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        self.state.leave_syntax_level();
    }
}

// =====================================================================
// Error recovery strategies
// =====================================================================

#[allow(dead_code)]
trait ErrorRecoveryStrategy {
    fn can_recover(&self, _error: &ParseError) -> bool;
    fn recover(&self, parser: &mut Parser);
}

struct FailFastRecovery;

impl ErrorRecoveryStrategy for FailFastRecovery {
    fn can_recover(&self, _error: &ParseError) -> bool {
        false
    }

    fn recover(&self, _parser: &mut Parser) {
        // 不做任何恢复
    }
}

struct StatementBoundaryRecovery;

impl ErrorRecoveryStrategy for StatementBoundaryRecovery {
    fn can_recover(&self, _error: &ParseError) -> bool {
        true
    }

    fn recover(&self, parser: &mut Parser) {
        parser.synchronize();
    }
}

fn make_recovery_strategy(mode: ParseRecoveryMode) -> Box<dyn ErrorRecoveryStrategy> {
    match mode {
        ParseRecoveryMode::StatementBoundary => Box::new(StatementBoundaryRecovery),
        ParseRecoveryMode::FailFast => Box::new(FailFastRecovery),
    }
}

// =====================================================================
// Parser
// =====================================================================

/// Lua 5.1 语法分析器
///
/// 使用递归下降算法解析 Lua 源代码。
///
pub struct Parser<'source> {
    /// Token 流
    token_stream: TokenStream<'source>,
    /// 解析状态（递归深度）
    parse_state: ParseState,
    /// 嵌套函数作用域栈
    function_scopes: Vec<FunctionSyntaxScope>,
    /// 收集的诊断信息
    diagnostics: Vec<ParseError>,
    /// 错误恢复策略
    #[allow(dead_code)]
    recovery: Box<dyn ErrorRecoveryStrategy>,
}

impl<'source> Parser<'source> {
    // ── 常量 ────────────────────────────────────────────────────────

    const MAX_RECURSION_DEPTH: i32 = 92;
    const MAX_BLOCK_RECURSION_DEPTH: i32 = 80;
    const MAX_RIGHT_ASSOC_RECURSION_DEPTH: i32 = 200;
    const MAX_LOCAL_VARIABLES: usize = 200;
    const MAX_UPVALUES_PER_FUNCTION: usize = 60;

    // ── 构造 ────────────────────────────────────────────────────────

    pub fn new(source: &'source str) -> Self {
        Self::with_options(source, ParserOptions::default())
    }

    pub fn with_options(source: &'source str, options: ParserOptions) -> Self {
        Self::with_options_bytes(source.as_bytes(), options)
    }

    /// 从任意 Lua 源码字节创建 Parser。
    ///
    /// 字符串字面量和长字符串保持原始字节；语法 token 只接受显式的
    /// ASCII 词法范围。[`Parser::new`] 是保留的 UTF-8 文本包装入口。
    pub fn from_bytes(source: &'source [u8]) -> Self {
        Self::with_options_bytes(source, ParserOptions::default())
    }

    /// 从任意 Lua 源码字节和指定配置创建 Parser。
    pub fn with_options_bytes(source: &'source [u8], options: ParserOptions) -> Self {
        let mut parser = Self {
            token_stream: TokenStream::from_bytes(source),
            parse_state: ParseState::new(),
            function_scopes: Vec::new(),
            diagnostics: Vec::new(),
            recovery: make_recovery_strategy(options.recovery_mode),
        };
        // 进入顶层函数语法作用域
        parser.enter_function_syntax_scope(1, &[]);
        parser
    }

    // ── 公开入口 ────────────────────────────────────────────────────

    /// 解析源代码，返回 Chunk 或 ParseError
    ///
    pub fn parse(&mut self) -> Result<Chunk, ParseError> {
        self.diagnostics.clear();
        self.function_scopes.clear();
        self.enter_function_syntax_scope(1, &[]);

        let chunk = self.parse_chunk()?;

        if !self.check(TokenType::Eos) {
            return Err(self.make_error("Expected end of file"));
        }

        if let Some(first_error) = self.diagnostics.first() {
            return Err(first_error.clone());
        }

        Ok(chunk)
    }

    /// 返回诊断信息
    pub fn diagnostics(&self) -> &[ParseError] {
        &self.diagnostics
    }

    /// 解析 Chunk（显式公开入口，与 parse() 相同但不清除状态）
    fn parse_chunk(&mut self) -> Result<Chunk, ParseError> {
        let statements = self.parse_block()?;
        Ok(Chunk { statements })
    }

    // ── Token 流操作 ────────────────────────────────────────────────

    fn current(&self) -> &Token {
        self.token_stream.current()
    }

    fn previous(&self) -> &Token {
        self.token_stream.previous()
    }

    fn advance(&mut self) {
        self.token_stream.advance();
    }

    fn peek(&mut self) -> Token {
        self.token_stream.peek()
    }

    fn check(&self, token_type: TokenType) -> bool {
        self.token_stream.check(token_type)
    }

    fn match_token(&mut self, token_type: TokenType) -> bool {
        self.token_stream.match_token(token_type)
    }

    fn expect(&mut self, token_type: TokenType, message: &str) -> Result<(), ParseError> {
        if !self.match_token(token_type) {
            Err(self.make_error(message))
        } else {
            Ok(())
        }
    }

    // ── 错误处理 ────────────────────────────────────────────────────

    fn make_error(&self, message: &str) -> ParseError {
        let tok = self.current();
        self.error_with_near(message, tok)
    }

    fn make_error_at(&self, token: &Token, message: &str) -> ParseError {
        self.error_with_near(message, token)
    }

    fn error_with_near(&self, message: &str, token: &Token) -> ParseError {
        let diagnostic = if token.token_type == TokenType::Error && !token.error_message.is_empty()
        {
            token.error_message.as_bytes()
        } else {
            message.as_bytes()
        };
        let mut message = diagnostic.to_vec();
        message.extend_from_slice(b" near '");
        message.extend_from_slice(get_token_bytes(token));
        message.push(b'\'');
        ParseError::new_bytes(message, token.line, token.column)
    }

    #[allow(dead_code)] // TODO: used by error recovery path (pending)
    fn publish_diagnostic(&mut self, error: ParseError) {
        self.diagnostics.push(error);
    }

    /// 同步到下一个语句边界
    ///
    fn synchronize(&mut self) {
        while !self.check(TokenType::Eos) {
            if self.match_token(TokenType::Semicolon) {
                return;
            }

            if self.check(TokenType::End)
                || self.check(TokenType::Else)
                || self.check(TokenType::Elseif)
                || self.check(TokenType::Until)
            {
                return;
            }

            if self.check(TokenType::Local)
                || self.check(TokenType::Function)
                || self.check(TokenType::If)
                || self.check(TokenType::While)
                || self.check(TokenType::For)
                || self.check(TokenType::Repeat)
                || self.check(TokenType::Return)
                || self.check(TokenType::Break)
            {
                return;
            }

            self.advance();
        }
    }

    // ── 作用域管理 ──────────────────────────────────────────────────

    fn enter_function_syntax_scope(&mut self, line: i32, params: &[String]) {
        let mut scope = FunctionSyntaxScope {
            line,
            locals: Vec::new(),
            upvalues: Vec::new(),
        };
        for param in params {
            if param != "..." {
                scope.locals.push(param.clone());
            }
        }
        self.function_scopes.push(scope);
    }

    fn leave_function_syntax_scope(&mut self) {
        self.function_scopes.pop();
    }

    fn declare_local_name(&mut self, name: &str, token: &Token) -> Result<(), ParseError> {
        if self.function_scopes.is_empty() {
            return Ok(());
        }

        let scope = self.function_scopes.last().unwrap();
        if scope.locals.iter().any(|n| n == name) {
            return Ok(());
        }

        let scope = self.function_scopes.last_mut().unwrap();
        if scope.locals.len() >= Self::MAX_LOCAL_VARIABLES {
            return Err(ParseError::new(
                format!(
                    "function at line {} has more than 200 local variables",
                    scope.line
                ),
                token.line,
                token.column,
            ));
        }
        scope.locals.push(name.to_string());
        Ok(())
    }

    fn note_name_use(&mut self, name: &str, token: &Token) -> Result<(), ParseError> {
        if self.function_scopes.is_empty() {
            return Ok(());
        }

        let mut owner: i32 = -1;
        for i in (0..self.function_scopes.len()).rev() {
            if self.function_scopes[i].locals.iter().any(|n| n == name) {
                owner = i as i32;
                break;
            }
        }

        if owner < 0 || owner == (self.function_scopes.len() - 1) as i32 {
            return Ok(());
        }

        for i in ((owner + 1) as usize..self.function_scopes.len()).rev() {
            let scope = &self.function_scopes[i];
            if scope.upvalues.iter().any(|n| n == name) {
                continue;
            }
            let scope = &mut self.function_scopes[i];
            if scope.upvalues.len() >= Self::MAX_UPVALUES_PER_FUNCTION {
                return Err(ParseError::new(
                    format!("function at line {} has more than 60 upvalues", scope.line),
                    token.line,
                    token.column,
                ));
            }
            scope.upvalues.push(name.to_string());
        }

        Ok(())
    }

    // ── 递归守卫 ────────────────────────────────────────────────────

    fn recursion_guard(&mut self, max_depth: i32) -> Result<RecursionGuard, ParseError> {
        let token = self.current().clone();
        RecursionGuard::new(&self.parse_state, max_depth, &token)
    }

    // ── Token 工具函数 ──────────────────────────────────────────────

    /// 从字符串 Token 中获取 Lua 原始字节。
    fn token_bytes(token: &Token) -> &[u8] {
        match &token.value {
            TokenValue::String(s) => s.as_bytes(),
            _ => token.lexeme.as_bytes(),
        }
    }

    /// 从语法 Name Token 中获取文本。
    ///
    /// Lexer 仅生成 ASCII Name，因此失败代表内部不变量被破坏，而不是
    /// 隐式替换非法源码字节。
    fn token_name(token: &Token) -> &str {
        std::str::from_utf8(token.lexeme.as_bytes())
            .expect("lexer invariant: identifiers are ASCII")
    }
}

// =====================================================================
// 辅助函数
// =====================================================================

/// 获取 Token 的原始字节表示（用于 Lua 错误消息）
///
fn get_token_bytes(token: &Token) -> &[u8] {
    if token.token_type == TokenType::Eos {
        return b"<eof>";
    }
    if !token.lexeme.is_empty() {
        return token.lexeme.as_bytes();
    }
    token.token_type.name().as_bytes()
}

/// 检查 Token 是否为 Name 类型
fn is_name(token: &Token) -> bool {
    token.token_type == TokenType::Name
}
