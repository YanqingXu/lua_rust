//! Lua 词法标记定义
//!
//! 定义语法分析阶段使用的 Token 类型、语义值和源代码位置信息。
//!
//! C++ 参考: `lua_cpp/src/compiler/parser/token.hpp`

// =====================================================================
// TokenType 枚举
// =====================================================================

/// Lua 词法标记类型
///
/// 关键词从 257 开始以区分 ASCII 单字符标记（0-255 范围）。
/// discriminant 值与 C++ `TokenType` 枚举完全一致。
///
/// C++ 对应: `Lua::TokenType` (enum class : i32)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TokenType {
    // ── 单字符标记（ASCII 值 = discriminant）──────────────────────
    /// `+`
    Plus = 43,
    /// `-`
    Minus = 45,
    /// `*`
    Star = 42,
    /// `/`
    Slash = 47,
    /// `^`
    Caret = 94,
    /// `%`
    Percent = 37,
    /// `=`
    Assign = 61,
    /// `<`
    Lt = 60,
    /// `>`
    Gt = 62,
    /// `(`
    LParen = 40,
    /// `)`
    RParen = 41,
    /// `{`
    LBrace = 123,
    /// `}`
    RBrace = 125,
    /// `[`
    LBracket = 91,
    /// `]`
    RBracket = 93,
    /// `;`
    Semicolon = 59,
    /// `,`
    Comma = 44,
    /// `.`
    Dot = 46,
    /// `#`
    Len = 35,
    /// `:`
    Colon = 58,

    // ── 关键词 (257+) ─────────────────────────────────────────────
    And = 257,
    Break = 258,
    Do = 259,
    Else = 260,
    Elseif = 261,
    End = 262,
    False = 263,
    For = 264,
    Function = 265,
    If = 266,
    In = 267,
    Local = 268,
    Nil = 269,
    Not = 270,
    Or = 271,
    Repeat = 272,
    Return = 273,
    Then = 274,
    True = 275,
    Until = 276,
    While = 277,

    // ── 多字符运算符 ──────────────────────────────────────────────
    /// `..`
    Concat = 278,
    /// `...`
    Dots = 279,
    /// `==`
    Eq = 280,
    /// `>=`
    Ge = 281,
    /// `<=`
    Le = 282,
    /// `~=`
    Ne = 283,

    // ── 字面量类型 ────────────────────────────────────────────────
    Number = 284,
    String = 285,
    /// 标识符
    Name = 286,

    // ── 特殊标记 ──────────────────────────────────────────────────
    /// 输入结束
    Eos = 287,
    /// 词法错误
    Error = 288,
}

impl TokenType {
    /// 检查是否为关键词
    #[inline]
    pub fn is_keyword(&self) -> bool {
        (*self as i32) >= TokenType::And as i32 && (*self as i32) <= TokenType::While as i32
    }

    /// 获取类型名称（用于调试和错误报告）
    pub fn name(&self) -> &'static str {
        match self {
            // 单字符标记
            TokenType::Plus => "+",
            TokenType::Minus => "-",
            TokenType::Star => "*",
            TokenType::Slash => "/",
            TokenType::Caret => "^",
            TokenType::Percent => "%",
            TokenType::Assign => "=",
            TokenType::Lt => "<",
            TokenType::Gt => ">",
            TokenType::LParen => "(",
            TokenType::RParen => ")",
            TokenType::LBrace => "{",
            TokenType::RBrace => "}",
            TokenType::LBracket => "[",
            TokenType::RBracket => "]",
            TokenType::Semicolon => ";",
            TokenType::Comma => ",",
            TokenType::Dot => ".",
            TokenType::Len => "#",
            TokenType::Colon => ":",
            // 关键词
            TokenType::And => "and",
            TokenType::Break => "break",
            TokenType::Do => "do",
            TokenType::Else => "else",
            TokenType::Elseif => "elseif",
            TokenType::End => "end",
            TokenType::False => "false",
            TokenType::For => "for",
            TokenType::Function => "function",
            TokenType::If => "if",
            TokenType::In => "in",
            TokenType::Local => "local",
            TokenType::Nil => "nil",
            TokenType::Not => "not",
            TokenType::Or => "or",
            TokenType::Repeat => "repeat",
            TokenType::Return => "return",
            TokenType::Then => "then",
            TokenType::True => "true",
            TokenType::Until => "until",
            TokenType::While => "while",
            // 复合运算符
            TokenType::Concat => "..",
            TokenType::Dots => "...",
            TokenType::Eq => "==",
            TokenType::Ge => ">=",
            TokenType::Le => "<=",
            TokenType::Ne => "~=",
            // 字面量 / 特殊
            TokenType::Number => "<number>",
            TokenType::String => "<string>",
            TokenType::Name => "<name>",
            TokenType::Eos => "<eos>",
            TokenType::Error => "<error>",
        }
    }
}

impl std::fmt::Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// =====================================================================
// TokenValue — Token 的语义值
// =====================================================================

/// Token 的语义值
///
/// C++ 对应: `Lua::TokenValue` (std::variant<std::monostate, f64, Str>)
#[derive(Debug, Clone)]
pub enum TokenValue {
    /// 无值（关键词、运算符、分隔符）
    None,
    /// 数值字面量
    Number(f64),
    /// 字符串字面量内容（不含引号）
    String(String),
}

// =====================================================================
// Token 结构体
// =====================================================================

