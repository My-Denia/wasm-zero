//! Decoded module IR. Filled in by the decoder (M2); the shape is fixed
//! here so the runner and store can be built against it.

use crate::types::{FuncType, GlobalType, MemType, RefType, TableType, ValType};

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    /// Type indices of locally defined functions.
    pub funcs: Vec<u32>,
    pub tables: Vec<TableType>,
    pub mems: Vec<MemType>,
    pub globals: Vec<Global>,
    pub exports: Vec<Export>,
    pub start: Option<u32>,
    pub elems: Vec<Elem>,
    pub datas: Vec<Data>,
    /// Code bodies, parallel to `funcs`.
    pub codes: Vec<Code>,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub desc: ImportDesc,
}

#[derive(Clone, Copy, Debug)]
pub enum ImportDesc {
    Func(u32),
    Table(TableType),
    Mem(MemType),
    Global(GlobalType),
}

#[derive(Clone, Debug)]
pub struct Global {
    pub ty: GlobalType,
    pub init: ConstExpr,
}

#[derive(Clone, Debug)]
pub struct Export {
    pub name: String,
    pub desc: ExportDesc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportDesc {
    Func(u32),
    Table(u32),
    Mem(u32),
    Global(u32),
}

/// A constant expression (module-level initializer).
#[derive(Clone, Debug)]
pub struct ConstExpr {
    pub instrs: Vec<ConstInstr>,
}

#[derive(Clone, Copy, Debug)]
pub enum ConstInstr {
    I32(u32),
    I64(u64),
    F32(u32),
    F64(u64),
    V128(u128),
    RefNull(RefType),
    RefFunc(u32),
    GlobalGet(u32),
}

#[derive(Clone, Debug)]
pub struct Elem {
    pub ty: RefType,
    pub init: Vec<ConstExpr>,
    pub mode: ElemMode,
}

#[derive(Clone, Debug)]
pub enum ElemMode {
    Active { table: u32, offset: ConstExpr },
    Passive,
    Declarative,
}

#[derive(Clone, Debug)]
pub struct Data {
    pub bytes: Vec<u8>,
    pub mode: DataMode,
}

#[derive(Clone, Debug)]
pub enum DataMode {
    Active { mem: u32, offset: ConstExpr },
    Passive,
}

#[derive(Clone, Debug, Default)]
pub struct Code {
    pub locals: Vec<ValType>,
    pub body: Vec<Instr>,
}

/// Block type immediate.
#[derive(Clone, Copy, Debug)]
pub enum BlockType {
    Empty,
    Val(ValType),
    Type(u32),
}

/// Flat instruction stream with structured markers. Branch/structure
/// targets are resolved by a post-decode pass and stored in immediates:
/// for `Block`/`If`, `end` is the index one past the matching End; for
/// `Loop`, `end` likewise; `If.else_` is the index one past Else (i.e. the
/// start of the else arm) when present.
#[derive(Clone, Debug)]
pub enum Instr {
    Unreachable,
    Nop,
    Block {
        bt: BlockType,
        end: u32,
    },
    Loop {
        bt: BlockType,
        end: u32,
    },
    If {
        bt: BlockType,
        else_: u32,
        end: u32,
    },
    Else {
        end: u32,
    },
    End,
    Br {
        depth: u32,
    },
    BrIf {
        depth: u32,
    },
    BrTable {
        depths: Vec<u32>,
        default: u32,
    },
    Return,
    Call {
        func: u32,
    },
    CallIndirect {
        table: u32,
        ty: u32,
    },

    Drop,
    Select {
        ty: Option<ValType>,
    },

    LocalGet {
        idx: u32,
    },
    LocalSet {
        idx: u32,
    },
    LocalTee {
        idx: u32,
    },
    GlobalGet {
        idx: u32,
    },
    GlobalSet {
        idx: u32,
    },

    TableGet {
        table: u32,
    },
    TableSet {
        table: u32,
    },
    TableInit {
        table: u32,
        elem: u32,
    },
    ElemDrop {
        elem: u32,
    },
    TableCopy {
        dst: u32,
        src: u32,
    },
    TableGrow {
        table: u32,
    },
    TableSize {
        table: u32,
    },
    TableFill {
        table: u32,
    },

