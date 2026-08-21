use lua_core::{ByteString, Instruction, Opcode, Proto, Value, MAX_SBX};
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileError {
    pub position: Position,
    pub message: String,
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}: {}",
            self.position.line, self.position.column, self.message
        )
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnaryOp {
    Negate,
    Not,
    Length,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryOp {
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
enum Expr {
    Literal(Value, Position),
    Name(ByteString, Position),
    Unary(UnaryOp, Box<Expr>, Position),
    Binary(Box<Expr>, BinaryOp, Box<Expr>, Position),
    Group(Box<Expr>, Position),
}

#[derive(Clone, Debug, PartialEq)]
enum Stmt {
    Local {
        names: Vec<(ByteString, Position)>,
        values: Vec<Expr>,
        position: Position,
    },
    Assign {
        names: Vec<(ByteString, Position)>,
        values: Vec<Expr>,
        position: Position,
    },
    Do {
        body: Vec<Stmt>,
        position: Position,
    },
    Return {
        values: Vec<Expr>,
        position: Position,
    },
    Empty,
}

#[derive(Clone, Debug, PartialEq)]
struct Chunk {
    statements: Vec<Stmt>,
    position: Position,
}

pub fn compile(source: &[u8]) -> Result<Proto, CompileError> {
    let chunk = Parser::new(Lexer::new(source).tokenize()?).parse()?;
    Codegen::new().compile(&chunk)
}

struct Scope {
    base_register: usize,
    locals: HashMap<ByteString, u8>,
}

struct Codegen {
    constants: Vec<Value>,
    code: Vec<Instruction>,
    lines: Vec<usize>,
    next_register: usize,
    permanent_registers: usize,
    max_registers: usize,
    scopes: Vec<Scope>,
}

impl Codegen {
    fn new() -> Self {
        Self {
            constants: Vec::new(),
            code: Vec::new(),
            lines: Vec::new(),
            next_register: 0,
            permanent_registers: 0,
            max_registers: 0,
            scopes: vec![Scope {
                base_register: 0,
                locals: HashMap::new(),
            }],
        }
    }

    fn compile(mut self, chunk: &Chunk) -> Result<Proto, CompileError> {
        for statement in &chunk.statements {
            self.statement(statement)?;
        }
        let result = self.load_nil(chunk.position)?;
        self.emit(
            Instruction::abc(Opcode::Return, result, 0, 0),
            chunk.position.line,
        );
        Ok(Proto {
            constants: self.constants,
            code: self.code,
            lines: self.lines,
            max_stack_size: u8::try_from(self.max_registers.max(1)).map_err(|_| CompileError {
                position: chunk.position,
                message: "chunk needs more than 255 registers".to_owned(),
            })?,
        })
    }

    fn statement(&mut self, statement: &Stmt) -> Result<(), CompileError> {
        match statement {
            Stmt::Local {
                names,
                values,
                position,
            } => self.local_statement(names, values, *position),
            Stmt::Assign {
                names,
                values,
                position,
            } => self.assignment(names, values, *position),
            Stmt::Do { body, .. } => {
                self.enter_scope();
                for statement in body {
                    self.statement(statement)?;
                }
                self.leave_scope();
                Ok(())
            }
            Stmt::Return { values, position } => {
                self.reset_temporaries();
                let result = if let Some(value) = values.first() {
                    self.expression(value)?
                } else {
                    self.load_nil(*position)?
                };
                self.emit(
                    Instruction::abc(Opcode::Return, result, 0, 0),
                    position.line,
                );
                self.reset_temporaries();
                Ok(())
            }
            Stmt::Empty => Ok(()),
        }
    }

    // ANCHOR: bindings
    fn local_statement(
        &mut self,
        names: &[(ByteString, Position)],
        values: &[Expr],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let value_registers = self.value_registers(values)?;
        let base = self.permanent_registers;
        for (index, (name, name_position)) in names.iter().enumerate() {
            let target = self.register_at(base + index, *name_position)?;
            if let Some(source) = value_registers.get(index) {
                self.emit(
                    Instruction::abc(Opcode::Move, target, u16::from(*source), 0),
                    position.line,
                );
            } else {
                let nil = self.nil_constant(position)?;
                self.emit(Instruction::abx(Opcode::LoadK, target, nil), position.line);
            }
            self.scopes
                .last_mut()
                .expect("codegen always has a scope")
                .locals
                .insert(name.clone(), target);
        }
        self.permanent_registers = base + names.len();
        self.reset_temporaries();
        Ok(())
    }

    fn assignment(
        &mut self,
        names: &[(ByteString, Position)],
        values: &[Expr],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let mut value_registers = self.value_registers(values)?;
        while value_registers.len() < names.len() {
            value_registers.push(self.load_nil(position)?);
        }
        for ((name, name_position), source) in names.iter().zip(value_registers) {
            if let Some(target) = self.resolve_local(name) {
                self.emit(
                    Instruction::abc(Opcode::Move, target, u16::from(source), 0),
                    position.line,
                );
            } else {
                let constant = self.constant(Value::String(name.clone()), *name_position)?;
                self.emit(
                    Instruction::abx(Opcode::SetGlobal, source, constant),
                    position.line,
                );
            }
        }
        self.reset_temporaries();
        Ok(())
    }

    fn value_registers(&mut self, values: &[Expr]) -> Result<Vec<u8>, CompileError> {
        values.iter().map(|value| self.expression(value)).collect()
    }

    fn enter_scope(&mut self) {
        self.scopes.push(Scope {
            base_register: self.permanent_registers,
            locals: HashMap::new(),
        });
    }

    fn leave_scope(&mut self) {
        let scope = self.scopes.pop().expect("cannot leave the root scope");
        self.permanent_registers = scope.base_register;
        self.reset_temporaries();
    }

    fn resolve_local(&self, name: &ByteString) -> Option<u8> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.locals.get(name).copied())
    }
    // ANCHOR_END: bindings

    fn reset_temporaries(&mut self) {
        self.next_register = self.permanent_registers;
    }

    // ANCHOR: codegen
    fn expression(&mut self, expression: &Expr) -> Result<u8, CompileError> {
        match expression {
            Expr::Literal(value, position) => {
                let register = self.register(*position)?;
                let constant = self.constant(value.clone(), *position)?;
                self.emit(
                    Instruction::abx(Opcode::LoadK, register, constant),
                    position.line,
                );
                Ok(register)
            }
            Expr::Name(name, position) => {
                let register = self.register(*position)?;
                if let Some(local) = self.resolve_local(name) {
                    self.emit(
                        Instruction::abc(Opcode::Move, register, u16::from(local), 0),
                        position.line,
                    );
                } else {
                    let constant = self.constant(Value::String(name.clone()), *position)?;
                    self.emit(
                        Instruction::abx(Opcode::GetGlobal, register, constant),
                        position.line,
                    );
                }
                Ok(register)
            }
            Expr::Group(expression, _) => self.expression(expression),
            Expr::Unary(op, expression, position) => {
                let register = self.expression(expression)?;
                let opcode = match op {
                    UnaryOp::Negate => Opcode::Neg,
                    UnaryOp::Not => Opcode::Not,
                    UnaryOp::Length => Opcode::Len,
                };
                self.emit(
                    Instruction::abc(opcode, register, u16::from(register), 0),
                    position.line,
                );
                Ok(register)
            }
            Expr::Binary(left, BinaryOp::And, right, position)
            | Expr::Binary(left, BinaryOp::Or, right, position) => {
                let left_register = self.expression(left)?;
                let jump_opcode = if matches!(expression, Expr::Binary(_, BinaryOp::And, _, _)) {
                    Opcode::JumpIfFalse
                } else {
                    Opcode::JumpIfTrue
                };
                let jump = self.emit(
                    Instruction::asbx(jump_opcode, left_register, 0),
                    position.line,
                );
                let right_register = self.expression(right)?;
                self.emit(
                    Instruction::abc(Opcode::Move, left_register, u16::from(right_register), 0),
                    position.line,
                );
                self.patch_jump(jump, self.code.len(), *position)?;
                Ok(left_register)
            }
            Expr::Binary(left, op, right, position) => {
                let left_register = self.expression(left)?;
                let right_register = self.expression(right)?;
                let opcode = match op {
                    BinaryOp::Equal => Opcode::Eq,
                    BinaryOp::NotEqual => Opcode::Ne,
                    BinaryOp::Less => Opcode::Lt,
                    BinaryOp::LessEqual => Opcode::Le,
                    BinaryOp::Greater => Opcode::Gt,
                    BinaryOp::GreaterEqual => Opcode::Ge,
                    BinaryOp::Concat => Opcode::Concat,
                    BinaryOp::Add => Opcode::Add,
                    BinaryOp::Subtract => Opcode::Sub,
                    BinaryOp::Multiply => Opcode::Mul,
                    BinaryOp::Divide => Opcode::Div,
                    BinaryOp::Modulo => Opcode::Mod,
                    BinaryOp::Power => Opcode::Pow,
                    BinaryOp::And | BinaryOp::Or => unreachable!(),
                };
                self.emit(
                    Instruction::abc(
                        opcode,
                        left_register,
                        u16::from(left_register),
                        u16::from(right_register),
                    ),
                    position.line,
                );
                Ok(left_register)
            }
        }
    }
    // ANCHOR_END: codegen

    fn register(&mut self, position: Position) -> Result<u8, CompileError> {
        let register = self.register_at(self.next_register, position)?;
        self.next_register += 1;
        Ok(register)
    }

    fn register_at(&mut self, index: usize, position: Position) -> Result<u8, CompileError> {
        if index >= 255 {
            return Err(CompileError {
                position,
                message: "chunk needs more than 255 registers".to_owned(),
            });
        }
        self.max_registers = self.max_registers.max(index + 1);
        Ok(index as u8)
    }

    fn load_nil(&mut self, position: Position) -> Result<u8, CompileError> {
        let register = self.register(position)?;
        let constant = self.nil_constant(position)?;
        self.emit(
            Instruction::abx(Opcode::LoadK, register, constant),
            position.line,
        );
        Ok(register)
    }

    fn nil_constant(&mut self, position: Position) -> Result<u32, CompileError> {
        self.constant(Value::Nil, position)
    }

    fn constant(&mut self, value: Value, position: Position) -> Result<u32, CompileError> {
        if let Some(index) = self
            .constants
            .iter()
            .position(|constant| *constant == value)
        {
            return Ok(index as u32);
        }
        let index = u32::try_from(self.constants.len()).map_err(|_| CompileError {
            position,
            message: "too many constants".to_owned(),
        })?;
        if index > 262_143 {
            return Err(CompileError {
                position,
                message: "too many constants for an ABx instruction".to_owned(),
            });
        }
        self.constants.push(value);
        Ok(index)
    }

    fn emit(&mut self, instruction: Instruction, line: usize) -> usize {
        let pc = self.code.len();
        self.code.push(instruction);
        self.lines.push(line);
        pc
    }

    fn patch_jump(
        &mut self,
        jump_pc: usize,
        target_pc: usize,
        position: Position,
    ) -> Result<(), CompileError> {
        let offset = target_pc as i64 - jump_pc as i64 - 1;
        if !(-i64::from(MAX_SBX)..=i64::from(MAX_SBX)).contains(&offset) {
            return Err(CompileError {
                position,
                message: "short-circuit jump is too far".to_owned(),
            });
        }
        let old = self.code[jump_pc];
        let opcode = old.opcode().expect("codegen emitted a valid opcode");
        self.code[jump_pc] = Instruction::asbx(opcode, old.a(), offset as i32);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TokenKind {
    Local,
    Do,
    End,
    Return,
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
    Equal,
    Comma,
    Semicolon,
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

    fn parse(mut self) -> Result<Chunk, CompileError> {
        let position = self.current().position;
        let statements = self.block(false)?;
        if self.current().kind != TokenKind::Eof {
            return Err(self.error("expected end of input"));
        }
        Ok(Chunk {
            statements,
            position,
        })
    }

    fn block(&mut self, stop_at_end: bool) -> Result<Vec<Stmt>, CompileError> {
        let mut statements = Vec::new();
        while self.current().kind != TokenKind::Eof
            && (!stop_at_end || self.current().kind != TokenKind::End)
        {
            let statement = self.statement()?;
            let is_return = matches!(statement, Stmt::Return { .. });
            statements.push(statement);
            if is_return {
                if self.current().kind == TokenKind::Semicolon {
                    self.advance();
                }
                let at_end = self.current().kind == TokenKind::Eof
                    || (stop_at_end && self.current().kind == TokenKind::End);
                if !at_end {
                    return Err(self.error("return must be the final statement in a block"));
                }
            }
        }
        Ok(statements)
    }

    fn statement(&mut self) -> Result<Stmt, CompileError> {
        match self.current().kind {
            TokenKind::Semicolon => {
                self.advance();
                Ok(Stmt::Empty)
            }
            TokenKind::Local => self.local_statement(),
            TokenKind::Do => self.do_statement(),
            TokenKind::Return => self.return_statement(),
            TokenKind::Name(_) => self.assignment(),
            _ => Err(self.error("expected local, assignment, do block or return")),
        }
    }

    fn local_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let names = self.name_list()?;
        let values = if self.current().kind == TokenKind::Equal {
            self.advance();
            self.expression_list()?
        } else {
            Vec::new()
        };
        Ok(Stmt::Local {
            names,
            values,
            position,
        })
    }

    fn assignment(&mut self) -> Result<Stmt, CompileError> {
        let position = self.current().position;
        let names = self.name_list()?;
        if self.current().kind != TokenKind::Equal {
            return Err(self.error("expected '=' after assignment targets"));
        }
        self.advance();
        let values = self.expression_list()?;
        Ok(Stmt::Assign {
            names,
            values,
            position,
        })
    }

    fn do_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let body = self.block(true)?;
        if self.current().kind != TokenKind::End {
            return Err(self.error("expected 'end' to close do block"));
        }
        self.advance();
        Ok(Stmt::Do { body, position })
    }

    fn return_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let values = if matches!(
            self.current().kind,
            TokenKind::Eof | TokenKind::End | TokenKind::Semicolon
        ) {
            Vec::new()
        } else {
            self.expression_list()?
        };
        Ok(Stmt::Return { values, position })
    }

    fn name_list(&mut self) -> Result<Vec<(ByteString, Position)>, CompileError> {
        let mut names = Vec::new();
        loop {
            let token = self.advance().clone();
            let TokenKind::Name(name) = token.kind else {
                return Err(CompileError {
                    position: token.position,
                    message: "expected a variable name".to_owned(),
                });
            };
            names.push((name, token.position));
            if self.current().kind != TokenKind::Comma {
                return Ok(names);
            }
            self.advance();
        }
    }

    fn expression_list(&mut self) -> Result<Vec<Expr>, CompileError> {
        let mut values = vec![self.expression(0)?];
        while self.current().kind == TokenKind::Comma {
            self.advance();
            values.push(self.expression(0)?);
        }
        Ok(values)
    }

    fn expression(&mut self, minimum_power: u8) -> Result<Expr, CompileError> {
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
                return Err(CompileError {
                    position: token.position,
                    message: format!("{spelling} cannot start an expression"),
                });
            }
            _ => {
                return Err(CompileError {
                    position: token.position,
                    message: "expected an expression".to_owned(),
                });
            }
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

    fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError {
            position: self.current().position,
            message: message.into(),
        }
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

    fn tokenize(mut self) -> Result<Vec<Token>, CompileError> {
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

    fn token(&mut self) -> Result<TokenKind, CompileError> {
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
            b'=' => Ok(self.one(TokenKind::Equal)),
            b',' => Ok(self.one(TokenKind::Comma)),
            b';' => Ok(self.one(TokenKind::Semicolon)),
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

    fn skip_trivia(&mut self) -> Result<(), CompileError> {
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
            b"local" => TokenKind::Local,
            b"do" => TokenKind::Do,
            b"end" => TokenKind::End,
            b"return" => TokenKind::Return,
            b"nil" => TokenKind::Nil,
            b"false" => TokenKind::False,
            b"true" => TokenKind::True,
            b"not" => TokenKind::Not,
            b"and" => TokenKind::And,
            b"or" => TokenKind::Or,
            b"break" | b"else" | b"elseif" | b"for" | b"function" | b"if" | b"in" | b"repeat"
            | b"then" | b"until" | b"while" => TokenKind::Unsupported(ByteString::new(name)),
            _ => TokenKind::Name(ByteString::new(name)),
        }
    }

    fn number(&mut self) -> Result<TokenKind, CompileError> {
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

    fn short_string(&mut self) -> Result<TokenKind, CompileError> {
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

    fn long_string(&mut self) -> Result<Box<[u8]>, CompileError> {
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

    fn error(&self, message: impl Into<String>) -> CompileError {
        CompileError {
            position: self.position(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compile;
    use lua_core::Opcode;

    #[test]
    fn precedence_reaches_codegen_in_the_right_shape() {
        let proto = compile(b"return 1 + 2 * 3").unwrap();
        let opcodes: Vec<_> = proto
            .code
            .iter()
            .map(|instruction| instruction.opcode().unwrap())
            .collect();
        let multiply = opcodes
            .iter()
            .position(|opcode| *opcode == Opcode::Mul)
            .unwrap();
        let add = opcodes
            .iter()
            .position(|opcode| *opcode == Opcode::Add)
            .unwrap();
        assert!(multiply < add);
    }

    #[test]
    fn proto_tables_have_matching_shapes() {
        let proto = compile(b"local answer = false or 42\nreturn answer").unwrap();
        assert_eq!(proto.validate_shape(), Ok(()));
        assert!(proto.max_stack_size >= 2);
    }

    #[test]
    fn register_limit_is_a_structured_compile_error() {
        let names = (0..256)
            .map(|index| format!("v{index}"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("local {names}\nreturn nil");
        let error = compile(source.as_bytes()).unwrap_err();
        assert!(error.message.contains("more than 255 registers"));
    }
}
