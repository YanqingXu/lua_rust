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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub position: Position,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    And,
    Break,
    Do,
    Else,
    ElseIf,
    End,
    False,
    For,
    Function,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,
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
    Equal,
    EqualEqual,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Semicolon,
    Colon,
    Comma,
    Dot,
    DotDot,
    DotDotDot,
    Eof,
}

impl TokenKind {
    pub fn label(&self) -> String {
        match self {
            Self::Name(value) => format!("name({value})"),
            Self::Number(value) => format!("number({value})"),
            Self::String(value) => format!("string(\"{value}\")"),
            Self::Eof => "<eof>".to_owned(),
            other => format!("{other:?}").to_ascii_lowercase(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexError {
    pub position: Position,
    pub message: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.position.line, self.position.column, self.message
        )
    }
}

impl std::error::Error for LexError {}

pub struct Lexer<'source> {
    source: &'source [u8],
    cursor: usize,
    line: usize,
    column: usize,
}

impl<'source> Lexer<'source> {
    pub fn new(source: &'source [u8]) -> Self {
        Self {
            source,
            cursor: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();
        loop {
            let token = self.next_token()?;
            let finished = token.kind == TokenKind::Eof;
            tokens.push(token);
            if finished {
                return Ok(tokens);
            }
        }
    }

    // ANCHOR: next_token
    pub fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_trivia()?;
        let position = self.position();
        let Some(byte) = self.current() else {
            return Ok(Token {
                kind: TokenKind::Eof,
                position,
            });
        };

        let kind = if byte.is_ascii_alphabetic() || byte == b'_' {
            self.lex_name_or_keyword()
        } else if byte.is_ascii_digit()
            || (byte == b'.' && self.peek(1).is_some_and(|next| next.is_ascii_digit()))
        {
            self.lex_number()?
        } else {
            match byte {
                b'\'' | b'"' => self.lex_short_string()?,
                b'[' if self.long_bracket_level(self.cursor).is_some() => {
                    TokenKind::String(ByteString::new(self.lex_long_string()?))
                }
                b'+' => self.one(TokenKind::Plus),
                b'-' => self.one(TokenKind::Minus),
                b'*' => self.one(TokenKind::Star),
                b'/' => self.one(TokenKind::Slash),
                b'%' => self.one(TokenKind::Percent),
                b'^' => self.one(TokenKind::Caret),
                b'#' => self.one(TokenKind::Hash),
                b'=' if self.peek(1) == Some(b'=') => self.two(TokenKind::EqualEqual),
                b'=' => self.one(TokenKind::Equal),
                b'~' if self.peek(1) == Some(b'=') => self.two(TokenKind::NotEqual),
                b'<' if self.peek(1) == Some(b'=') => self.two(TokenKind::LessEqual),
                b'<' => self.one(TokenKind::Less),
                b'>' if self.peek(1) == Some(b'=') => self.two(TokenKind::GreaterEqual),
                b'>' => self.one(TokenKind::Greater),
                b'(' => self.one(TokenKind::LeftParen),
                b')' => self.one(TokenKind::RightParen),
                b'{' => self.one(TokenKind::LeftBrace),
                b'}' => self.one(TokenKind::RightBrace),
                b'[' => self.one(TokenKind::LeftBracket),
                b']' => self.one(TokenKind::RightBracket),
                b';' => self.one(TokenKind::Semicolon),
                b':' => self.one(TokenKind::Colon),
                b',' => self.one(TokenKind::Comma),
                b'.' if self.peek(1) == Some(b'.') && self.peek(2) == Some(b'.') => {
                    self.advance(3);
                    TokenKind::DotDotDot
                }
                b'.' if self.peek(1) == Some(b'.') => self.two(TokenKind::DotDot),
                b'.' => self.one(TokenKind::Dot),
                _ => {
                    return Err(self.error(format!("unexpected byte 0x{byte:02x}")));
                }
            }
        };

        Ok(Token { kind, position })
    }
    // ANCHOR_END: next_token

