use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
        }
    }

    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Boolean(false))
    }

    fn into_concat_bytes(self) -> Option<Vec<u8>> {
        match self {
            Self::String(value) => Some(value.as_bytes().to_vec()),
            Self::Number(value) => Some(format!("{value}").into_bytes()),
            Self::Nil | Self::Boolean(_) => None,
        }
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

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Literal(Value, Position),
    Name(ByteString, Position),
    Unary(UnaryOp, Box<Expr>, Position),
    Binary(Box<Expr>, BinaryOp, Box<Expr>, Position),
    Group(Box<Expr>, Position),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub position: Position,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.position.line, self.position.column, self.message
        )
    }
}

impl std::error::Error for Error {}

pub fn parse(source: &[u8]) -> Result<Expr, Error> {
    Parser::new(Lexer::new(source).tokenize()?).parse()
}

pub fn execute(source: &[u8]) -> Result<Value, Error> {
    evaluate(&parse(source)?)
}

// ANCHOR: evaluator
pub fn evaluate(expression: &Expr) -> Result<Value, Error> {
    match expression {
        Expr::Literal(value, _) => Ok(value.clone()),
        Expr::Group(expression, _) => evaluate(expression),
        Expr::Name(name, position) => Err(runtime_error(
            *position,
            format!("undefined name '{name}' (variables arrive in v0.6)"),
        )),
        Expr::Unary(op, expression, position) => {
            let value = evaluate(expression)?;
            match op {
                UnaryOp::Not => Ok(Value::Boolean(!value.is_truthy())),
                UnaryOp::Negate => match value {
                    Value::Number(value) => Ok(Value::Number(-value)),
                    other => Err(type_error(*position, "negate", &other, "number")),
                },
                UnaryOp::Length => match value {
                    Value::String(value) => Ok(Value::Number(value.as_bytes().len() as f64)),
                    other => Err(type_error(*position, "get length of", &other, "string")),
                },
            }
        }
        Expr::Binary(left, BinaryOp::And, right, _) => {
            let left = evaluate(left)?;
            if left.is_truthy() {
                evaluate(right)
            } else {
                Ok(left)
            }
        }
        Expr::Binary(left, BinaryOp::Or, right, _) => {
            let left = evaluate(left)?;
            if left.is_truthy() {
                Ok(left)
            } else {
                evaluate(right)
            }
        }
        Expr::Binary(left, op, right, position) => {
            let left = evaluate(left)?;
            let right = evaluate(right)?;
            evaluate_eager_binary(left, *op, right, *position)
        }
    }
}
// ANCHOR_END: evaluator

// ANCHOR: eager_operators
fn evaluate_eager_binary(
    left: Value,
    op: BinaryOp,
    right: Value,
    position: Position,
) -> Result<Value, Error> {
    match op {
        // ANCHOR: compare_operators
        BinaryOp::Equal | BinaryOp::NotEqual => {
            let equal = left == right;
            Ok(Value::Boolean(if op == BinaryOp::Equal {
                equal
            } else {
                !equal
            }))
        }
        BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
            let ordering = compare(&left, &right).ok_or_else(|| {
                runtime_error(
                    position,
                    format!(
                        "cannot compare {} with {}",
                        left.type_name(),
                        right.type_name()
                    ),
                )
            })?;
            let result = match op {
                BinaryOp::Less => ordering == Ordering::Less,
                BinaryOp::LessEqual => ordering != Ordering::Greater,
                BinaryOp::Greater => ordering == Ordering::Greater,
                BinaryOp::GreaterEqual => ordering != Ordering::Less,
                _ => unreachable!(),
            };
            Ok(Value::Boolean(result))
        }
        // ANCHOR_END: compare_operators
        // ANCHOR: concat_operator
        BinaryOp::Concat => {
            let left_type = left.type_name();
            let right_type = right.type_name();
            let mut bytes = left.into_concat_bytes().ok_or_else(|| {
                runtime_error(position, format!("cannot concatenate {left_type}"))
            })?;
            let right = right.into_concat_bytes().ok_or_else(|| {
                runtime_error(position, format!("cannot concatenate {right_type}"))
            })?;
            bytes.extend(right);
            Ok(Value::String(ByteString::new(bytes)))
        }
        // ANCHOR_END: concat_operator
        // ANCHOR: arithmetic_operators
        BinaryOp::Add
        | BinaryOp::Subtract
        | BinaryOp::Multiply
        | BinaryOp::Divide
        | BinaryOp::Modulo
        | BinaryOp::Power => {
            let (Value::Number(left), Value::Number(right)) = (&left, &right) else {
                return Err(runtime_error(
                    position,
                    format!(
                        "arithmetic needs numbers, got {} and {}",
                        left.type_name(),
                        right.type_name()
                    ),
                ));
            };
            let result = match op {
                BinaryOp::Add => left + right,
                BinaryOp::Subtract => left - right,
                BinaryOp::Multiply => left * right,
                BinaryOp::Divide => left / right,
                BinaryOp::Modulo => left - (left / right).floor() * right,
                BinaryOp::Power => left.powf(*right),
                _ => unreachable!(),
            };
            Ok(Value::Number(result))
        }
        // ANCHOR_END: arithmetic_operators
        BinaryOp::And | BinaryOp::Or => unreachable!("short-circuit operators are handled first"),
    }
}
// ANCHOR_END: eager_operators