/// 词法标记
///
/// C++ 对应: `Lua::Token`
#[derive(Debug, Clone)]
pub struct Token {
    /// 标记类型
    pub token_type: TokenType,
    /// 语义值（数值或字符串）
    pub value: TokenValue,
    /// 源代码词素文本
    pub lexeme: String,
    /// 错误消息（仅 Error 类型）
    pub error_message: String,
    /// 源代码行号（1-based）
    pub line: i32,
    /// 源代码列号（1-based）
    pub column: i32,
}

impl Token {
    /// 创建新的 Token
    pub fn new(token_type: TokenType, lexeme: String, line: i32, column: i32) -> Self {
        Self {
            token_type,
            value: TokenValue::None,
            lexeme,
            error_message: String::new(),
            line,
            column,
        }
    }

    /// 创建带数值的 Token
    pub fn new_number(lexeme: String, value: f64, line: i32, column: i32) -> Self {
        Self {
            token_type: TokenType::Number,
            value: TokenValue::Number(value),
            lexeme,
            error_message: String::new(),
            line,
            column,
        }
    }

    /// 创建带字符串值的 Token
    pub fn new_string(lexeme: String, value: String, line: i32, column: i32) -> Self {
        Self {
            token_type: TokenType::String,
            value: TokenValue::String(value),
            lexeme,
            error_message: String::new(),
            line,
            column,
        }
    }

    /// 创建错误 Token
    pub fn new_error(message: String, line: i32, column: i32) -> Self {
        Self::new_error_with_lexeme(message, String::new(), line, column)
    }

    /// 创建带源词素的错误 Token
    pub fn new_error_with_lexeme(message: String, lexeme: String, line: i32, column: i32) -> Self {
        let msg = message.clone();
        Self {
            token_type: TokenType::Error,
            value: TokenValue::None,
            lexeme,
            error_message: msg,
            line,
            column,
        }
    }

    /// 创建 EOS Token
    pub fn eos(line: i32, column: i32) -> Self {
        Self {
            token_type: TokenType::Eos,
            value: TokenValue::None,
            lexeme: String::new(),
            error_message: String::new(),
            line,
            column,
        }
    }

    // ── 类型检查 ──────────────────────────────────────────────────

    #[inline]
    pub fn is_number(&self) -> bool {
        self.token_type == TokenType::Number
    }

    #[inline]
    pub fn is_string(&self) -> bool {
        self.token_type == TokenType::String
    }

    #[inline]
    pub fn is_name(&self) -> bool {
        self.token_type == TokenType::Name
    }

    #[inline]
    pub fn is_keyword(&self) -> bool {
        self.token_type.is_keyword()
    }

    #[inline]
    pub fn is_eos(&self) -> bool {
        self.token_type == TokenType::Eos
    }

    #[inline]
    pub fn is_error(&self) -> bool {
        self.token_type == TokenType::Error
    }
}

impl Default for Token {
    fn default() -> Self {
        Self::eos(1, 1)
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_type_discriminants() {
        assert_eq!(TokenType::And as i32, 257);
        assert_eq!(TokenType::Break as i32, 258);
        assert_eq!(TokenType::While as i32, 277);
        assert_eq!(TokenType::Concat as i32, 278);
        assert_eq!(TokenType::Dots as i32, 279);
        assert_eq!(TokenType::Eq as i32, 280);
        assert_eq!(TokenType::Ge as i32, 281);
        assert_eq!(TokenType::Le as i32, 282);
        assert_eq!(TokenType::Ne as i32, 283);
        assert_eq!(TokenType::Number as i32, 284);
        assert_eq!(TokenType::String as i32, 285);
        assert_eq!(TokenType::Name as i32, 286);
        assert_eq!(TokenType::Eos as i32, 287);
        assert_eq!(TokenType::Error as i32, 288);
    }

    #[test]
    fn test_is_keyword() {
        assert!(TokenType::And.is_keyword());
        assert!(TokenType::While.is_keyword());
        assert!(!TokenType::Number.is_keyword());
        assert!(!TokenType::Name.is_keyword());
        assert!(!TokenType::Eos.is_keyword());
    }

    #[test]
    fn test_token_name() {
        assert_eq!(TokenType::And.name(), "and");
        assert_eq!(TokenType::Function.name(), "function");
        assert_eq!(TokenType::Eq.name(), "==");
        assert_eq!(TokenType::Number.name(), "<number>");
        assert_eq!(TokenType::Eos.name(), "<eos>");
    }

    #[test]
    fn test_token_new() {
        let t = Token::new(TokenType::Name, "foo".to_string(), 1, 5);
        assert_eq!(t.token_type, TokenType::Name);
        assert_eq!(t.lexeme, "foo");
        assert_eq!(t.line, 1);
        assert_eq!(t.column, 5);
        assert!(t.is_name());
    }

    #[test]
    fn test_token_new_number() {
        let t = Token::new_number("42".to_string(), 42.0, 3, 10);
        assert!(t.is_number());
        assert!(matches!(t.value, TokenValue::Number(42.0)));
    }

    #[test]
    fn test_token_new_string() {
        let t = Token::new_string("\"hello\"".to_string(), "hello".to_string(), 2, 1);
        assert!(t.is_string());
        assert!(matches!(t.value, TokenValue::String(ref s) if s == "hello"));
    }

    #[test]
    fn test_token_error() {
        let t = Token::new_error("unterminated string".to_string(), 5, 20);
        assert!(t.is_error());
        assert!(!t.error_message.is_empty());
    }

    #[test]
    fn test_token_eos() {
        let t = Token::eos(10, 1);
        assert!(t.is_eos());
    }
}