    fn skip_trivia(&mut self) -> Result<(), LexError> {
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
            self.advance(2);
            if self.current() == Some(b'[') && self.long_bracket_level(self.cursor).is_some() {
                self.lex_long_string()?;
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

    fn lex_name_or_keyword(&mut self) -> TokenKind {
        let start = self.cursor;
        while self
            .current()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            self.bump();
        }
        let name = &self.source[start..self.cursor];
        match name {
            b"and" => TokenKind::And,
            b"break" => TokenKind::Break,
            b"do" => TokenKind::Do,
            b"else" => TokenKind::Else,
            b"elseif" => TokenKind::ElseIf,
            b"end" => TokenKind::End,
            b"false" => TokenKind::False,
            b"for" => TokenKind::For,
            b"function" => TokenKind::Function,
            b"if" => TokenKind::If,
            b"in" => TokenKind::In,
            b"local" => TokenKind::Local,
            b"nil" => TokenKind::Nil,
            b"not" => TokenKind::Not,
            b"or" => TokenKind::Or,
            b"repeat" => TokenKind::Repeat,
            b"return" => TokenKind::Return,
            b"then" => TokenKind::Then,
            b"true" => TokenKind::True,
            b"until" => TokenKind::Until,
            b"while" => TokenKind::While,
            _ => TokenKind::Name(ByteString::new(name)),
        }
    }

    fn lex_number(&mut self) -> Result<TokenKind, LexError> {
        let start = self.cursor;
        if self.current() == Some(b'0') && matches!(self.peek(1), Some(b'x' | b'X')) {
            self.advance(2);
            let digits_start = self.cursor;
            while self.current().is_some_and(|byte| byte.is_ascii_hexdigit()) {
                self.bump();
            }
            if self.cursor == digits_start {
                return Err(self.error("hexadecimal number needs at least one digit"));
            }
            let text = std::str::from_utf8(&self.source[digits_start..self.cursor])
                .expect("hexadecimal digits are ASCII");
            let integer = u64::from_str_radix(text, 16)
                .map_err(|_| self.error("hexadecimal number is too large"))?;
            return Ok(TokenKind::Number(integer as f64));
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
            let exponent_start = self.cursor;
            while self.current().is_some_and(|byte| byte.is_ascii_digit()) {
                self.bump();
            }
            if self.cursor == exponent_start {
                return Err(self.error("number exponent needs at least one digit"));
            }
        }

        let text = std::str::from_utf8(&self.source[start..self.cursor])
            .expect("number spelling is ASCII");
        let value = text
            .parse::<f64>()
            .map_err(|_| self.error(format!("invalid number {text}")))?;
        Ok(TokenKind::Number(value))
    }

    fn lex_short_string(&mut self) -> Result<TokenKind, LexError> {
        let quote = self.current().expect("caller checked quote");
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
            if matches!(byte, b'\n' | b'\r') {
                return Err(self.error("newline in short string"));
            }
            if byte != b'\\' {
                value.push(byte);
                self.bump();
                continue;
            }

            self.bump();
            let Some(escaped) = self.current() else {
                return Err(self.error("unterminated string escape"));
            };
            match escaped {
                b'a' => {
                    value.push(7);
                    self.bump();
                }
                b'b' => {
                    value.push(8);
                    self.bump();
                }
                b'f' => {
                    value.push(12);
                    self.bump();
                }
                b'n' => {
                    value.push(b'\n');
                    self.bump();
                }
                b'r' => {
                    value.push(b'\r');
                    self.bump();
                }
                b't' => {
                    value.push(b'\t');
                    self.bump();
                }
                b'v' => {
                    value.push(11);
                    self.bump();
                }
                b'\n' => {
                    value.push(b'\n');
                    self.bump();
                }
                b'\r' => {
                    value.push(b'\n');
                    self.bump();
                    if self.current() == Some(b'\n') {
                        self.bump();
                    }
                }
                digit if digit.is_ascii_digit() => {
                    let mut decimal = 0_u16;
                    for _ in 0..3 {
                        let Some(digit) = self.current().filter(u8::is_ascii_digit) else {
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
                    value.push(other);
                    self.bump();
                }
            }
        }
    }

    // ANCHOR: long_string
    fn lex_long_string(&mut self) -> Result<Box<[u8]>, LexError> {
        let level = self
            .long_bracket_level(self.cursor)
            .expect("caller checked long bracket");
        self.advance(level + 2);
        if self.current() == Some(b'\r') {
            self.bump();
            if self.current() == Some(b'\n') {
                self.bump();
            }
        } else if self.current() == Some(b'\n') {
            self.bump();
        }
        let content_start = self.cursor;
        loop {
            let Some(byte) = self.current() else {
                return Err(self.error("unterminated long string or comment"));
            };
            if byte == b']' && self.long_bracket_close_matches(level) {
                let content = self.source[content_start..self.cursor].to_vec();
                self.advance(level + 2);
                return Ok(content.into_boxed_slice());
            }
            self.bump();
        }
    }
    // ANCHOR_END: long_string

    fn long_bracket_level(&self, start: usize) -> Option<usize> {
        if self.source.get(start) != Some(&b'[') {
            return None;
        }
        let mut level = 0;
        while self.source.get(start + level + 1) == Some(&b'=') {
            level += 1;
        }
        (self.source.get(start + level + 1) == Some(&b'[')).then_some(level)
    }

    fn long_bracket_close_matches(&self, level: usize) -> bool {
        if self.current() != Some(b']') {
            return false;
        }
        (0..level).all(|index| self.peek(index + 1) == Some(b'='))
            && self.peek(level + 1) == Some(b']')
    }

    fn one(&mut self, kind: TokenKind) -> TokenKind {
        self.bump();
        kind
    }

    fn two(&mut self, kind: TokenKind) -> TokenKind {
        self.advance(2);
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

    fn advance(&mut self, count: usize) {
        for _ in 0..count {
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

    fn error(&self, message: impl Into<String>) -> LexError {
        LexError {
            position: self.position(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteString, Lexer, Position, TokenKind};

    #[test]
    fn keywords_names_numbers_and_operators_are_distinct() {
        let kinds: Vec<_> = Lexer::new(b"local answer = 0x2a + .5..'x'")
            .tokenize()
            .unwrap()
            .into_iter()
            .map(|token| token.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Local,
                TokenKind::Name(ByteString::new(&b"answer"[..])),
                TokenKind::Equal,
                TokenKind::Number(42.0),
                TokenKind::Plus,
                TokenKind::Number(0.5),
                TokenKind::DotDot,
                TokenKind::String(ByteString::new(&b"x"[..])),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn comments_and_long_strings_update_positions() {
        let tokens = Lexer::new(b"-- heading\n[=[\nhello\n]=]")
            .tokenize()
            .unwrap();
        assert_eq!(
            tokens[0].position,
            Position {
                offset: 11,
                line: 2,
                column: 1
            }
        );
        assert_eq!(
            tokens[0].kind,
            TokenKind::String(ByteString::new(&b"hello\n"[..]))
        );
        assert_eq!(tokens[1].position.line, 4);
    }

    #[test]
    fn decimal_escapes_keep_arbitrary_bytes() {
        let tokens = Lexer::new(b"'A\\000\\255'").tokenize().unwrap();
        assert_eq!(
            tokens[0].kind,
            TokenKind::String(ByteString::new(&b"A\0\xff"[..]))
        );
    }

    #[test]
    fn malformed_input_reports_its_location() {
        let error = Lexer::new(b"\n[=[never closed").tokenize().unwrap_err();
        assert_eq!(error.position.line, 2);
        assert!(error.message.contains("unterminated long string"));
    }
}