fn compare(left: &Value, right: &Value) -> Option<Ordering> {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left.partial_cmp(right),
        (Value::String(left), Value::String(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn type_error(position: Position, action: &str, value: &Value, expected: &str) -> Error {
    runtime_error(
        position,
        format!("cannot {action} {}, expected {expected}", value.type_name()),
    )
}

fn runtime_error(position: Position, message: impl Into<String>) -> Error {
    Error {
        position,
        message: message.into(),
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

#[derive(Clone, Debug)]
struct Token {
    kind: TokenKind,
    position: Position,
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn parse(mut self) -> Result<Expr, Error> {
        let expression = self.expression(0)?;
        if self.current().kind != TokenKind::Eof {
            return Err(self.error("expected end of input after expression"));
        }
        Ok(expression)
    }

    fn expression(&mut self, minimum_power: u8) -> Result<Expr, Error> {
        let token = self.advance().clone();
        let mut left = match token.kind {
            TokenKind::Nil => Expr::Literal(Value::Nil, token.position),
            TokenKind::False => Expr::Literal(Value::Boolean(false), token.position),
            TokenKind::True => Expr::Literal(Value::Boolean(true), token.position),
            TokenKind::Number(value) => Expr::Literal(Value::Number(value), token.position),
            TokenKind::String(value) => Expr::Literal(Value::String(value), token.position),
            TokenKind::Name(name) => Expr::Name(name, token.position),
            TokenKind::Minus | TokenKind::Not | TokenKind::Hash => {
                let op = match token.kind {
                    TokenKind::Minus => UnaryOp::Negate,
                    TokenKind::Not => UnaryOp::Not,
                    TokenKind::Hash => UnaryOp::Length,
                    _ => unreachable!(),
                };
                Expr::Unary(op, Box::new(self.expression(8)?), token.position)
            }
            TokenKind::LeftParen => {
                let expression = self.expression(0)?;
                if self.current().kind != TokenKind::RightParen {
                    return Err(self.error("expected ')' to close grouped expression"));
                }
                self.advance();
                Expr::Group(Box::new(expression), token.position)
            }
            TokenKind::Unsupported(spelling) => {
                return Err(runtime_error(
                    token.position,
                    format!("{spelling} cannot start an expression"),
                ));
            }
            _ => return Err(runtime_error(token.position, "expected an expression")),
        };

        while let Some((op, left_power, right_power)) = binding_power(&self.current().kind) {
            if left_power < minimum_power {
                break;
            }
            let position = self.advance().position;
            let right = self.expression(right_power)?;
            left = Expr::Binary(Box::new(left), op, Box::new(right), position);
        }
        Ok(left)
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

    fn error(&self, message: impl Into<String>) -> Error {
        runtime_error(self.current().position, message)
    }
}

fn binding_power(kind: &TokenKind) -> Option<(BinaryOp, u8, u8)> {
    Some(match kind {
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
    })
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

    fn tokenize(mut self) -> Result<Vec<Token>, Error> {
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

    fn token(&mut self) -> Result<TokenKind, Error> {
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

    fn skip_trivia(&mut self) -> Result<(), Error> {
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

    fn number(&mut self) -> Result<TokenKind, Error> {
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

    fn short_string(&mut self) -> Result<TokenKind, Error> {
        let quote = self.current().unwrap();
        self.bump();
        let mut bytes = Vec::new();
        loop {
            let Some(byte) = self.current() else {
                return Err(self.error("unterminated short string"));
            };
            if byte == quote {
                self.bump();
                return Ok(TokenKind::String(ByteString::new(bytes)));
            }
            if matches!(byte, b'\r' | b'\n') {
                return Err(self.error("newline in short string"));
            }
            if byte != b'\\' {
                bytes.push(byte);
                self.bump();
                continue;
            }
            self.bump();
            let escaped = self
                .current()
                .ok_or_else(|| self.error("unterminated string escape"))?;
            if escaped.is_ascii_digit() {
                let mut decimal = 0_u16;
                for _ in 0..3 {
                    let Some(digit) = self.current().filter(|value| value.is_ascii_digit()) else {
                        break;
                    };
                    decimal = decimal * 10 + u16::from(digit - b'0');
                    self.bump();
                }
                if decimal > 255 {
                    return Err(self.error("decimal escape is greater than 255"));
                }
                bytes.push(decimal as u8);
            } else {
                bytes.push(match escaped {
                    b'a' => 7,
                    b'b' => 8,
                    b'f' => 12,
                    b'n' => b'\n',
                    b'r' => b'\r',
                    b't' => b'\t',
                    b'v' => 11,
                    _ => escaped,
                });
                self.bump();
            }
        }
    }

    fn long_string(&mut self) -> Result<Box<[u8]>, Error> {
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

    fn error(&self, message: impl Into<String>) -> Error {
        runtime_error(self.position(), message)
    }
}

#[cfg(test)]
mod tests {
    use super::{execute, ByteString, Value};

    fn value(source: &str) -> Value {
        execute(source.as_bytes()).unwrap()
    }

    #[test]
    fn only_nil_and_false_are_falsey() {
        assert_eq!(value("not nil"), Value::Boolean(true));
        assert_eq!(value("not false"), Value::Boolean(true));
        assert_eq!(value("not 0"), Value::Boolean(false));
        assert_eq!(value("not ''"), Value::Boolean(false));
    }

    #[test]
    fn and_or_return_values_and_short_circuit() {
        assert_eq!(
            value("nil or 'fallback'"),
            Value::String(ByteString::new(&b"fallback"[..]))
        );
        assert_eq!(value("'left' and 42"), Value::Number(42.0));
        assert_eq!(value("false and missing"), Value::Boolean(false));
        assert_eq!(value("true or missing"), Value::Boolean(true));
    }

    #[test]
    fn arithmetic_uses_lua_modulo_and_precedence() {
        assert_eq!(value("-3 % 2"), Value::Number(1.0));
        assert_eq!(value("-2 ^ 2"), Value::Number(-4.0));
        assert_eq!(value("2 ^ -2"), Value::Number(0.25));
    }

    #[test]
    fn strings_are_bytes_during_length_concat_and_compare() {
        assert_eq!(value("#'A\\000\\255'"), Value::Number(3.0));
        assert_eq!(
            value("'answer=' .. 42"),
            Value::String(ByteString::new(&b"answer=42"[..]))
        );
        assert_eq!(value("'a' < 'b'"), Value::Boolean(true));
    }

    #[test]
    fn equality_does_not_coerce_types() {
        assert_eq!(value("1 == '1'"), Value::Boolean(false));
        assert_eq!(value("nil ~= false"), Value::Boolean(true));
    }

    #[test]
    fn type_errors_include_the_operator_position() {
        let error = execute(b"1 +\ntrue").unwrap_err();
        assert_eq!((error.position.line, error.position.column), (1, 3));
        assert!(error.message.contains("arithmetic needs numbers"));
    }
}
