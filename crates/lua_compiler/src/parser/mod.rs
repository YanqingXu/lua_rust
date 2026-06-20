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
//! C++ 参考: `lua_cpp/src/compiler/parser/`

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
/// C++ 对应: `Lua::ParseError`
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: i32,
    pub column: i32,
}

impl ParseError {
    pub fn new(message: impl Into<String>, line: i32, column: i32) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

// =====================================================================
// ParseRecoveryMode
// =====================================================================

/// 错误恢复模式
///
/// C++ 对应: `Lua::ParseRecoveryMode`
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
/// C++ 对应: `Lua::ParserOptions`
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
/// C++ 对应: `Lua::Parser::Impl::FunctionSyntaxScope`
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
/// C++ 对应: `Lua::Parser::Impl::TokenStream`
struct TokenStream<'source> {
    lexer: Lexer<'source>,
    current: Token,
    previous: Token,
}

impl<'source> TokenStream<'source> {
    fn new(source: &'source str) -> Self {
        let mut lexer = Lexer::new(source);
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
/// C++ 对应: `Lua::Parser::Impl::ParseState`
struct ParseState {
    recursion_depth: i32,
}

impl ParseState {
    fn new() -> Self {
        Self { recursion_depth: 0 }
    }

    fn enter_syntax_level(&mut self) -> i32 {
        self.recursion_depth += 1;
        self.recursion_depth
    }

    fn leave_syntax_level(&mut self) {
        self.recursion_depth -= 1;
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
/// SAFETY: `state` 是从 `&mut ParseState` 派生的裸指针。
/// 守卫的生命周期始终短于 Parser（它在 `&mut self` 方法内创建），
/// 因此该指针在守卫的 Drop 期间始终有效。
///
/// C++ 对应: `Lua::Parser::Impl::RecursionGuard`
struct RecursionGuard {
    state: *mut ParseState,
    entered: bool,
}

impl RecursionGuard {
    fn new(
        parse_state: &mut ParseState,
        max_depth: i32,
        token: &Token,
    ) -> Result<Self, ParseError> {
        if parse_state.enter_syntax_level() > max_depth {
            parse_state.leave_syntax_level();
            return Err(ParseError::new(
                "chunk has too many syntax levels",
                token.line,
                token.column,
            ));
        }
        Ok(Self {
            state: parse_state as *mut ParseState,
            entered: true,
        })
    }
}

impl Drop for RecursionGuard {
    fn drop(&mut self) {
        if self.entered {
            // SAFETY: self.state is derived from a &mut ParseState that
            // outlives this guard (the guard is always dropped before the
            // parser method returns, and ParseState is owned by Parser).
            unsafe {
                (*self.state).leave_syntax_level();
            }
        }
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
/// C++ 对应: `Lua::Parser`
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
        let mut parser = Self {
            token_stream: TokenStream::new(source),
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
    /// C++ 对应: `Lua::Parser::parse()`
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
            token.error_message.clone()
        } else {
            message.to_string()
        };
        let near_text = get_token_text(token);
        ParseError::new(
            format!("{} near '{}'", diagnostic, near_text),
            token.line,
            token.column,
        )
    }

    #[allow(dead_code)] // TODO: used by error recovery path (pending)
    fn publish_diagnostic(&mut self, error: ParseError) {
        self.diagnostics.push(error);
    }

    /// 同步到下一个语句边界
    ///
    /// C++ 对应: `Lua::Parser::Impl::synchronize()`
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
        RecursionGuard::new(&mut self.parse_state, max_depth, &token)
    }

    // ── Token 工具函数 ──────────────────────────────────────────────

    /// 从 Token 中获取字符串值
    fn token_string(token: &Token) -> &str {
        match &token.value {
            TokenValue::String(s) => s.as_str(),
            _ => token.lexeme.as_str(),
        }
    }
}

// =====================================================================
// 辅助函数
// =====================================================================

/// 获取 Token 的文本表示（用于错误消息）
///
/// C++ 对应: `Lua::getTokenText()`
fn get_token_text(token: &Token) -> String {
    if token.token_type == TokenType::Eos {
        return "<eof>".to_string();
    }
    if !token.lexeme.is_empty() {
        return token.lexeme.clone();
    }
    token.token_type.to_string()
}

/// 检查 Token 是否为 Name 类型
fn is_name(token: &Token) -> bool {
    token.token_type == TokenType::Name
}