    Load {
        op: LoadOp,
        align: u32,
        offset: u32,
    },
    Store {
        op: StoreOp,
        align: u32,
        offset: u32,
    },
    MemorySize,
    MemoryGrow,
    MemoryInit {
        data: u32,
    },
    DataDrop {
        data: u32,
    },
    MemoryCopy,
    MemoryFill,

    I32Const(u32),
    I64Const(u64),
    F32Const(u32),
    F64Const(u64),
    V128Const(u128),

    /// All numeric/reference operators without immediates, keyed by opcode.
    Num(NumOp),
    /// SIMD operators without further immediates.
    Simd(SimdOp),
    /// SIMD operators with a lane immediate.
    SimdLane {
        op: SimdLaneOp,
        lane: u8,
    },
    /// i8x16.shuffle immediate.
    Shuffle([u8; 16]),
    /// v128 loads/stores with memarg (+ optional lane).
    SimdMem {
        op: SimdMemOp,
        align: u32,
        offset: u32,
        lane: u8,
    },

    RefNull(RefType),
    RefIsNull,
    RefFunc {
        func: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadOp {
    I32,
    I64,
    F32,
    F64,
    I32L8S,
    I32L8U,
    I32L16S,
    I32L16U,
    I64L8S,
    I64L8U,
    I64L16S,
    I64L16U,
    I64L32S,
    I64L32U,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOp {
    I32,
    I64,
    F32,
    F64,
    I32S8,
    I32S16,
    I64S8,
    I64S16,
    I64S32,
}

/// Plain numeric operators (no immediates). Names follow the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumOp {
    I32Eqz,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32GtU,
    I32LeS,
    I32LeU,
    I32GeS,
    I32GeU,
    I64Eqz,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LtU,
    I64GtS,
    I64GtU,
    I64LeS,
    I64LeU,
    I64GeS,
    I64GeU,
    F32Eq,
    F32Ne,
    F32Lt,
    F32Gt,
    F32Le,
    F32Ge,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,
    I32Clz,
    I32Ctz,
    I32Popcnt,
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrS,
    I32ShrU,
    I32Rotl,
    I32Rotr,
    I64Clz,
    I64Ctz,
    I64Popcnt,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64DivU,
    I64RemS,
    I64RemU,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrS,
    I64ShrU,
    I64Rotl,
    I64Rotr,
    F32Abs,
    F32Neg,
    F32Ceil,
    F32Floor,
    F32Trunc,
    F32Nearest,
    F32Sqrt,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Min,
    F32Max,
    F32Copysign,
    F64Abs,
    F64Neg,
    F64Ceil,
    F64Floor,
    F64Trunc,
    F64Nearest,
    F64Sqrt,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Min,
    F64Max,
    F64Copysign,
    I32WrapI64,
    I32TruncF32S,
    I32TruncF32U,
    I32TruncF64S,
    I32TruncF64U,
    I64ExtendI32S,
    I64ExtendI32U,
    I64TruncF32S,
    I64TruncF32U,
    I64TruncF64S,
    I64TruncF64U,
    F32ConvertI32S,
    F32ConvertI32U,
    F32ConvertI64S,
    F32ConvertI64U,
    F32DemoteF64,
    F64ConvertI32S,
    F64ConvertI32U,
    F64ConvertI64S,
    F64ConvertI64U,
    F64PromoteF32,
    I32ReinterpretF32,
    I64ReinterpretF64,
    F32ReinterpretI32,
    F64ReinterpretI64,
    I32Extend8S,
    I32Extend16S,
    I64Extend8S,
    I64Extend16S,
    I64Extend32S,
    I32TruncSatF32S,
    I32TruncSatF32U,
    I32TruncSatF64S,
    I32TruncSatF64U,
    I64TruncSatF32S,
    I64TruncSatF32U,
    I64TruncSatF64S,
    I64TruncSatF64U,
}

// SIMD op enums are declared in M2 alongside the decoder; placeholders here
// keep the IR complete from the start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdOp {
    Placeholder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdLaneOp {
    Placeholder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdMemOp {
    Placeholder,
}
