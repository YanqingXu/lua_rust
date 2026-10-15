//! Independent fixed programs, not historical VM snapshots or a GC implementation.
use std::collections::BTreeMap;

pub type Id = usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    Number(i64),
    Nil,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    Suspended,
    Running,
    Normal,
    Dead,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Frame {
    pub pc: usize,
    pub local: i64,
}

pub struct Coroutine {
    pub frame: Option<Frame>,
    pub status: Status,
    seed: i64,
    started: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Yield(Vec<Value>),
    Return(Vec<Value>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResumeError {
    Unknown(Id),
    Dead(Id),
    LostFrame(Id),
    BadPc(usize),
}

pub type SaveFrame = fn(&mut Coroutine, &mut Option<Frame>, Frame);
pub type ResumePc = fn(&Frame) -> usize;

#[derive(Clone, Copy)]
pub struct Design {
    pub save: SaveFrame,
    pub pc: ResumePc,
}

pub struct ContinuationVm {
    pub coroutines: BTreeMap<Id, Coroutine>,
    pub starts: BTreeMap<Id, usize>,
    pub scratch: Option<Frame>,
    design: Design,
}

impl ContinuationVm {
    pub fn new(design: Design, seeds: &[(Id, i64)]) -> Self {
        Self {
            coroutines: seeds
                .iter()
                .map(|&(id, seed)| {
                    (
                        id,
                        Coroutine {
                            frame: None,
                            status: Status::Suspended,
                            seed,
                            started: false,
                        },
                    )
                })
                .collect(),
            starts: seeds.iter().map(|&(id, _)| (id, 0)).collect(),
            scratch: None,
            design,
        }
    }

    pub fn resume(&mut self, id: Id, input: i64) -> Result<Outcome, ResumeError> {
        let state = self
            .coroutines
            .get_mut(&id)
            .ok_or(ResumeError::Unknown(id))?;
        if state.status == Status::Dead {
            return Err(ResumeError::Dead(id));
        }
        let mut frame = if state.started {
            // The scratch fallback deliberately lets the faulty storage design run.
            let mut saved = state
                .frame
                .take()
                .or_else(|| self.scratch.take())
                .ok_or(ResumeError::LostFrame(id))?;
            saved.pc = (self.design.pc)(&saved);
            saved
        } else {
            state.started = true;
            Frame {
                pc: 0,
                local: state.seed,
            }
        };
        state.status = Status::Running;
        loop {
            match frame.pc {
                0 => {
                    *self.starts.get_mut(&id).unwrap() += 1;
                    frame.local = self.coroutines[&id].seed;
                    frame.pc = 1;
                }
                1 | 3 => {
                    let values = vec![Value::Number(frame.local), Value::Nil];
                    frame.pc += 1;
                    let state = self.coroutines.get_mut(&id).unwrap();
                    state.status = Status::Suspended;
                    (self.design.save)(state, &mut self.scratch, frame);
                    return Ok(Outcome::Yield(values));
                }
                2 => {
                    frame.local += input;
                    frame.pc = 3;
                }
                4 => {
                    self.coroutines.get_mut(&id).unwrap().status = Status::Dead;
                    return Ok(Outcome::Return(vec![
                        Value::Number(frame.local + input),
                        Value::Nil,
                    ]));
                }
                other => return Err(ResumeError::BadPc(other)),
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct ChildYield {
    pub values: Vec<Value>,
    pub frames: Vec<&'static str>,
}

pub struct TurnVm {
    pub statuses: [Status; 2],
    pub current: Id,
    pub yield_values: Option<Vec<Value>>,
    pub yield_frames: Vec<&'static str>,
    pub observed_parent: Option<Status>,
    pub observed_current: Option<Id>,
    pub child_saw_empty_channels: bool,
}

impl TurnVm {
    pub fn new() -> Self {
        Self {
            statuses: [Status::Running, Status::Suspended],
            current: 0,
            // Sentinels make restoration observable; normal nested Lua entry can be empty.
            yield_values: Some(vec![Value::Number(900)]),
            yield_frames: vec!["outer-frame"],
            observed_parent: None,
            observed_current: None,
            child_saw_empty_channels: false,
        }
    }

    pub fn child_body(&mut self) {
        self.observed_parent = Some(self.statuses[0]);
        self.observed_current = Some(self.current);
        self.child_saw_empty_channels = self.yield_values.is_none() && self.yield_frames.is_empty();
        self.yield_values.get_or_insert_with(Vec::new).extend([
            Value::Number(7),
            Value::Nil,
            Value::Number(9),
        ]);
        self.yield_frames.push("child-frame");
    }
}
