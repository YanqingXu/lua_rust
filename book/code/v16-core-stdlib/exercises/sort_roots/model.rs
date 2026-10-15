//! 真实 Heap 的 proposal 模型：模拟 insertion snapshot 与 comparator 的 root 协议。
//! 不修改真实 VM，不模拟所有 callable/yield/GC phase，也不扫描任意 Rust Vec。
use lua_core::{ByteString, Heap, HeapError, TableRef, Value};

pub type Operation = fn(&mut SortVm, &mut [Value]) -> Result<(), SortError>;
pub type Scope = fn(&mut SortVm, &mut [Value], Operation) -> Result<(), SortError>;

#[derive(Debug, PartialEq)]
pub enum SortError {
    Heap(HeapError),
    Comparator,
}

impl From<HeapError> for SortError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

#[derive(Clone, Copy)]
pub enum Scenario {
    ClearAndCollect,
    ErrorAfterMutation,
}

pub fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}

pub struct SortVm {
    pub heap: Heap,
    pub source: TableRef,
    pub prefix: TableRef,
    pub items: Vec<TableRef>,
    pub temporary_roots: Vec<Value>,
    pub comparisons: usize,
    scenario: Scenario,
    cleared: bool,
}

impl SortVm {
    pub fn new(scenario: Scenario) -> Self {
        let mut heap = Heap::new();
        let prefix = heap.allocate_table().unwrap();
        heap.table_set(prefix, key("sentinel"), Value::Number(900.0))
            .unwrap();
        let source = heap.allocate_table().unwrap();
        let mut items = Vec::new();
        for rank in [4, 3, 2, 1] {
            let item = heap.allocate_table().unwrap();
            heap.table_set(item, key("rank"), Value::Number(f64::from(rank)))
                .unwrap();
            items.push(item);
            heap.table_set(
                source,
                Value::Number(items.len() as f64),
                Value::Table(item),
            )
            .unwrap();
        }
        Self {
            heap,
            source,
            prefix,
            items,
            temporary_roots: vec![Value::Table(prefix)],
            comparisons: 0,
            scenario,
            cleared: false,
        }
    }

    pub fn collect(&mut self) -> Result<(), HeapError> {
        let mut roots = vec![Value::Table(self.source)];
        roots.extend(self.temporary_roots.iter().cloned());
        self.heap.collect(&roots, &[])?;
        Ok(())
    }

    pub fn clear_source(&mut self) -> Result<(), HeapError> {
        for index in 1..=self.items.len() {
            self.heap
                .table_set(self.source, Value::Number(index as f64), Value::Nil)?;
        }
        Ok(())
    }

    pub fn rank(&self, value: &Value) -> Result<f64, HeapError> {
        let Value::Table(table) = value else {
            panic!("fixture item must be a table")
        };
        let Value::Number(rank) = self.heap.table_get(*table, &key("rank"))? else {
            panic!("fixture item must have a numeric rank")
        };
        Ok(rank)
    }

    pub fn source_ranks(&self) -> Result<Vec<f64>, HeapError> {
        (1..=self.items.len())
            .map(|index| {
                let value = self
                    .heap
                    .table_get(self.source, &Value::Number(index as f64))?;
                self.rank(&value)
            })
            .collect()
    }

    pub fn run_sort(&mut self, scope: Scope) -> Result<(), SortError> {
        let mut values = (1..=self.items.len())
            .map(|index| {
                self.heap
                    .table_get(self.source, &Value::Number(index as f64))
            })
            .collect::<Result<Vec<_>, _>>()?;
        scope(self, &mut values, insertion_and_commit)
    }

    fn compare(&mut self, left: &Value, right: &Value) -> Result<bool, SortError> {
        // 对应 invoke 的当前参数保护；它没有保护 snapshot 中尚未参与比较的对象。
        let base = self.temporary_roots.len();
        self.temporary_roots.extend([left.clone(), right.clone()]);
        self.comparisons += 1;
        let result = self.comparator_body(left, right);
        self.temporary_roots.truncate(base);
        result
    }

    fn comparator_body(&mut self, left: &Value, right: &Value) -> Result<bool, SortError> {
        match self.scenario {
            Scenario::ClearAndCollect if !self.cleared => {
                self.cleared = true;
                self.clear_source()?;
                self.collect()?;
            }
            Scenario::ErrorAfterMutation if self.comparisons == 2 => {
                self.heap
                    .table_set(self.source, Value::Number(1.0), Value::Number(99.0))?;
                return Err(SortError::Comparator);
            }
            _ => {}
        }
        Ok(self.rank(left)? < self.rank(right)?)
    }
}

fn insertion_and_commit(vm: &mut SortVm, values: &mut [Value]) -> Result<(), SortError> {
    for right in 1..values.len() {
        let mut cursor = right;
        while cursor > 0 {
            if !vm.compare(&values[cursor], &values[cursor - 1])? {
                break;
            }
            values.swap(cursor, cursor - 1);
            cursor -= 1;
        }
    }
    // comparator Err 时不执行这批排序回写，但其已经产生的 source 写入不会回滚。
    for (index, value) in values.iter().enumerate() {
        vm.heap
            .table_set(vm.source, Value::Number((index + 1) as f64), value.clone())?;
    }
    Ok(())
}
