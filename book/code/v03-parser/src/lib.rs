use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ByteString(Box<[u8]>);

impl ByteString {
    pub fn new(bytes: impl Into<Box<[u8]>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Display for ByteString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in self.as_bytes() {
            for escaped in std::ascii::escape_default(byte) {
                write!(formatter, "{}", char::from(escaped))?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(ByteString),
}

impl Value {
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Boolean(false))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Nil => formatter.write_str("nil"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => write!(formatter, "\"{value}\""),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnaryOp {
    Negate,
    Not,
    Length,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryOp {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Concat,
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Power,
}

// ANCHOR: ast
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal {
        value: Value,
        position: Position,
    },
    Name {
        name: ByteString,
        position: Position,
    },
    Unary {
        op: UnaryOp,
        expression: Box<Expr>,
        position: Position,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
        position: Position,
    },
    Group {
        expression: Box<Expr>,
        position: Position,
    },
}
// ANCHOR_END: ast

impl Expr {
    pub fn position(&self) -> Position {
        match self {
            Self::Literal { position, .. }
            | Self::Name { position, .. }
            | Self::Unary { position, .. }
            | Self::Binary { position, .. }
            | Self::Group { position, .. } => *position,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Nil,
    False,
    True,
    Not,
    And,
    Or,
    Name(ByteString),
    Number(f64),
    String(ByteString),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Hash,
    EqualEqual,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    DotDot,
    LeftParen,
    RightParen,
    Unsupported(ByteString),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
struct Token {
    kind: TokenKind,
    position: Position,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub position: Position,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.position.line, self.position.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

pub fn parse(source: &[u8]) -> Result<Expr, ParseError> {
    let tokens = Lexer::new(source).tokenize()?;
    Parser::new(tokens).parse()
}

pub fn evaluate(expression: &Expr) -> Result<Value, String> {
    match expression {
        Expr::Literal { value, .. } => Ok(value.clone()),
        Expr::Group { expression, .. } => evaluate(expression),
        Expr::Name { name, position } => Err(format!(
            "{}:{}: names are parsed but not bound yet: {name}",
            position.line, position.column
        )),
        Expr::Unary { op, expression, .. } => {
            let value = evaluate(expression)?;
            match (op, value) {
                (UnaryOp::Negate, Value::Number(value)) => Ok(Value::Number(-value)),
                (UnaryOp::Not, value) => Ok(Value::Boolean(!value.is_truthy())),
                (UnaryOp::Length, Value::String(value)) => {
                    Ok(Value::Number(value.as_bytes().len() as f64))
                }
                _ => {
                    Err("v0.3 only evaluates numeric arithmetic, not and string length".to_owned())
                }
            }
        }
        Expr::Binary {
            left, op, right, ..
        } => {
            let left = evaluate(left)?;
            let right = evaluate(right)?;
            let (Value::Number(left), Value::Number(right)) = (left, right) else {
                return Err("v0.3 arithmetic operands must be numbers".to_owned());
            };
            let result = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Subtract => left - right,
                BinaryOp::Multiply => left * right,
                BinaryOp::Divide => left / right,
                BinaryOp::Modulo => left - (left / right).floor() * right,
                BinaryOp::Power => left.powf(right),
                _ => return Err("operator is parsed now and implemented in v0.4".to_owned()),
            };
            Ok(Value::Number(result))
        }
    }
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn parse(mut self) -> Result<Expr, ParseError> {
        let expression = self.expression(0)?;
        if self.current().kind != TokenKind::Eof {
            return Err(self.error("expected end of input after expression"));
        }
        Ok(expression)
    }

    // ANCHOR: pratt_parser
    fn expression(&mut self, minimum_binding_power: u8) -> Result<Expr, ParseError> {
        // ANCHOR: pratt_prefix
        let token = self.advance().clone();
        let mut left = match token.kind {
            TokenKind::Nil => self.literal(Value::Nil, token.position),
            TokenKind::False => self.literal(Value::Boolean(false), token.position),
            TokenKind::True => self.literal(Value::Boolean(true), token.position),
            TokenKind::Number(value) => self.literal(Value::Number(value), token.position),
            TokenKind::String(value) => self.literal(Value::String(value), token.position),
            TokenKind::Name(name) => Expr::Name {
                name,
                position: token.position,
            },
            TokenKind::Minus | TokenKind::Not | TokenKind::Hash => {
                let op = match token.kind {
                    TokenKind::Minus => UnaryOp::Negate,
                    TokenKind::Not => UnaryOp::Not,
                    TokenKind::Hash => UnaryOp::Length,
                    _ => unreachable!(),
                };
                let expression = self.expression(8)?;
                Expr::Unary {
                    op,
                    expression: Box::new(expression),
                    position: token.position,
                }
            }
            TokenKind::LeftParen => {
                let expression = self.expression(0)?;
                if self.current().kind != TokenKind::RightParen {
                    return Err(self.error("expected ')' to close grouped expression"));
                }
                self.advance();
                Expr::Group {
                    expression: Box::new(expression),
                    position: token.position,
                }
            }
            TokenKind::Unsupported(spelling) => {
                return Err(ParseError {
                    position: token.position,
                    message: format!("{spelling} cannot start an expression"),
                });
            }
            _ => {
                return Err(ParseError {
                    position: token.position,
                    message: "expected an expression".to_owned(),
                });
            }
        };
        // ANCHOR_END: pratt_prefix

        // ANCHOR: pratt_infix
        while let Some((op, left_power, right_power)) = infix_binding_power(&self.current().kind) {
            if left_power < minimum_binding_power {
                break;
            }
            let position = self.advance().position;
            let right = self.expression(right_power)?;
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                position,
            };
        }
        // ANCHOR_END: pratt_infix
        Ok(left)
    }
    // ANCHOR_END: pratt_parser

    fn literal(&self, value: Value, position: Position) -> Expr {
        Expr::Literal { value, position }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn advance(&mut self) -> &Token {
        let index = self.cursor;
        if self.tokens[index].kind != TokenKind::Eof {
            self.cursor += 1;
        }
        &self.tokens[index]
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            position: self.current().position,
            message: message.into(),
        }
    }
}

fn infix_binding_power(kind: &TokenKind) -> Option<(BinaryOp, u8, u8)> {
    let value = match kind {
        TokenKind::Or => (BinaryOp::Or, 1, 2),
        TokenKind::And => (BinaryOp::And, 2, 3),
        TokenKind::EqualEqual => (BinaryOp::Equal, 3, 4),
        TokenKind::NotEqual => (BinaryOp::NotEqual, 3, 4),
        TokenKind::Less => (BinaryOp::Less, 3, 4),
        TokenKind::LessEqual => (BinaryOp::LessEqual, 3, 4),
        TokenKind::Greater => (BinaryOp::Greater, 3, 4),
        TokenKind::GreaterEqual => (BinaryOp::GreaterEqual, 3, 4),
        TokenKind::DotDot => (BinaryOp::Concat, 5, 5),
        TokenKind::Plus => (BinaryOp::Add, 6, 7),
        TokenKind::Minus => (BinaryOp::Subtract, 6, 7),
        TokenKind::Star => (BinaryOp::Multiply, 7, 8),
        TokenKind::Slash => (BinaryOp::Divide, 7, 8),
        TokenKind::Percent => (BinaryOp::Modulo, 7, 8),
        TokenKind::Caret => (BinaryOp::Power, 9, 9),
        _ => return None,
    };
    Some(value)
}

struct Lexer<'source> {
    source: &'source [u8],
    cursor: usize,
    line: usize,
    column: usize,
}

impl<'source> Lexer<'source> {
    fn new(source: &'source [u8]) -> Self {
        Self {
            source,
            cursor: 0,
            line: 1,
            column: 1,
        }
    }

    fn tokenize(mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_trivia()?;
            let position = self.position();
            let kind = self.token()?;
            let finished = kind == TokenKind::Eof;
            tokens.push(Token { kind, position });
            if finished {
                return Ok(tokens);
            }
        }
    }

    fn token(&mut self) -> Result<TokenKind, ParseError> {
        let Some(byte) = self.current() else {
            return Ok(TokenKind::Eof);
        };
        if byte.is_ascii_alphabetic() || byte == b'_' {
            return Ok(self.name());
        }
        if byte.is_ascii_digit()
            || (byte == b'.' && self.peek(1).is_some_and(|next| next.is_ascii_digit()))
        {
            return self.number();
        }
        match byte {
            b'\'' | b'"' => self.short_string(),
            b'[' if self.long_level(self.cursor).is_some() => {
                let value = self.long_string()?;
                Ok(TokenKind::String(ByteString::new(value)))
            }
            b'+' => Ok(self.one(TokenKind::Plus)),
            b'-' => Ok(self.one(TokenKind::Minus)),
            b'*' => Ok(self.one(TokenKind::Star)),
            b'/' => Ok(self.one(TokenKind::Slash)),
            b'%' => Ok(self.one(TokenKind::Percent)),
            b'^' => Ok(self.one(TokenKind::Caret)),
            b'#' => Ok(self.one(TokenKind::Hash)),
            b'(' => Ok(self.one(TokenKind::LeftParen)),
            b')' => Ok(self.one(TokenKind::RightParen)),
            b'=' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::EqualEqual)),
            b'~' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::NotEqual)),
            b'<' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::LessEqual)),
            b'<' => Ok(self.one(TokenKind::Less)),
            b'>' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::GreaterEqual)),
            b'>' => Ok(self.one(TokenKind::Greater)),
            b'.' if self.peek(1) == Some(b'.') => Ok(self.two(TokenKind::DotDot)),
            _ => {
                self.bump();
                Ok(TokenKind::Unsupported(ByteString::new([byte])))
            }
        }
    }

    fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            while self
                .current()
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                self.bump();
            }
            if self.current() != Some(b'-') || self.peek(1) != Some(b'-') {
                return Ok(());
            }
            self.advance_by(2);
            if self.current() == Some(b'[') && self.long_level(self.cursor).is_some() {
                self.long_string()?;
            } else {
                while self
                    .current()
                    .is_some_and(|byte| byte != b'\n' && byte != b'\r')
                {
                    self.bump();
                }
            }
        }
    }

    fn name(&mut self) -> TokenKind {
        let start = self.cursor;
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.bump();
        }
        let name = &self.source[start..self.cursor];
        match name {
            b"nil" => TokenKind::Nil,
            b"false" => TokenKind::False,
            b"true" => TokenKind::True,
            b"not" => TokenKind::Not,
            b"and" => TokenKind::And,
            b"or" => TokenKind::Or,
            b"break" | b"do" | b"else" | b"elseif" | b"end" | b"for" | b"function" | b"if"
            | b"in" | b"local" | b"repeat" | b"return" | b"then" | b"until" | b"while" => {
                TokenKind::Unsupported(ByteString::new(name))
            }
            _ => TokenKind::Name(ByteString::new(name)),
        }
    }

    fn number(&mut self) -> Result<TokenKind, ParseError> {
        let start = self.cursor;
        if self.current() == Some(b'0') && matches!(self.peek(1), Some(b'x' | b'X')) {
            self.advance_by(2);
            let digits = self.cursor;
            while self.current().is_some_and(|byte| byte.is_ascii_hexdigit()) {
                self.bump();
            }
            if digits == self.cursor {
                return Err(self.error("hexadecimal number needs a digit"));
            }
            let text = std::str::from_utf8(&self.source[digits..self.cursor]).unwrap();
            let value = u64::from_str_radix(text, 16)
                .map_err(|_| self.error("hexadecimal number is too large"))?;
            return Ok(TokenKind::Number(value as f64));
        }
        while self.current().is_some_and(|byte| byte.is_ascii_digit()) {
            self.bump();
        }
        if self.current() == Some(b'.') && self.peek(1) != Some(b'.') {
            self.bump();
            while self.current().is_some_and(|byte| byte.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.current(), Some(b'e' | b'E')) {
            self.bump();
            if matches!(self.current(), Some(b'+' | b'-')) {
                self.bump();
            }
            let exponent = self.cursor;
            while self.current().is_some_and(|byte| byte.is_ascii_digit()) {
                self.bump();
            }
            if exponent == self.cursor {
                return Err(self.error("number exponent needs a digit"));
            }
        }
        let text = std::str::from_utf8(&self.source[start..self.cursor]).unwrap();
        text.parse::<f64>()
            .map(TokenKind::Number)
            .map_err(|_| self.error("invalid number"))
    }

    fn short_string(&mut self) -> Result<TokenKind, ParseError> {
        let quote = self.current().unwrap();
        self.bump();
        let mut value = Vec::new();
        loop {
            let Some(byte) = self.current() else {
                return Err(self.error("unterminated short string"));
            };
            if byte == quote {
                self.bump();
                return Ok(TokenKind::String(ByteString::new(value)));
            }
            if matches!(byte, b'\r' | b'\n') {
                return Err(self.error("newline in short string"));
            }
            if byte != b'\\' {
                value.push(byte);
                self.bump();
                continue;
            }
            self.bump();
            let escaped = self
                .current()
                .ok_or_else(|| self.error("unterminated string escape"))?;
            match escaped {
                digit if digit.is_ascii_digit() => {
                    let mut decimal = 0_u16;
                    for _ in 0..3 {
                        let Some(digit) = self.current().filter(|value| value.is_ascii_digit())
                        else {
                            break;
                        };
                        decimal = decimal * 10 + u16::from(digit - b'0');
                        self.bump();
                    }
                    if decimal > 255 {
                        return Err(self.error("decimal escape is greater than 255"));
                    }
                    value.push(decimal as u8);
                }
                other => {
                    let decoded = match other {
                        b'a' => 7,
                        b'b' => 8,
                        b'f' => 12,
                        b'n' => b'\n',
                        b'r' => b'\r',
                        b't' => b'\t',
                        b'v' => 11,
                        _ => other,
                    };
                    value.push(decoded);
                    self.bump();
                }
            }
        }
    }

    fn long_string(&mut self) -> Result<Box<[u8]>, ParseError> {
        let level = self.long_level(self.cursor).unwrap();
        self.advance_by(level + 2);
        if self.current() == Some(b'\r') {
            self.bump();
        }
        if self.current() == Some(b'\n') {
            self.bump();
        }
        let start = self.cursor;
        loop {
            let Some(byte) = self.current() else {
                return Err(self.error("unterminated long string or comment"));
            };
            if byte == b']'
                && (0..level).all(|index| self.peek(index + 1) == Some(b'='))
                && self.peek(level + 1) == Some(b']')
            {
                let value = self.source[start..self.cursor].to_vec().into_boxed_slice();
                self.advance_by(level + 2);
                return Ok(value);
            }
            self.bump();
        }
    }

    fn long_level(&self, start: usize) -> Option<usize> {
        if self.source.get(start) != Some(&b'[') {
            return None;
        }
        let mut level = 0;
        while self.source.get(start + level + 1) == Some(&b'=') {
            level += 1;
        }
        (self.source.get(start + level + 1) == Some(&b'[')).then_some(level)
    }

    fn one(&mut self, kind: TokenKind) -> TokenKind {
        self.bump();
        kind
    }

    fn two(&mut self, kind: TokenKind) -> TokenKind {
        self.advance_by(2);
        kind
    }

    fn current(&self) -> Option<u8> {
        self.source.get(self.cursor).copied()
    }

    fn peek(&self, distance: usize) -> Option<u8> {
        self.source.get(self.cursor + distance).copied()
    }

    fn bump(&mut self) {
        if let Some(byte) = self.current() {
            self.cursor += 1;
            if byte == b'\n' {
                self.line += 1;
                self.column = 1;
            } else {
                self.column += 1;
            }
        }
    }

    fn advance_by(&mut self, amount: usize) {
        for _ in 0..amount {
            self.bump();
        }
    }

    fn position(&self) -> Position {
        Position {
            offset: self.cursor,
            line: self.line,
            column: self.column,
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            position: self.position(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate, parse, BinaryOp, Expr, UnaryOp, Value};

    #[test]
    fn multiplication_binds_tighter_than_addition() {
        let expression = parse(b"1 + 2 * 3").unwrap();
        let Expr::Binary { op, right, .. } = expression else {
            panic!("expected binary expression");
        };
        assert_eq!(op, BinaryOp::Add);
        assert!(matches!(
            *right,
            Expr::Binary {
                op: BinaryOp::Multiply,
                ..
            }
        ));
    }

    #[test]
    fn power_and_concat_are_right_associative() {
        let power = parse(b"2 ^ 3 ^ 2").unwrap();
        let Expr::Binary { op, right, .. } = power else {
            panic!("expected binary expression");
        };
        assert_eq!(op, BinaryOp::Power);
        assert!(matches!(
            *right,
            Expr::Binary {
                op: BinaryOp::Power,
                ..
            }
        ));

        let concat = parse(b"'a' .. 'b' .. 'c'").unwrap();
        let Expr::Binary { op, right, .. } = concat else {
            panic!("expected binary expression");
        };
        assert_eq!(op, BinaryOp::Concat);
        assert!(matches!(
            *right,
            Expr::Binary {
                op: BinaryOp::Concat,
                ..
            }
        ));
    }

    #[test]
    fn power_binds_tighter_than_unary_minus() {
        let expression = parse(b"-2 ^ 2").unwrap();
        assert!(matches!(
            &expression,
            Expr::Unary {
                op: UnaryOp::Negate,
                expression,
                ..
            } if matches!(**expression, Expr::Binary { op: BinaryOp::Power, .. })
        ));
        assert_eq!(evaluate(&expression).unwrap(), Value::Number(-4.0));
    }

    #[test]
    fn malformed_input_has_a_source_location() {
        let error = parse(b"1 +\n)").unwrap_err();
        assert_eq!((error.position.line, error.position.column), (2, 1));
        assert!(error.message.contains("expected an expression"));
    }
}
