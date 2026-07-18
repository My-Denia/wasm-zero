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

/// A module-level initializer expression. Decoded with the full
/// instruction grammar; constness is enforced by validation (the spec
/// classifies non-constant initializers as invalid, not malformed).
#[derive(Clone, Debug)]
pub struct ConstExpr {
    pub instrs: Vec<Instr>,
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
    /// Typed select as decoded (arity checked in validation).
    SelectT {
        tys: Vec<ValType>,
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

/// SIMD operators without immediates (v128 stack ops). Names follow the spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdOp {
    I8x16Swizzle,
    I8x16Splat,
    I16x8Splat,
    I32x4Splat,
    I64x2Splat,
    F32x4Splat,
    F64x2Splat,
    I8x16Eq,
    I8x16Ne,
    I8x16LtS,
    I8x16LtU,
    I8x16GtS,
    I8x16GtU,
    I8x16LeS,
    I8x16LeU,
    I8x16GeS,
    I8x16GeU,
    I16x8Eq,
    I16x8Ne,
    I16x8LtS,
    I16x8LtU,
    I16x8GtS,
    I16x8GtU,
    I16x8LeS,
    I16x8LeU,
    I16x8GeS,
    I16x8GeU,
    I32x4Eq,
    I32x4Ne,
    I32x4LtS,
    I32x4LtU,
    I32x4GtS,
    I32x4GtU,
    I32x4LeS,
    I32x4LeU,
    I32x4GeS,
    I32x4GeU,
    F32x4Eq,
    F32x4Ne,
    F32x4Lt,
    F32x4Gt,
    F32x4Le,
    F32x4Ge,
    F64x2Eq,
    F64x2Ne,
    F64x2Lt,
    F64x2Gt,
    F64x2Le,
    F64x2Ge,
    V128Not,
    V128And,
    V128Andnot,
    V128Or,
    V128Xor,
    V128Bitselect,
    V128AnyTrue,
    F32x4DemoteF64x2Zero,
    F64x2PromoteLowF32x4,
    I8x16Abs,
    I8x16Neg,
    I8x16Popcnt,
    I8x16AllTrue,
    I8x16Bitmask,
    I8x16NarrowI16x8S,
    I8x16NarrowI16x8U,
    F32x4Ceil,
    F32x4Floor,
    F32x4Trunc,
    F32x4Nearest,
    I8x16Shl,
    I8x16ShrS,
    I8x16ShrU,
    I8x16Add,
    I8x16AddSatS,
    I8x16AddSatU,
    I8x16Sub,
    I8x16SubSatS,
    I8x16SubSatU,
    F64x2Ceil,
    F64x2Floor,
    I8x16MinS,
    I8x16MinU,
    I8x16MaxS,
    I8x16MaxU,
    F64x2Trunc,
    I8x16AvgrU,
    I16x8ExtaddPairwiseI8x16S,
    I16x8ExtaddPairwiseI8x16U,
    I32x4ExtaddPairwiseI16x8S,
    I32x4ExtaddPairwiseI16x8U,
    I16x8Abs,
    I16x8Neg,
    I16x8Q15mulrSatS,
    I16x8AllTrue,
    I16x8Bitmask,
    I16x8NarrowI32x4S,
    I16x8NarrowI32x4U,
    I16x8ExtendLowI8x16S,
    I16x8ExtendHighI8x16S,
    I16x8ExtendLowI8x16U,
    I16x8ExtendHighI8x16U,
    I16x8Shl,
    I16x8ShrS,
    I16x8ShrU,
    I16x8Add,
    I16x8AddSatS,
    I16x8AddSatU,
    I16x8Sub,
    I16x8SubSatS,
    I16x8SubSatU,
    F64x2Nearest,
    I16x8Mul,
    I16x8MinS,
    I16x8MinU,
    I16x8MaxS,
    I16x8MaxU,
    I16x8AvgrU,
    I16x8ExtmulLowI8x16S,
    I16x8ExtmulHighI8x16S,
    I16x8ExtmulLowI8x16U,
    I16x8ExtmulHighI8x16U,
    I32x4Abs,
    I32x4Neg,
    I32x4AllTrue,
    I32x4Bitmask,
    I32x4ExtendLowI16x8S,
    I32x4ExtendHighI16x8S,
    I32x4ExtendLowI16x8U,
    I32x4ExtendHighI16x8U,
    I32x4Shl,
    I32x4ShrS,
    I32x4ShrU,
    I32x4Add,
    I32x4Sub,
    I32x4Mul,
    I32x4MinS,
    I32x4MinU,
    I32x4MaxS,
    I32x4MaxU,
    I32x4DotI16x8S,
    I32x4ExtmulLowI16x8S,
    I32x4ExtmulHighI16x8S,
    I32x4ExtmulLowI16x8U,
    I32x4ExtmulHighI16x8U,
    I64x2Abs,
    I64x2Neg,
    I64x2AllTrue,
    I64x2Bitmask,
    I64x2ExtendLowI32x4S,
    I64x2ExtendHighI32x4S,
    I64x2ExtendLowI32x4U,
    I64x2ExtendHighI32x4U,
    I64x2Shl,
    I64x2ShrS,
    I64x2ShrU,
    I64x2Add,
    I64x2Sub,
    I64x2Mul,
    I64x2Eq,
    I64x2Ne,
    I64x2LtS,
    I64x2GtS,
    I64x2LeS,
    I64x2GeS,
    I64x2ExtmulLowI32x4S,
    I64x2ExtmulHighI32x4S,
    I64x2ExtmulLowI32x4U,
    I64x2ExtmulHighI32x4U,
    F32x4Abs,
    F32x4Neg,
    F32x4Sqrt,
    F32x4Add,
    F32x4Sub,
    F32x4Mul,
    F32x4Div,
    F32x4Min,
    F32x4Max,
    F32x4Pmin,
    F32x4Pmax,
    F64x2Abs,
    F64x2Neg,
    F64x2Sqrt,
    F64x2Add,
    F64x2Sub,
    F64x2Mul,
    F64x2Div,
    F64x2Min,
    F64x2Max,
    F64x2Pmin,
    F64x2Pmax,
    I32x4TruncSatF32x4S,
    I32x4TruncSatF32x4U,
    F32x4ConvertI32x4S,
    F32x4ConvertI32x4U,
    I32x4TruncSatF64x2SZero,
    I32x4TruncSatF64x2UZero,
    F64x2ConvertLowI32x4S,
    F64x2ConvertLowI32x4U,
}

/// SIMD operators with a lane immediate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdLaneOp {
    I8x16ExtractLaneS,
    I8x16ExtractLaneU,
    I8x16ReplaceLane,
    I16x8ExtractLaneS,
    I16x8ExtractLaneU,
    I16x8ReplaceLane,
    I32x4ExtractLane,
    I32x4ReplaceLane,
    I64x2ExtractLane,
    I64x2ReplaceLane,
    F32x4ExtractLane,
    F32x4ReplaceLane,
    F64x2ExtractLane,
    F64x2ReplaceLane,
}

/// v128 memory operators (memarg immediate; `*Lane` also carry a lane).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SimdMemOp {
    Load,
    Load8x8S,
    Load8x8U,
    Load16x4S,
    Load16x4U,
    Load32x2S,
    Load32x2U,
    Load8Splat,
    Load16Splat,
    Load32Splat,
    Load64Splat,
    Load32Zero,
    Load64Zero,
    Store,
    Load8Lane,
    Load16Lane,
    Load32Lane,
    Load64Lane,
    Store8Lane,
    Store16Lane,
    Store32Lane,
    Store64Lane,
}
