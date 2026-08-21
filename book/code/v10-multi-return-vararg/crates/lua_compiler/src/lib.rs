use lua_core::{ByteString, Instruction, Opcode, Proto, Value, MAX_SBX};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

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
    Table(Vec<TableField>, Position),
    Index(Box<Expr>, Box<Expr>, Position),
    Function {
        parameters: Vec<(ByteString, Position)>,
        is_vararg: bool,
        body: Vec<Stmt>,
        position: Position,
    },
    Vararg(Position),
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
        position: Position,
    },
    MethodCall {
        receiver: Box<Expr>,
        method: ByteString,
        arguments: Vec<Expr>,
        position: Position,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum TableField {
    List(Expr),
    Record(Expr, Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum LValue {
    Name(ByteString, Position),
    Index(Expr, Expr, Position),
}

#[derive(Clone, Debug, PartialEq)]
enum Stmt {
    Local {
        names: Vec<(ByteString, Position)>,
        values: Vec<Expr>,
        position: Position,
    },
    Assign {
        targets: Vec<LValue>,
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
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Vec<Stmt>,
        position: Position,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
        position: Position,
    },
    Repeat {
        body: Vec<Stmt>,
        condition: Expr,
        position: Position,
    },
    NumericFor {
        name: (ByteString, Position),
        initial: Expr,
        limit: Expr,
        step: Option<Expr>,
        body: Vec<Stmt>,
        position: Position,
    },
    GenericFor {
        names: Vec<(ByteString, Position)>,
        values: Vec<Expr>,
        body: Vec<Stmt>,
        position: Position,
    },
    Break(Position),
    Expression(Expr),
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

enum ResolvedTarget {
    Local(u8),
    Global(u32),
    Table { table: u8, key: u8 },
}

#[derive(Clone, Copy)]
enum CallMode {
    Results(Option<usize>),
    Tail,
}

struct Codegen {
    constants: Vec<Value>,
    code: Vec<Instruction>,
    lines: Vec<usize>,
    next_register: usize,
    permanent_registers: usize,
    max_registers: usize,
    scopes: Vec<Scope>,
    break_jumps: Vec<Vec<usize>>,
    protos: Vec<Rc<Proto>>,
    is_vararg: bool,
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
            break_jumps: Vec::new(),
            protos: Vec::new(),
            is_vararg: false,
        }
    }

    fn compile(mut self, chunk: &Chunk) -> Result<Proto, CompileError> {
        for statement in &chunk.statements {
            self.statement(statement)?;
        }
        self.emit(
            Instruction::abc(Opcode::Return, 0, 1, 0),
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
            protos: self.protos,
            num_params: 0,
            is_vararg: false,
            debug_name: "chunk".to_owned(),
        })
    }

    fn compile_function(
        mut self,
        parameters: &[(ByteString, Position)],
        is_vararg: bool,
        body: &[Stmt],
        position: Position,
    ) -> Result<Proto, CompileError> {
        let num_params = u8::try_from(parameters.len()).map_err(|_| CompileError {
            position,
            message: "function has more than 255 parameters".to_owned(),
        })?;
        self.is_vararg = is_vararg;
        for (index, (name, parameter_position)) in parameters.iter().enumerate() {
            let register = self.register_at(index, *parameter_position)?;
            self.scopes[0].locals.insert(name.clone(), register);
        }
        self.permanent_registers = parameters.len();
        self.reset_temporaries();
        for statement in body {
            self.statement(statement)?;
        }
        self.emit(Instruction::abc(Opcode::Return, 0, 1, 0), position.line);
        Ok(Proto {
            constants: self.constants,
            code: self.code,
            lines: self.lines,
            max_stack_size: u8::try_from(self.max_registers.max(1)).map_err(|_| CompileError {
                position,
                message: "function needs more than 255 registers".to_owned(),
            })?,
            protos: self.protos,
            num_params,
            is_vararg,
            debug_name: format!("function@{}", position.line),
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
                targets,
                values,
                position,
            } => self.assignment(targets, values, *position),
            Stmt::Do { body, .. } => {
                self.enter_scope();
                for statement in body {
                    self.statement(statement)?;
                }
                self.leave_scope();
                Ok(())
            }
            Stmt::Return { values, position } => self.return_statement(values, *position),
            Stmt::If {
                branches,
                else_body,
                position,
            } => self.if_statement(branches, else_body, *position),
            Stmt::While {
                condition,
                body,
                position,
            } => self.while_statement(condition, body, *position),
            Stmt::Repeat {
                body,
                condition,
                position,
            } => self.repeat_statement(body, condition, *position),
            Stmt::NumericFor {
                name,
                initial,
                limit,
                step,
                body,
                position,
            } => self.numeric_for(name, initial, limit, step.as_ref(), body, *position),
            Stmt::GenericFor {
                names,
                values,
                body,
                position,
            } => self.generic_for(names, values, body, *position),
            Stmt::Break(position) => {
                if self.break_jumps.is_empty() {
                    return Err(CompileError {
                        position: *position,
                        message: "break is only valid inside a loop".to_owned(),
                    });
                }
                let jump = self.emit(Instruction::asbx(Opcode::Jump, 0, 0), position.line);
                self.break_jumps.last_mut().unwrap().push(jump);
                Ok(())
            }
            Stmt::Expression(expression) => {
                self.reset_temporaries();
                self.expression(expression)?;
                self.reset_temporaries();
                Ok(())
            }
            Stmt::Empty => Ok(()),
        }
    }

    // ANCHOR: control_flow_codegen
    fn if_statement(
        &mut self,
        branches: &[(Expr, Vec<Stmt>)],
        else_body: &[Stmt],
        position: Position,
    ) -> Result<(), CompileError> {
        let mut end_jumps = Vec::new();
        for (condition, body) in branches {
            self.reset_temporaries();
            let condition = self.expression(condition)?;
            let next_branch = self.emit(
                Instruction::asbx(Opcode::JumpIfFalse, condition, 0),
                position.line,
            );
            self.reset_temporaries();
            self.enter_scope();
            for statement in body {
                self.statement(statement)?;
            }
            self.leave_scope();
            end_jumps.push(self.emit(Instruction::asbx(Opcode::Jump, 0, 0), position.line));
            self.patch_jump(next_branch, self.code.len(), position)?;
        }
        self.enter_scope();
        for statement in else_body {
            self.statement(statement)?;
        }
        self.leave_scope();
        let end = self.code.len();
        for jump in end_jumps {
            self.patch_jump(jump, end, position)?;
        }
        Ok(())
    }

    fn while_statement(
        &mut self,
        condition: &Expr,
        body: &[Stmt],
        position: Position,
    ) -> Result<(), CompileError> {
        let condition_pc = self.code.len();
        self.reset_temporaries();
        let condition = self.expression(condition)?;
        let exit = self.emit(
            Instruction::asbx(Opcode::JumpIfFalse, condition, 0),
            position.line,
        );
        self.reset_temporaries();
        self.break_jumps.push(Vec::new());
        self.enter_scope();
        for statement in body {
            self.statement(statement)?;
        }
        self.leave_scope();
        let back = self.emit(Instruction::asbx(Opcode::Jump, 0, 0), position.line);
        self.patch_jump(back, condition_pc, position)?;
        let loop_end = self.code.len();
        self.patch_jump(exit, loop_end, position)?;
        self.patch_breaks(loop_end, position)
    }

    fn repeat_statement(
        &mut self,
        body: &[Stmt],
        condition: &Expr,
        position: Position,
    ) -> Result<(), CompileError> {
        let body_pc = self.code.len();
        self.break_jumps.push(Vec::new());
        self.enter_scope();
        for statement in body {
            self.statement(statement)?;
        }
        self.reset_temporaries();
        let condition = self.expression(condition)?;
        let repeat = self.emit(
            Instruction::asbx(Opcode::JumpIfFalse, condition, 0),
            position.line,
        );
        self.patch_jump(repeat, body_pc, position)?;
        self.leave_scope();
        self.patch_breaks(self.code.len(), position)
    }

    fn numeric_for(
        &mut self,
        name: &(ByteString, Position),
        initial: &Expr,
        limit: &Expr,
        step: Option<&Expr>,
        body: &[Stmt],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let initial = self.expression(initial)?;
        let limit = self.expression(limit)?;
        let step = if let Some(step) = step {
            self.expression(step)?
        } else {
            let register = self.register(position)?;
            let constant = self.constant(Value::Number(1.0), position)?;
            self.emit(
                Instruction::abx(Opcode::LoadK, register, constant),
                position.line,
            );
            register
        };

        let base = self.permanent_registers;
        for (offset, source) in [initial, limit, step].into_iter().enumerate() {
            let target = self.register_at(base + offset, position)?;
            self.emit(
                Instruction::abc(Opcode::Move, target, u16::from(source), 0),
                position.line,
            );
        }
        let visible = self.register_at(base + 3, name.1)?;
        self.enter_scope();
        self.scopes
            .last_mut()
            .unwrap()
            .locals
            .insert(name.0.clone(), visible);
        self.permanent_registers = base + 4;
        self.reset_temporaries();

        let prep = self.emit(
            Instruction::asbx(Opcode::ForPrep, base as u8, 0),
            position.line,
        );
        let body_pc = self.code.len();
        self.break_jumps.push(Vec::new());
        for statement in body {
            self.statement(statement)?;
        }
        let loop_pc = self.emit(
            Instruction::asbx(Opcode::ForLoop, base as u8, 0),
            position.line,
        );
        self.patch_jump(loop_pc, body_pc, position)?;
        self.patch_jump(prep, loop_pc, position)?;
        self.leave_scope();
        self.patch_breaks(self.code.len(), position)
    }

    fn generic_for(
        &mut self,
        names: &[(ByteString, Position)],
        values: &[Expr],
        body: &[Stmt],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let triple = self.value_registers_fixed(values, 3, position)?;
        let base = self.permanent_registers;
        for (offset, source) in triple.into_iter().enumerate() {
            let target = self.register_at(base + offset, position)?;
            self.emit(
                Instruction::abc(Opcode::Move, target, u16::from(source), 0),
                position.line,
            );
        }

        self.enter_scope();
        for (offset, (name, name_position)) in names.iter().enumerate() {
            let target = self.register_at(base + 3 + offset, *name_position)?;
            self.scopes
                .last_mut()
                .unwrap()
                .locals
                .insert(name.clone(), target);
        }
        self.permanent_registers = base + 3 + names.len();
        self.reset_temporaries();

        let loop_pc = self.code.len();
        let call_base = self.register(position)?;
        self.emit(
            Instruction::abc(Opcode::Move, call_base, base as u16, 0),
            position.line,
        );
        let state = self.register(position)?;
        self.emit(
            Instruction::abc(Opcode::Move, state, (base + 1) as u16, 0),
            position.line,
        );
        let control = self.register(position)?;
        self.emit(
            Instruction::abc(Opcode::Move, control, (base + 2) as u16, 0),
            position.line,
        );
        self.emit(
            Instruction::abc(
                Opcode::Call,
                call_base,
                3,
                u16::try_from(names.len() + 1).map_err(|_| CompileError {
                    position,
                    message: "generic for has too many variables".to_owned(),
                })?,
            ),
            position.line,
        );
        for offset in 0..names.len() {
            self.emit(
                Instruction::abc(
                    Opcode::Move,
                    (base + 3 + offset) as u8,
                    u16::from(call_base) + offset as u16,
                    0,
                ),
                position.line,
            );
        }
        self.emit(
            Instruction::abc(Opcode::Move, (base + 2) as u8, u16::from(call_base), 0),
            position.line,
        );
        let exit = self.emit(
            Instruction::asbx(Opcode::JumpIfNil, call_base, 0),
            position.line,
        );
        self.reset_temporaries();
        self.break_jumps.push(Vec::new());
        for statement in body {
            self.statement(statement)?;
        }
        let back = self.emit(Instruction::asbx(Opcode::Jump, 0, 0), position.line);
        self.patch_jump(back, loop_pc, position)?;
        let loop_end = self.code.len();
        self.patch_jump(exit, loop_end, position)?;
        self.leave_scope();
        self.patch_breaks(loop_end, position)
    }

    fn patch_breaks(&mut self, target: usize, position: Position) -> Result<(), CompileError> {
        let breaks = self.break_jumps.pop().expect("loop context exists");
        for jump in breaks {
            self.patch_jump(jump, target, position)?;
        }
        Ok(())
    }
    // ANCHOR_END: control_flow_codegen

    // ANCHOR: bindings
    fn local_statement(
        &mut self,
        names: &[(ByteString, Position)],
        values: &[Expr],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let value_registers = self.value_registers_fixed(values, names.len(), position)?;
        let base = self.permanent_registers;
        for (index, (name, name_position)) in names.iter().enumerate() {
            let target = self.register_at(base + index, *name_position)?;
            if let Some(source) = value_registers.get(index) {
                self.emit(
                    Instruction::abc(Opcode::Move, target, u16::from(*source), 0),
                    position.line,
                );
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
        targets: &[LValue],
        values: &[Expr],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        let targets = targets
            .iter()
            .map(|target| self.resolve_target(target))
            .collect::<Result<Vec<_>, _>>()?;
        let value_registers = self.value_registers_fixed(values, targets.len(), position)?;
        for (target, source) in targets.into_iter().zip(value_registers) {
            match target {
                ResolvedTarget::Local(target) => {
                    self.emit(
                        Instruction::abc(Opcode::Move, target, u16::from(source), 0),
                        position.line,
                    );
                }
                ResolvedTarget::Global(constant) => {
                    self.emit(
                        Instruction::abx(Opcode::SetGlobal, source, constant),
                        position.line,
                    );
                }
                ResolvedTarget::Table { table, key } => {
                    self.emit(
                        Instruction::abc(
                            Opcode::SetTable,
                            table,
                            u16::from(key),
                            u16::from(source),
                        ),
                        position.line,
                    );
                }
            }
        }
        self.reset_temporaries();
        Ok(())
    }

    fn resolve_target(&mut self, target: &LValue) -> Result<ResolvedTarget, CompileError> {
        match target {
            LValue::Name(name, position) => {
                if let Some(register) = self.resolve_local(name) {
                    Ok(ResolvedTarget::Local(register))
                } else {
                    Ok(ResolvedTarget::Global(
                        self.constant(Value::String(name.clone()), *position)?,
                    ))
                }
            }
            LValue::Index(table, key, _) => Ok(ResolvedTarget::Table {
                table: self.expression(table)?,
                key: self.expression(key)?,
            }),
        }
    }

    fn value_registers_fixed(
        &mut self,
        values: &[Expr],
        wanted: usize,
        position: Position,
    ) -> Result<Vec<u8>, CompileError> {
        let mut registers = Vec::with_capacity(wanted);
        for (index, value) in values.iter().enumerate() {
            let is_tail = index + 1 == values.len();
            if is_tail && self.is_expandable(value) && registers.len() < wanted {
                let count = wanted - registers.len();
                let base = self.reserve_window(count, position)?;
                self.expression_into(value, base, Some(count), position)?;
                registers.extend((0..count).map(|offset| base + offset as u8));
            } else {
                let register = self.expression(value)?;
                if registers.len() < wanted {
                    registers.push(register);
                }
            }
        }
        while registers.len() < wanted {
            registers.push(self.load_nil(position)?);
        }
        Ok(registers)
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
            Expr::Table(fields, position) => {
                let table = self.register(*position)?;
                self.emit(
                    Instruction::abc(Opcode::NewTable, table, 0, 0),
                    position.line,
                );
                let mut list_index = 1_usize;
                for (field_offset, field) in fields.iter().enumerate() {
                    match field {
                        TableField::List(value) => {
                            if field_offset + 1 == fields.len() && self.is_expandable(value) {
                                let result_base = self.reserve_window(1, *position)?;
                                self.expression_into(value, result_base, None, *position)?;
                                self.emit(
                                    Instruction::abc(
                                        Opcode::SetListOpen,
                                        table,
                                        u16::from(result_base),
                                        u16::try_from(list_index).map_err(|_| CompileError {
                                            position: *position,
                                            message: "table constructor has too many list fields"
                                                .to_owned(),
                                        })?,
                                    ),
                                    position.line,
                                );
                                continue;
                            }
                            let value = self.expression(value)?;
                            if let Ok(index) = u16::try_from(list_index) {
                                if index <= 511 {
                                    self.emit(
                                        Instruction::abc(
                                            Opcode::SetList,
                                            table,
                                            u16::from(value),
                                            index,
                                        ),
                                        position.line,
                                    );
                                } else {
                                    self.set_numeric_field(table, value, list_index, *position)?;
                                }
                            } else {
                                self.set_numeric_field(table, value, list_index, *position)?;
                            }
                            list_index += 1;
                        }
                        TableField::Record(key, value) => {
                            let key = self.expression(key)?;
                            let value = self.expression(value)?;
                            self.emit(
                                Instruction::abc(
                                    Opcode::SetTable,
                                    table,
                                    u16::from(key),
                                    u16::from(value),
                                ),
                                position.line,
                            );
                        }
                    }
                }
                Ok(table)
            }
            Expr::Index(table, key, position) => {
                let table = self.expression(table)?;
                let key = self.expression(key)?;
                let result = self.register(*position)?;
                self.emit(
                    Instruction::abc(Opcode::GetTable, result, u16::from(table), u16::from(key)),
                    position.line,
                );
                Ok(result)
            }
            Expr::Function {
                parameters,
                is_vararg,
                body,
                position,
            } => {
                let proto =
                    Codegen::new().compile_function(parameters, *is_vararg, body, *position)?;
                let index = u32::try_from(self.protos.len()).map_err(|_| CompileError {
                    position: *position,
                    message: "too many nested function prototypes".to_owned(),
                })?;
                if index > 262_143 {
                    return Err(CompileError {
                        position: *position,
                        message: "too many nested functions for CLOSURE".to_owned(),
                    });
                }
                self.protos.push(Rc::new(proto));
                let register = self.register(*position)?;
                self.emit(
                    Instruction::abx(Opcode::Closure, register, index),
                    position.line,
                );
                Ok(register)
            }
            Expr::Call {
                callee,
                arguments,
                position,
            } => {
                let target = self.register(*position)?;
                self.compile_call_into(
                    target,
                    callee,
                    arguments,
                    CallMode::Results(Some(1)),
                    *position,
                )?;
                Ok(target)
            }
            Expr::MethodCall {
                receiver,
                method,
                arguments,
                position,
            } => {
                let target = self.register(*position)?;
                self.compile_method_call_into(
                    target,
                    receiver,
                    method,
                    arguments,
                    CallMode::Results(Some(1)),
                    *position,
                )?;
                Ok(target)
            }
            Expr::Vararg(position) => {
                if !self.is_vararg {
                    return Err(CompileError {
                        position: *position,
                        message: "cannot use '...' outside a vararg function".to_owned(),
                    });
                }
                let target = self.register(*position)?;
                self.emit(
                    Instruction::abc(Opcode::Vararg, target, 2, 0),
                    position.line,
                );
                Ok(target)
            }
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

    fn set_numeric_field(
        &mut self,
        table: u8,
        value: u8,
        index: usize,
        position: Position,
    ) -> Result<(), CompileError> {
        let key = self.register(position)?;
        let constant = self.constant(Value::Number(index as f64), position)?;
        self.emit(
            Instruction::abx(Opcode::LoadK, key, constant),
            position.line,
        );
        self.emit(
            Instruction::abc(Opcode::SetTable, table, u16::from(key), u16::from(value)),
            position.line,
        );
        Ok(())
    }

    // ANCHOR: result_protocol
    fn return_statement(
        &mut self,
        values: &[Expr],
        position: Position,
    ) -> Result<(), CompileError> {
        self.reset_temporaries();
        if values.is_empty() {
            self.emit(Instruction::abc(Opcode::Return, 0, 1, 0), position.line);
            return Ok(());
        }
        if values.len() == 1 {
            match &values[0] {
                Expr::Call {
                    callee,
                    arguments,
                    position: call_position,
                } => {
                    let target = self.register(*call_position)?;
                    self.compile_call_into(
                        target,
                        callee,
                        arguments,
                        CallMode::Tail,
                        *call_position,
                    )?;
                    self.reset_temporaries();
                    return Ok(());
                }
                Expr::MethodCall {
                    receiver,
                    method,
                    arguments,
                    position: call_position,
                } => {
                    let target = self.register(*call_position)?;
                    self.compile_method_call_into(
                        target,
                        receiver,
                        method,
                        arguments,
                        CallMode::Tail,
                        *call_position,
                    )?;
                    self.reset_temporaries();
                    return Ok(());
                }
                _ => {}
            }
        }

        let tail_is_open = values.last().is_some_and(|value| self.is_expandable(value));
        let prefix_len = if tail_is_open {
            values.len() - 1
        } else {
            values.len()
        };
        let prefix = values[..prefix_len]
            .iter()
            .map(|value| self.expression(value))
            .collect::<Result<Vec<_>, _>>()?;
        let base = self.reserve_window(prefix_len + usize::from(tail_is_open), position)?;
        for (offset, source) in prefix.into_iter().enumerate() {
            self.emit(
                Instruction::abc(Opcode::Move, base + offset as u8, u16::from(source), 0),
                position.line,
            );
        }
        if tail_is_open {
            self.expression_into(
                values.last().unwrap(),
                base + prefix_len as u8,
                None,
                position,
            )?;
            self.emit(Instruction::abc(Opcode::Return, base, 0, 0), position.line);
        } else {
            self.emit(
                Instruction::abc(
                    Opcode::Return,
                    base,
                    u16::try_from(values.len() + 1).map_err(|_| CompileError {
                        position,
                        message: "return has too many values".to_owned(),
                    })?,
                    0,
                ),
                position.line,
            );
        }
        self.reset_temporaries();
        Ok(())
    }

    fn expression_into(
        &mut self,
        expression: &Expr,
        target: u8,
        wanted: Option<usize>,
        position: Position,
    ) -> Result<(), CompileError> {
        match expression {
            Expr::Call {
                callee,
                arguments,
                position,
            } => self.compile_call_into(
                target,
                callee,
                arguments,
                CallMode::Results(wanted),
                *position,
            ),
            Expr::MethodCall {
                receiver,
                method,
                arguments,
                position,
            } => self.compile_method_call_into(
                target,
                receiver,
                method,
                arguments,
                CallMode::Results(wanted),
                *position,
            ),
            Expr::Vararg(vararg_position) => {
                if !self.is_vararg {
                    return Err(CompileError {
                        position: *vararg_position,
                        message: "cannot use '...' outside a vararg function".to_owned(),
                    });
                }
                let count = Self::result_operand(wanted, *vararg_position)?;
                self.emit(
                    Instruction::abc(Opcode::Vararg, target, count, 0),
                    vararg_position.line,
                );
                Ok(())
            }
            _ => {
                let source = self.expression(expression)?;
                self.emit(
                    Instruction::abc(Opcode::Move, target, u16::from(source), 0),
                    position.line,
                );
                if let Some(wanted) = wanted {
                    for offset in 1..wanted {
                        let nil = self.nil_constant(position)?;
                        self.emit(
                            Instruction::abx(Opcode::LoadK, target + offset as u8, nil),
                            position.line,
                        );
                    }
                }
                Ok(())
            }
        }
    }

    fn compile_call_into(
        &mut self,
        target: u8,
        callee: &Expr,
        arguments: &[Expr],
        mode: CallMode,
        position: Position,
    ) -> Result<(), CompileError> {
        let callee = self.expression(callee)?;
        self.emit(
            Instruction::abc(Opcode::Move, target, u16::from(callee), 0),
            position.line,
        );
        let first_argument = target.checked_add(1).ok_or_else(|| CompileError {
            position,
            message: "call needs more than 255 registers".to_owned(),
        })?;
        let open_arguments = self.compile_arguments(first_argument, arguments, position)?;
        let argument_operand = if open_arguments {
            0
        } else {
            u16::try_from(arguments.len() + 1).map_err(|_| CompileError {
                position,
                message: "call has too many arguments".to_owned(),
            })?
        };
        let opcode = match mode {
            CallMode::Tail => Opcode::TailCall,
            CallMode::Results(_) => Opcode::Call,
        };
        self.emit(
            Instruction::abc(
                opcode,
                target,
                argument_operand,
                match mode {
                    CallMode::Tail => 0,
                    CallMode::Results(wanted) => Self::result_operand(wanted, position)?,
                },
            ),
            position.line,
        );
        Ok(())
    }

    fn compile_method_call_into(
        &mut self,
        target: u8,
        receiver: &Expr,
        method: &ByteString,
        arguments: &[Expr],
        mode: CallMode,
        position: Position,
    ) -> Result<(), CompileError> {
        let receiver = self.expression(receiver)?;
        let key = self.register(position)?;
        let constant = self.constant(Value::String(method.clone()), position)?;
        self.emit(
            Instruction::abx(Opcode::LoadK, key, constant),
            position.line,
        );
        let method_register = self.register(position)?;
        self.emit(
            Instruction::abc(
                Opcode::GetTable,
                method_register,
                u16::from(receiver),
                u16::from(key),
            ),
            position.line,
        );
        self.emit(
            Instruction::abc(Opcode::Move, target, u16::from(method_register), 0),
            position.line,
        );
        let self_slot = target.checked_add(1).ok_or_else(|| CompileError {
            position,
            message: "method call needs more than 255 registers".to_owned(),
        })?;
        self.ensure_register(usize::from(self_slot), position)?;
        self.emit(
            Instruction::abc(Opcode::Move, self_slot, u16::from(receiver), 0),
            position.line,
        );
        let first_argument = target.checked_add(2).ok_or_else(|| CompileError {
            position,
            message: "method call needs more than 255 registers".to_owned(),
        })?;
        let open_arguments = self.compile_arguments(first_argument, arguments, position)?;
        let argument_operand = if open_arguments {
            0
        } else {
            u16::try_from(arguments.len() + 2).map_err(|_| CompileError {
                position,
                message: "method call has too many arguments".to_owned(),
            })?
        };
        let opcode = match mode {
            CallMode::Tail => Opcode::TailCall,
            CallMode::Results(_) => Opcode::Call,
        };
        self.emit(
            Instruction::abc(
                opcode,
                target,
                argument_operand,
                match mode {
                    CallMode::Tail => 0,
                    CallMode::Results(wanted) => Self::result_operand(wanted, position)?,
                },
            ),
            position.line,
        );
        Ok(())
    }

    fn compile_arguments(
        &mut self,
        first: u8,
        arguments: &[Expr],
        position: Position,
    ) -> Result<bool, CompileError> {
        for (offset, argument) in arguments.iter().enumerate() {
            let target_index =
                usize::from(first)
                    .checked_add(offset)
                    .ok_or_else(|| CompileError {
                        position,
                        message: "call needs more than 255 registers".to_owned(),
                    })?;
            let target = self.ensure_register(target_index, position)?;
            let open = offset + 1 == arguments.len() && self.is_expandable(argument);
            if open {
                self.expression_into(argument, target, None, position)?;
            } else {
                self.expression_into(argument, target, Some(1), position)?;
            }
        }
        Ok(arguments
            .last()
            .is_some_and(|argument| self.is_expandable(argument)))
    }

    fn result_operand(wanted: Option<usize>, position: Position) -> Result<u16, CompileError> {
        match wanted {
            None => Ok(0),
            Some(count) => u16::try_from(count + 1).map_err(|_| CompileError {
                position,
                message: "call requests too many results".to_owned(),
            }),
        }
    }

    fn is_expandable(&self, expression: &Expr) -> bool {
        matches!(
            expression,
            Expr::Call { .. } | Expr::MethodCall { .. } | Expr::Vararg(_)
        )
    }

    fn reserve_window(&mut self, count: usize, position: Position) -> Result<u8, CompileError> {
        let base = self.next_register;
        for offset in 0..count {
            self.ensure_register(base + offset, position)?;
        }
        Ok(base as u8)
    }

    fn ensure_register(&mut self, index: usize, position: Position) -> Result<u8, CompileError> {
        let register = self.register_at(index, position)?;
        self.next_register = self.next_register.max(index + 1);
        Ok(register)
    }
    // ANCHOR_END: result_protocol

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
    If,
    Then,
    ElseIf,
    Else,
    While,
    Repeat,
    Until,
    For,
    In,
    Break,
    Function,
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
    Ellipsis,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Dot,
    Colon,
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
        let statements = self.block(&[])?;
        if self.current().kind != TokenKind::Eof {
            return Err(self.error("expected end of input"));
        }
        Ok(Chunk {
            statements,
            position,
        })
    }

    fn block(&mut self, terminators: &[TokenKind]) -> Result<Vec<Stmt>, CompileError> {
        let mut statements = Vec::new();
        while self.current().kind != TokenKind::Eof && !terminators.contains(&self.current().kind) {
            let statement = self.statement()?;
            let is_return = matches!(statement, Stmt::Return { .. });
            statements.push(statement);
            if is_return {
                if self.current().kind == TokenKind::Semicolon {
                    self.advance();
                }
                let at_end = self.current().kind == TokenKind::Eof
                    || terminators.contains(&self.current().kind);
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
            TokenKind::Function => self.function_statement(),
            TokenKind::Do => self.do_statement(),
            TokenKind::Return => self.return_statement(),
            TokenKind::If => self.if_statement(),
            TokenKind::While => self.while_statement(),
            TokenKind::Repeat => self.repeat_statement(),
            TokenKind::For => self.for_statement(),
            TokenKind::Break => {
                let position = self.advance().position;
                Ok(Stmt::Break(position))
            }
            TokenKind::Name(_) => self.assignment_or_call(),
            _ => Err(self.error("expected local, assignment, do block or return")),
        }
    }

    fn local_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        if self.current().kind == TokenKind::Function {
            self.advance();
            let token = self.advance().clone();
            let TokenKind::Name(name) = token.kind else {
                return Err(CompileError {
                    position: token.position,
                    message: "expected local function name".to_owned(),
                });
            };
            let function = self.function_body(position, false)?;
            return Ok(Stmt::Local {
                names: vec![(name, token.position)],
                values: vec![function],
                position,
            });
        }
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

    fn assignment_or_call(&mut self) -> Result<Stmt, CompileError> {
        let position = self.current().position;
        let token = self.advance().clone();
        let TokenKind::Name(name) = token.kind else {
            unreachable!()
        };
        let expression = self.postfix(Expr::Name(name, token.position))?;
        if matches!(expression, Expr::Call { .. } | Expr::MethodCall { .. })
            && self.current().kind != TokenKind::Comma
            && self.current().kind != TokenKind::Equal
        {
            return Ok(Stmt::Expression(expression));
        }
        let mut targets = vec![self.expression_to_lvalue(expression, token.position)?];
        while self.current().kind == TokenKind::Comma {
            self.advance();
            targets.push(self.assignment_target()?);
        }
        if self.current().kind != TokenKind::Equal {
            return Err(self.error("expected '=' after assignment targets"));
        }
        self.advance();
        let values = self.expression_list()?;
        Ok(Stmt::Assign {
            targets,
            values,
            position,
        })
    }

    fn expression_to_lvalue(
        &self,
        expression: Expr,
        position: Position,
    ) -> Result<LValue, CompileError> {
        match expression {
            Expr::Name(name, position) => Ok(LValue::Name(name, position)),
            Expr::Index(table, key, position) => Ok(LValue::Index(*table, *key, position)),
            _ => Err(CompileError {
                position,
                message: "expression is not assignable".to_owned(),
            }),
        }
    }

    fn assignment_target(&mut self) -> Result<LValue, CompileError> {
        let token = self.advance().clone();
        let TokenKind::Name(name) = token.kind else {
            return Err(CompileError {
                position: token.position,
                message: "expected assignment target".to_owned(),
            });
        };
        let expression = self.postfix(Expr::Name(name, token.position))?;
        match expression {
            Expr::Name(name, position) => Ok(LValue::Name(name, position)),
            Expr::Index(table, key, position) => Ok(LValue::Index(*table, *key, position)),
            _ => Err(CompileError {
                position: token.position,
                message: "expression is not assignable".to_owned(),
            }),
        }
    }

    fn function_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let token = self.advance().clone();
        let TokenKind::Name(name) = token.kind else {
            return Err(CompileError {
                position: token.position,
                message: "expected function name".to_owned(),
            });
        };
        let mut target = Expr::Name(name, token.position);
        while self.current().kind == TokenKind::Dot {
            let dot = self.advance().position;
            let field = self.advance().clone();
            let TokenKind::Name(name) = field.kind else {
                return Err(CompileError {
                    position: field.position,
                    message: "expected field name after '.'".to_owned(),
                });
            };
            let key = Expr::Literal(Value::String(name), field.position);
            target = Expr::Index(Box::new(target), Box::new(key), dot);
        }
        let method = if self.current().kind == TokenKind::Colon {
            let colon = self.advance().position;
            let field = self.advance().clone();
            let TokenKind::Name(name) = field.kind else {
                return Err(CompileError {
                    position: field.position,
                    message: "expected method name after ':'".to_owned(),
                });
            };
            let key = Expr::Literal(Value::String(name), field.position);
            target = Expr::Index(Box::new(target), Box::new(key), colon);
            true
        } else {
            false
        };
        let target = self.expression_to_lvalue(target, token.position)?;
        let function = self.function_body(position, method)?;
        Ok(Stmt::Assign {
            targets: vec![target],
            values: vec![function],
            position,
        })
    }

    fn function_body(&mut self, position: Position, method: bool) -> Result<Expr, CompileError> {
        self.expect(TokenKind::LeftParen, "expected '(' before parameters")?;
        let mut parameters = Vec::new();
        let mut is_vararg = false;
        if method {
            parameters.push((ByteString::new(&b"self"[..]), position));
        }
        if self.current().kind != TokenKind::RightParen {
            loop {
                if self.current().kind == TokenKind::Ellipsis {
                    self.advance();
                    is_vararg = true;
                    break;
                }
                let token = self.advance().clone();
                let TokenKind::Name(name) = token.kind else {
                    return Err(CompileError {
                        position: token.position,
                        message: "expected parameter name".to_owned(),
                    });
                };
                parameters.push((name, token.position));
                if self.current().kind != TokenKind::Comma {
                    break;
                }
                self.advance();
                if self.current().kind == TokenKind::RightParen {
                    return Err(self.error("expected parameter name or '...' after ','"));
                }
            }
        }
        self.expect(TokenKind::RightParen, "expected ')' after parameters")?;
        let body = self.block(&[TokenKind::End])?;
        self.expect(TokenKind::End, "expected 'end' to close function")?;
        Ok(Expr::Function {
            parameters,
            is_vararg,
            body,
            position,
        })
    }

    fn do_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let body = self.block(&[TokenKind::End])?;
        if self.current().kind != TokenKind::End {
            return Err(self.error("expected 'end' to close do block"));
        }
        self.advance();
        Ok(Stmt::Do { body, position })
    }

    fn if_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let mut branches = Vec::new();
        let condition = self.expression(0)?;
        self.expect(TokenKind::Then, "expected 'then' after if condition")?;
        let body = self.block(&[TokenKind::ElseIf, TokenKind::Else, TokenKind::End])?;
        branches.push((condition, body));
        while self.current().kind == TokenKind::ElseIf {
            self.advance();
            let condition = self.expression(0)?;
            self.expect(TokenKind::Then, "expected 'then' after elseif condition")?;
            let body = self.block(&[TokenKind::ElseIf, TokenKind::Else, TokenKind::End])?;
            branches.push((condition, body));
        }
        let else_body = if self.current().kind == TokenKind::Else {
            self.advance();
            self.block(&[TokenKind::End])?
        } else {
            Vec::new()
        };
        self.expect(TokenKind::End, "expected 'end' to close if")?;
        Ok(Stmt::If {
            branches,
            else_body,
            position,
        })
    }

    fn while_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let condition = self.expression(0)?;
        self.expect(TokenKind::Do, "expected 'do' after while condition")?;
        let body = self.block(&[TokenKind::End])?;
        self.expect(TokenKind::End, "expected 'end' to close while")?;
        Ok(Stmt::While {
            condition,
            body,
            position,
        })
    }

    fn repeat_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let body = self.block(&[TokenKind::Until])?;
        self.expect(TokenKind::Until, "expected 'until' to close repeat")?;
        let condition = self.expression(0)?;
        Ok(Stmt::Repeat {
            body,
            condition,
            position,
        })
    }

    fn for_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let token = self.advance().clone();
        let TokenKind::Name(name) = token.kind else {
            return Err(CompileError {
                position: token.position,
                message: "expected for-loop variable".to_owned(),
            });
        };
        if self.current().kind != TokenKind::Equal {
            let mut names = vec![(name, token.position)];
            while self.current().kind == TokenKind::Comma {
                self.advance();
                let token = self.advance().clone();
                let TokenKind::Name(name) = token.kind else {
                    return Err(CompileError {
                        position: token.position,
                        message: "expected generic for variable".to_owned(),
                    });
                };
                names.push((name, token.position));
            }
            self.expect(TokenKind::In, "expected 'in' in generic for")?;
            let values = self.expression_list()?;
            self.expect(TokenKind::Do, "expected 'do' after generic for expressions")?;
            let body = self.block(&[TokenKind::End])?;
            self.expect(TokenKind::End, "expected 'end' to close generic for")?;
            return Ok(Stmt::GenericFor {
                names,
                values,
                body,
                position,
            });
        }
        self.advance();
        let initial = self.expression(0)?;
        self.expect(
            TokenKind::Comma,
            "expected ',' after numeric for initial value",
        )?;
        let limit = self.expression(0)?;
        let step = if self.current().kind == TokenKind::Comma {
            self.advance();
            Some(self.expression(0)?)
        } else {
            None
        };
        self.expect(TokenKind::Do, "expected 'do' after numeric for range")?;
        let body = self.block(&[TokenKind::End])?;
        self.expect(TokenKind::End, "expected 'end' to close numeric for")?;
        Ok(Stmt::NumericFor {
            name: (name, token.position),
            initial,
            limit,
            step,
            body,
            position,
        })
    }

    fn return_statement(&mut self) -> Result<Stmt, CompileError> {
        let position = self.advance().position;
        let values = if matches!(
            self.current().kind,
            TokenKind::Eof
                | TokenKind::End
                | TokenKind::ElseIf
                | TokenKind::Else
                | TokenKind::Until
                | TokenKind::Semicolon
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

    fn expect(&mut self, kind: TokenKind, message: &'static str) -> Result<(), CompileError> {
        if self.current().kind != kind {
            return Err(self.error(message));
        }
        self.advance();
        Ok(())
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
            TokenKind::LeftBrace => self.table_constructor(token.position)?,
            TokenKind::Function => self.function_body(token.position, false)?,
            TokenKind::Ellipsis => Expr::Vararg(token.position),
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
        left = self.postfix(left)?;
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

    fn postfix(&mut self, mut expression: Expr) -> Result<Expr, CompileError> {
        loop {
            match self.current().kind {
                TokenKind::LeftBracket => {
                    let position = self.advance().position;
                    let key = self.expression(0)?;
                    self.expect(TokenKind::RightBracket, "expected ']' after table key")?;
                    expression = Expr::Index(Box::new(expression), Box::new(key), position);
                }
                TokenKind::Dot => {
                    let position = self.advance().position;
                    let token = self.advance().clone();
                    let TokenKind::Name(name) = token.kind else {
                        return Err(CompileError {
                            position: token.position,
                            message: "expected field name after '.'".to_owned(),
                        });
                    };
                    let key = Expr::Literal(Value::String(name), token.position);
                    expression = Expr::Index(Box::new(expression), Box::new(key), position);
                }
                TokenKind::LeftParen => {
                    let position = self.current().position;
                    let arguments = self.call_arguments()?;
                    expression = Expr::Call {
                        callee: Box::new(expression),
                        arguments,
                        position,
                    };
                }
                TokenKind::Colon => {
                    let position = self.advance().position;
                    let token = self.advance().clone();
                    let TokenKind::Name(method) = token.kind else {
                        return Err(CompileError {
                            position: token.position,
                            message: "expected method name after ':'".to_owned(),
                        });
                    };
                    let arguments = self.call_arguments()?;
                    expression = Expr::MethodCall {
                        receiver: Box::new(expression),
                        method,
                        arguments,
                        position,
                    };
                }
                _ => return Ok(expression),
            }
        }
    }

    fn call_arguments(&mut self) -> Result<Vec<Expr>, CompileError> {
        self.expect(TokenKind::LeftParen, "expected '(' before arguments")?;
        let arguments = if self.current().kind == TokenKind::RightParen {
            Vec::new()
        } else {
            self.expression_list()?
        };
        self.expect(TokenKind::RightParen, "expected ')' after arguments")?;
        Ok(arguments)
    }

    fn table_constructor(&mut self, position: Position) -> Result<Expr, CompileError> {
        let mut fields = Vec::new();
        while self.current().kind != TokenKind::RightBrace {
            if self.current().kind == TokenKind::Eof {
                return Err(self.error("expected '}' to close table constructor"));
            }
            let field = if self.current().kind == TokenKind::LeftBracket {
                self.advance();
                let key = self.expression(0)?;
                self.expect(TokenKind::RightBracket, "expected ']' after record key")?;
                self.expect(TokenKind::Equal, "expected '=' after record key")?;
                TableField::Record(key, self.expression(0)?)
            } else if matches!(self.current().kind, TokenKind::Name(_))
                && self.peek_token(1).kind == TokenKind::Equal
            {
                let token = self.advance().clone();
                let TokenKind::Name(name) = token.kind else {
                    unreachable!()
                };
                self.advance();
                TableField::Record(
                    Expr::Literal(Value::String(name), token.position),
                    self.expression(0)?,
                )
            } else {
                TableField::List(self.expression(0)?)
            };
            fields.push(field);
            if matches!(self.current().kind, TokenKind::Comma | TokenKind::Semicolon) {
                self.advance();
            } else if self.current().kind != TokenKind::RightBrace {
                return Err(self.error("expected ',', ';' or '}' after table field"));
            }
        }
        self.advance();
        Ok(Expr::Table(fields, position))
    }

    fn current(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn peek_token(&self, distance: usize) -> &Token {
        self.tokens
            .get(self.cursor + distance)
            .unwrap_or_else(|| self.tokens.last().expect("token stream has EOF"))
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
            b'{' => Ok(self.one(TokenKind::LeftBrace)),
            b'}' => Ok(self.one(TokenKind::RightBrace)),
            b'[' => Ok(self.one(TokenKind::LeftBracket)),
            b']' => Ok(self.one(TokenKind::RightBracket)),
            b'=' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::EqualEqual)),
            b'=' => Ok(self.one(TokenKind::Equal)),
            b',' => Ok(self.one(TokenKind::Comma)),
            b';' => Ok(self.one(TokenKind::Semicolon)),
            b':' => Ok(self.one(TokenKind::Colon)),
            b'~' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::NotEqual)),
            b'<' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::LessEqual)),
            b'<' => Ok(self.one(TokenKind::Less)),
            b'>' if self.peek(1) == Some(b'=') => Ok(self.two(TokenKind::GreaterEqual)),
            b'>' => Ok(self.one(TokenKind::Greater)),
            b'.' if self.peek(1) == Some(b'.') && self.peek(2) == Some(b'.') => {
                self.advance_by(3);
                Ok(TokenKind::Ellipsis)
            }
            b'.' if self.peek(1) == Some(b'.') => Ok(self.two(TokenKind::DotDot)),
            b'.' => Ok(self.one(TokenKind::Dot)),
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
            b"if" => TokenKind::If,
            b"then" => TokenKind::Then,
            b"elseif" => TokenKind::ElseIf,
            b"else" => TokenKind::Else,
            b"while" => TokenKind::While,
            b"repeat" => TokenKind::Repeat,
            b"until" => TokenKind::Until,
            b"for" => TokenKind::For,
            b"in" => TokenKind::In,
            b"break" => TokenKind::Break,
            b"function" => TokenKind::Function,
            b"nil" => TokenKind::Nil,
            b"false" => TokenKind::False,
            b"true" => TokenKind::True,
            b"not" => TokenKind::Not,
            b"and" => TokenKind::And,
            b"or" => TokenKind::Or,
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

    #[test]
    fn function_protos_record_parameters_and_source_lines() {
        let proto =
            compile(b"local f = function(left, right) return left + right end\nreturn f(1, 2)")
                .unwrap();
        assert_eq!(proto.protos.len(), 1);
        assert_eq!(proto.protos[0].num_params, 2);
        assert!(!proto.protos[0].is_vararg);
        assert_eq!(proto.protos[0].debug_name, "function@1");
        assert_eq!(proto.validate_shape(), Ok(()));
    }

    #[test]
    fn vararg_and_tail_calls_are_explicit_bytecode_contracts() {
        let proto = compile(
            b"function forward(first, ...) return forward(first - 1, ...) end\nreturn forward(2, 3)",
        )
        .unwrap();
        let function = &proto.protos[0];
        assert!(function.is_vararg);
        let opcodes = function
            .code
            .iter()
            .map(|instruction| instruction.opcode().unwrap())
            .collect::<Vec<_>>();
        assert!(opcodes.contains(&Opcode::Vararg));
        assert!(opcodes.contains(&Opcode::TailCall));
    }
}
