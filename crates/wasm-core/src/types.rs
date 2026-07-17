//! Wasm 2.0 type grammar.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    FuncRef,
    ExternRef,
}

impl ValType {
    pub fn is_num(self) -> bool {
        matches!(
            self,
            ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
        )
    }
    pub fn is_ref(self) -> bool {
        matches!(self, ValType::FuncRef | ValType::ExternRef)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefType {
    FuncRef,
    ExternRef,
}

impl From<RefType> for ValType {
    fn from(r: RefType) -> ValType {
        match r {
            RefType::FuncRef => ValType::FuncRef,
            RefType::ExternRef => ValType::ExternRef,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Limits {
    pub min: u32,
    pub max: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TableType {
    pub elem: RefType,
    pub limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MemType {
    pub limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlobalType {
    pub val: ValType,
    pub mutable: bool,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ExternType {
    Func(FuncType),
    Table(TableType),
    Mem(MemType),
    Global(GlobalType),
}
