//! Runtime values. Floats are stored as raw bit patterns so that NaN
//! payloads survive loads/stores/locals exactly as the spec requires.

use crate::types::ValType;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Value {
    I32(u32),
    I64(u64),
    /// f32 as raw bits.
    F32(u32),
    /// f64 as raw bits.
    F64(u64),
    /// v128 as 16 little-endian bytes.
    V128(u128),
    /// Function reference: store-level function address, or null.
    FuncRef(Option<u32>),
    /// Extern reference: host-provided identity, or null.
    ExternRef(Option<u32>),
}

impl Value {
    pub fn ty(&self) -> ValType {
        match self {
            Value::I32(_) => ValType::I32,
            Value::I64(_) => ValType::I64,
            Value::F32(_) => ValType::F32,
            Value::F64(_) => ValType::F64,
            Value::V128(_) => ValType::V128,
            Value::FuncRef(_) => ValType::FuncRef,
            Value::ExternRef(_) => ValType::ExternRef,
        }
    }

    pub fn default_for(ty: ValType) -> Value {
        match ty {
            ValType::I32 => Value::I32(0),
            ValType::I64 => Value::I64(0),
            ValType::F32 => Value::F32(0),
            ValType::F64 => Value::F64(0),
            ValType::V128 => Value::V128(0),
            ValType::FuncRef => Value::FuncRef(None),
            ValType::ExternRef => Value::ExternRef(None),
        }
    }
}
