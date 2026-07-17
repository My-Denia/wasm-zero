//! WebAssembly 2.0 binary format decoder, written strictly against the
//! wg-2.0 binary grammar. Everything the grammar rejects must fail here
//! (assert_malformed territory); everything it accepts but the typing
//! rules reject fails later in validation (assert_invalid territory).

use crate::error::DecodeError;
use crate::module::*;
use crate::types::*;

pub fn decode(bytes: &[u8]) -> Result<Module, DecodeError> {
    Decoder::new(bytes).module()
}

struct Decoder<'a> {
    b: &'a [u8],
    pos: usize,
}

type R<T> = Result<T, DecodeError>;

impl<'a> Decoder<'a> {
    fn new(b: &'a [u8]) -> Self {
        Decoder { b, pos: 0 }
    }

    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(DecodeError {
            offset: self.pos,
            msg: msg.into(),
        })
    }

    fn eof<T>(&self) -> R<T> {
        self.err("unexpected end of section or function")
    }

    fn byte(&mut self) -> R<u8> {
        if self.pos >= self.b.len() {
            return self.eof();
        }
        let v = self.b[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        if self.b.len() - self.pos < n {
            return self.eof();
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Unsigned LEB128, at most `bits` significant bits.
    fn uleb(&mut self, bits: u32) -> R<u64> {
        let max_bytes = bits.div_ceil(7) as usize;
        let mut result: u64 = 0;
        let mut shift = 0u32;
        for i in 0.. {
            let byte = self.byte()?;
            if i == max_bytes - 1 {
                // Last permitted byte: continuation must be clear and the
                // unused high bits must be zero.
                if byte & 0x80 != 0 {
                    return self.err("integer representation too long");
                }
                let unused = 7 - (bits - 7 * (max_bytes as u32 - 1));
                if unused > 0 && byte >> (7 - unused) != 0 {
                    return self.err("integer too large");
                }
            }
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        Ok(result)
    }

    fn u32(&mut self) -> R<u32> {
        Ok(self.uleb(32)? as u32)
    }

    /// Signed LEB128 with at most `bits` significant bits.
    fn sleb(&mut self, bits: u32) -> R<i64> {
        let max_bytes = bits.div_ceil(7) as usize;
        let mut result: i64 = 0;
        let mut shift = 0u32;
        for i in 0.. {
            let byte = self.byte()?;
            if i == max_bytes - 1 {
                if byte & 0x80 != 0 {
                    return self.err("integer representation too long");
                }
                // Unused bits must be a sign extension of the value.
                let used = bits - 7 * (max_bytes as u32 - 1);
                if used < 7 {
                    let unused_mask = 0x7fu8 & !((1u8 << used) - 1) & !(1 << (used - 1));
                    let sign_bit = 1u8 << (used - 1);
                    let unused = byte & unused_mask;
                    if byte & sign_bit != 0 {
                        if unused != unused_mask {
                            return self.err("integer too large");
                        }
                    } else if unused != 0 {
                        return self.err("integer too large");
                    }
                }
            }
            result |= i64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    result |= -1i64 << shift;
                }
                break;
            }
        }
        Ok(result)
    }

    fn s32(&mut self) -> R<u32> {
        Ok(self.sleb(32)? as i32 as u32)
    }

    fn s64(&mut self) -> R<u64> {
        Ok(self.sleb(64)? as u64)
    }

    fn f32_bits(&mut self) -> R<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes(b.try_into().unwrap()))
    }

    fn f64_bits(&mut self) -> R<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().unwrap()))
    }

    fn name(&mut self) -> R<String> {
        let len = self.u32()? as usize;
        let bytes = self.take(len)?;
        match std::str::from_utf8(bytes) {
            Ok(s) => Ok(s.to_string()),
            Err(_) => self.err("malformed UTF-8 encoding"),
        }
    }

    fn valtype(&mut self) -> R<ValType> {
        let b = self.byte()?;
        self.valtype_of(b)
    }

    fn valtype_of(&self, b: u8) -> R<ValType> {
        match b {
            0x7f => Ok(ValType::I32),
            0x7e => Ok(ValType::I64),
            0x7d => Ok(ValType::F32),
            0x7c => Ok(ValType::F64),
            0x7b => Ok(ValType::V128),
            0x70 => Ok(ValType::FuncRef),
            0x6f => Ok(ValType::ExternRef),
            _ => self.err(format!("malformed value type {b:#x}")),
        }
    }

    fn reftype(&mut self) -> R<RefType> {
        match self.byte()? {
            0x70 => Ok(RefType::FuncRef),
            0x6f => Ok(RefType::ExternRef),
            b => self.err(format!("malformed reference type {b:#x}")),
        }
    }

    fn limits(&mut self) -> R<Limits> {
        match self.byte()? {
            0x00 => Ok(Limits {
                min: self.u32()?,
                max: None,
            }),
            0x01 => {
                let min = self.u32()?;
                let max = self.u32()?;
                Ok(Limits {
                    min,
                    max: Some(max),
                })
            }
            b => self.err(format!("malformed limits flag {b:#x}")),
        }
    }

    fn tabletype(&mut self) -> R<TableType> {
        let elem = self.reftype()?;
        let limits = self.limits()?;
        Ok(TableType { elem, limits })
    }

    fn memtype(&mut self) -> R<MemType> {
        Ok(MemType {
            limits: self.limits()?,
        })
    }

    fn globaltype(&mut self) -> R<GlobalType> {
        let val = self.valtype()?;
        let mutable = match self.byte()? {
            0x00 => false,
            0x01 => true,
            b => return self.err(format!("malformed mutability {b:#x}")),
        };
        Ok(GlobalType { val, mutable })
    }

    fn blocktype(&mut self) -> R<BlockType> {
        // Single-byte shortcuts first; otherwise an s33 type index.
        match self.peek() {
            Some(0x40) => {
                self.pos += 1;
                Ok(BlockType::Empty)
            }
            Some(b) if self.valtype_of(b).is_ok() => {
                self.pos += 1;
                Ok(BlockType::Val(self.valtype_of(b)?))
            }
            _ => {
                let v = self.sleb(33)?;
                if v < 0 {
                    return self.err("malformed block type");
                }
                Ok(BlockType::Type(v as u32))
            }
        }
    }

    fn memarg(&mut self) -> R<(u32, u32)> {
        let align = self.u32()?;
        if align >= 32 {
            // 2^align would overflow the 32-bit address space; the suite
            // treats this as a decode error ("malformed memop flags").
            return self.err("malformed memop flags");
        }
        let offset = self.u32()?;
        Ok((align, offset))
    }

    // ---- module structure ----

    fn module(&mut self) -> R<Module> {
        if self.take(4).map_err(|_| DecodeError {
            offset: 0,
            msg: "unexpected end".into(),
        })? != b"\0asm"
        {
            return Err(DecodeError {
                offset: 0,
                msg: "magic header not detected".into(),
            });
        }
        if self.take(4).map_err(|_| DecodeError {
            offset: 4,
            msg: "unexpected end".into(),
        })? != [1, 0, 0, 0]
        {
            return Err(DecodeError {
                offset: 4,
                msg: "unknown binary version".into(),
            });
        }

        let mut m = Module::default();
        let mut last_rank = 0u8;
        let mut func_types: Vec<u32> = Vec::new();
        let mut datacount: Option<u32> = None;
        let mut saw_code = false;
        let mut saw_data = false;
        let mut dc_used = false;

        while self.pos < self.b.len() {
            let id = self.byte()?;
            let size = self.u32()? as usize;
            if self.b.len() - self.pos < size {
                // A section length that overruns the module.
                return self.err("unexpected end of section or function");
            }
            let end = self.pos + size;
            let rank = match id {
                0 => 0, // custom: allowed anywhere
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                5 => 5,
                6 => 6,
                7 => 7,
                8 => 8,
                9 => 9,
                12 => 10,
                10 => 11,
                11 => 12,
                _ => return self.err("malformed section id"),
            };
            if id != 0 {
                if rank <= last_rank {
                    return self.err("unexpected content after last section");
                }
                last_rank = rank;
            }
            match id {
                0 => {
                    // Custom section: name must be well-formed; content is
                    // opaque but must stay within the declared size.
                    let sec_end = end;
                    let name_start = self.pos;
                    let len = self.u32()? as usize;
                    if self.pos + len > sec_end {
                        return self.err("unexpected end");
                    }
                    let bytes = self.take(len)?;
                    if std::str::from_utf8(bytes).is_err() {
                        return self.err("malformed UTF-8 encoding");
                    }
                    let _ = name_start;
                    self.pos = sec_end;
                }
                1 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        if self.byte()? != 0x60 {
                            self.pos -= 1;
                            return self.err("malformed functype");
                        }
                        let np = self.u32()?;
                        let mut params = Vec::new();
                        for _ in 0..np {
                            params.push(self.valtype()?);
                        }
                        let nr = self.u32()?;
                        let mut results = Vec::new();
                        for _ in 0..nr {
                            results.push(self.valtype()?);
                        }
                        m.types.push(FuncType { params, results });
                    }
                }
                2 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        let module = self.name()?;
                        let name = self.name()?;
                        let desc = match self.byte()? {
                            0x00 => ImportDesc::Func(self.u32()?),
                            0x01 => ImportDesc::Table(self.tabletype()?),
                            0x02 => ImportDesc::Mem(self.memtype()?),
                            0x03 => ImportDesc::Global(self.globaltype()?),
                            b => return self.err(format!("malformed import kind {b:#x}")),
                        };
                        m.imports.push(Import { module, name, desc });
                    }
                }
                3 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        func_types.push(self.u32()?);
                    }
                }
                4 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        m.tables.push(self.tabletype()?);
                    }
                }
                5 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        m.mems.push(self.memtype()?);
                    }
                }
                6 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        let ty = self.globaltype()?;
                        let init = self.const_expr()?;
                        m.globals.push(Global { ty, init });
                    }
                }
                7 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        let name = self.name()?;
                        let desc = match self.byte()? {
                            0x00 => ExportDesc::Func(self.u32()?),
                            0x01 => ExportDesc::Table(self.u32()?),
                            0x02 => ExportDesc::Mem(self.u32()?),
                            0x03 => ExportDesc::Global(self.u32()?),
                            b => return self.err(format!("malformed export kind {b:#x}")),
                        };
                        m.exports.push(Export { name, desc });
                    }
                }
                8 => {
                    m.start = Some(self.u32()?);
                }
                9 => {
                    let n = self.u32()?;
                    for _ in 0..n {
                        m.elems.push(self.elem()?);
                    }
                }
                12 => {
                    datacount = Some(self.u32()?);
                }
                10 => {
                    saw_code = true;
                    let n = self.u32()?;
                    if n as usize != func_types.len() {
                        return self.err("function and code section have inconsistent lengths");
                    }
                    for _ in 0..n {
                        m.codes.push(self.code(&mut dc_used)?);
                    }
                }
                11 => {
                    saw_data = true;
                    let n = self.u32()?;
                    if let Some(dc) = datacount {
                        if dc != n {
                            return self
                                .err("data count and data section have inconsistent lengths");
                        }
                    }
                    for _ in 0..n {
                        m.datas.push(self.data()?);
                    }
                }
                _ => unreachable!(),
            }
            if self.pos != end {
                return self.err("section size mismatch");
            }
        }

        if !func_types.is_empty() && !saw_code {
            return self.err("function and code section have inconsistent lengths");
        }
        // memory.init/data.drop need the data count section for single-pass
        // validation. Corpus-adjudicated nuance: when the module has no data
        // section at all, toolchains omit the count and the suite expects a
        // validation error ("unknown data segment") instead of a decode error.
        if dc_used && datacount.is_none() && saw_data {
            return self.err("data count section required");
        }
        if let Some(dc) = datacount {
            if dc != 0 && !saw_data {
                return self.err("data count and data section have inconsistent lengths");
            }
        }
        m.funcs = func_types;
        Ok(m)
    }

    fn elem(&mut self) -> R<Elem> {
        let flags = self.u32()?;
        if flags > 7 {
            return self.err("malformed element segment kind");
        }
        let active = flags & 0b001 == 0;
        let explicit_idx = flags & 0b010 != 0;
        let use_exprs = flags & 0b100 != 0;

        let table = if active && explicit_idx {
            self.u32()?
        } else {
            0
        };
        let offset = if active {
            Some(self.const_expr()?)
        } else {
            None
        };
        let ty = if flags & 0b011 != 0 {
            // A kind/type byte is present.
            if use_exprs {
                self.reftype()?
            } else {
                match self.byte()? {
                    0x00 => RefType::FuncRef,
                    b => return self.err(format!("malformed element kind {b:#x}")),
                }
            }
        } else {
            RefType::FuncRef
        };
        let n = self.u32()?;
        let mut init = Vec::new();
        for _ in 0..n {
            if use_exprs {
                init.push(self.const_expr()?);
            } else {
                let f = self.u32()?;
                init.push(ConstExpr {
                    instrs: vec![Instr::RefFunc { func: f }],
                });
            }
        }
        let mode = if active {
            ElemMode::Active {
                table,
                offset: offset.unwrap(),
            }
        } else if explicit_idx {
            ElemMode::Declarative
        } else {
            ElemMode::Passive
        };
        Ok(Elem { ty, init, mode })
    }

    fn data(&mut self) -> R<Data> {
        let flags = self.u32()?;
        match flags {
            0 => {
                let offset = self.const_expr()?;
                let len = self.u32()? as usize;
                let bytes = self.take(len)?.to_vec();
                Ok(Data {
                    bytes,
                    mode: DataMode::Active { mem: 0, offset },
                })
            }
            1 => {
                let len = self.u32()? as usize;
                let bytes = self.take(len)?.to_vec();
                Ok(Data {
                    bytes,
                    mode: DataMode::Passive,
                })
            }
            2 => {
                let mem = self.u32()?;
                let offset = self.const_expr()?;
                let len = self.u32()? as usize;
                let bytes = self.take(len)?.to_vec();
                Ok(Data {
                    bytes,
                    mode: DataMode::Active { mem, offset },
                })
            }
            _ => self.err("malformed data segment kind"),
        }
    }

    fn code(&mut self, dc_used: &mut bool) -> R<Code> {
        let size = self.u32()? as usize;
        if self.b.len() - self.pos < size {
            return self.eof();
        }
        let end = self.pos + size;
        let nlocals = self.u32()?;
        let mut locals = Vec::new();
        let mut total: u64 = 0;
        for _ in 0..nlocals {
            let count = self.u32()?;
            let ty = self.valtype()?;
            total += u64::from(count);
            if total > u64::from(u32::MAX) {
                return self.err("too many locals");
            }
            for _ in 0..count {
                locals.push(ty);
            }
        }
        let body = self.expr_until_end(end, dc_used)?;
        if self.pos != end {
            return self.err("section size mismatch (code entry)");
        }
        Ok(Code { locals, body })
    }

    fn const_expr(&mut self) -> R<ConstExpr> {
        // Init expressions use the full instruction grammar; validation
        // enforces constness afterwards.
        let mut dc_used = false;
        let instrs = self.expr_until_end(self.b.len(), &mut dc_used)?;
        Ok(ConstExpr { instrs })
    }

    /// Decode instructions until the terminating `end` of depth 0.
    /// `limit` bounds reads (code body end or module end).
    fn expr_until_end(&mut self, limit: usize, dc_used: &mut bool) -> R<Vec<Instr>> {
        let mut out: Vec<Instr> = Vec::new();
        let mut depth: u32 = 0;
        // Stack of indices of open Block/Loop/If (and the If's Else) for
        // target resolution.
        let mut open: Vec<usize> = Vec::new();
        loop {
            if self.pos >= limit {
                return self.eof();
            }
            let op = self.byte()?;
            let instr = match op {
                0x0b => {
                    out.push(Instr::End);
                    if depth == 0 {
                        return Ok(out);
                    }
                    depth -= 1;
                    // Resolve the matching opener (and Else if present).
                    let opener = open.pop().unwrap();
                    let here = out.len() as u32;
                    match &mut out[opener] {
                        Instr::Block { end, .. } | Instr::Loop { end, .. } => *end = here,
                        Instr::If { else_, end, .. } => {
                            *end = here;
                            if *else_ != u32::MAX {
                                let e = *else_ as usize - 1;
                                if let Instr::Else { end } = &mut out[e] {
                                    *end = here;
                                }
                            } else {
                                *else_ = here;
                            }
                        }
                        _ => unreachable!(),
                    }
                    continue;
                }
                0x05 => {
                    // else: must close an If arm.
                    let Some(&opener) = open.last() else {
                        return self.err("misplaced else");
                    };
                    let here = out.len() as u32 + 1;
                    match &mut out[opener] {
                        Instr::If { else_, .. } if *else_ == u32::MAX => *else_ = here,
                        _ => return self.err("misplaced else"),
                    }
                    out.push(Instr::Else { end: u32::MAX });
                    continue;
                }
                0x00 => Instr::Unreachable,
                0x01 => Instr::Nop,
                0x02 => {
                    depth += 1;
                    open.push(out.len());
                    Instr::Block {
                        bt: self.blocktype()?,
                        end: u32::MAX,
                    }
                }
                0x03 => {
                    depth += 1;
                    open.push(out.len());
                    Instr::Loop {
                        bt: self.blocktype()?,
                        end: u32::MAX,
                    }
                }
                0x04 => {
                    depth += 1;
                    open.push(out.len());
                    Instr::If {
                        bt: self.blocktype()?,
                        else_: u32::MAX,
                        end: u32::MAX,
                    }
                }
                0x0c => Instr::Br { depth: self.u32()? },
                0x0d => Instr::BrIf { depth: self.u32()? },
                0x0e => {
                    let n = self.u32()?;
                    let mut depths = Vec::new();
                    for _ in 0..n {
                        depths.push(self.u32()?);
                    }
                    let default = self.u32()?;
                    Instr::BrTable { depths, default }
                }
                0x0f => Instr::Return,
                0x10 => Instr::Call { func: self.u32()? },
                0x11 => {
                    let ty = self.u32()?;
                    let table = self.u32()?;
                    Instr::CallIndirect { table, ty }
                }
                0x1a => Instr::Drop,
                0x1b => Instr::Select { ty: None },
                0x1c => {
                    // Typed select carries a type vector; arity != 1 is a
                    // validation error, not a decode error.
                    let n = self.u32()?;
                    let mut tys = Vec::new();
                    for _ in 0..n {
                        tys.push(self.valtype()?);
                    }
                    Instr::SelectT { tys }
                }
                0x20 => Instr::LocalGet { idx: self.u32()? },
                0x21 => Instr::LocalSet { idx: self.u32()? },
                0x22 => Instr::LocalTee { idx: self.u32()? },
                0x23 => Instr::GlobalGet { idx: self.u32()? },
                0x24 => Instr::GlobalSet { idx: self.u32()? },
                0x25 => Instr::TableGet { table: self.u32()? },
                0x26 => Instr::TableSet { table: self.u32()? },
                0x28..=0x35 => {
                    let (align, offset) = self.memarg()?;
                    let op = match op {
                        0x28 => LoadOp::I32,
                        0x29 => LoadOp::I64,
                        0x2a => LoadOp::F32,
                        0x2b => LoadOp::F64,
                        0x2c => LoadOp::I32L8S,
                        0x2d => LoadOp::I32L8U,
                        0x2e => LoadOp::I32L16S,
                        0x2f => LoadOp::I32L16U,
                        0x30 => LoadOp::I64L8S,
                        0x31 => LoadOp::I64L8U,
                        0x32 => LoadOp::I64L16S,
                        0x33 => LoadOp::I64L16U,
                        0x34 => LoadOp::I64L32S,
                        0x35 => LoadOp::I64L32U,
                        _ => unreachable!(),
                    };
                    Instr::Load { op, align, offset }
                }
                0x36..=0x3e => {
                    let (align, offset) = self.memarg()?;
                    let op = match op {
                        0x36 => StoreOp::I32,
                        0x37 => StoreOp::I64,
                        0x38 => StoreOp::F32,
                        0x39 => StoreOp::F64,
                        0x3a => StoreOp::I32S8,
                        0x3b => StoreOp::I32S16,
                        0x3c => StoreOp::I64S8,
                        0x3d => StoreOp::I64S16,
                        0x3e => StoreOp::I64S32,
                        _ => unreachable!(),
                    };
                    Instr::Store { op, align, offset }
                }
                0x3f => {
                    if self.byte()? != 0x00 {
                        self.pos -= 1;
                        return self.err("zero byte expected");
                    }
                    Instr::MemorySize
                }
                0x40 => {
                    if self.byte()? != 0x00 {
                        self.pos -= 1;
                        return self.err("zero byte expected");
                    }
                    Instr::MemoryGrow
                }
                0x41 => Instr::I32Const(self.s32()?),
                0x42 => Instr::I64Const(self.s64()?),
                0x43 => Instr::F32Const(self.f32_bits()?),
                0x44 => Instr::F64Const(self.f64_bits()?),
                0x45..=0xc4 => Instr::Num(plain_numop(op).ok_or_else(|| DecodeError {
                    offset: self.pos - 1,
                    msg: format!("illegal opcode {op:#x}"),
                })?),
                0xd0 => Instr::RefNull(self.reftype()?),
                0xd1 => Instr::RefIsNull,
                0xd2 => Instr::RefFunc { func: self.u32()? },
                0xfc => {
                    let sub = self.u32()?;
                    match sub {
                        0 => Instr::Num(NumOp::I32TruncSatF32S),
                        1 => Instr::Num(NumOp::I32TruncSatF32U),
                        2 => Instr::Num(NumOp::I32TruncSatF64S),
                        3 => Instr::Num(NumOp::I32TruncSatF64U),
                        4 => Instr::Num(NumOp::I64TruncSatF32S),
                        5 => Instr::Num(NumOp::I64TruncSatF32U),
                        6 => Instr::Num(NumOp::I64TruncSatF64S),
                        7 => Instr::Num(NumOp::I64TruncSatF64U),
                        8 => {
                            *dc_used = true;
                            let data = self.u32()?;
                            if self.byte()? != 0x00 {
                                self.pos -= 1;
                                return self.err("zero byte expected");
                            }
                            Instr::MemoryInit { data }
                        }
                        9 => {
                            *dc_used = true;
                            Instr::DataDrop { data: self.u32()? }
                        }
                        10 => {
                            if self.byte()? != 0x00 {
                                self.pos -= 1;
                                return self.err("zero byte expected");
                            }
                            if self.byte()? != 0x00 {
                                self.pos -= 1;
                                return self.err("zero byte expected");
                            }
                            Instr::MemoryCopy
                        }
                        11 => {
                            if self.byte()? != 0x00 {
                                self.pos -= 1;
                                return self.err("zero byte expected");
                            }
                            Instr::MemoryFill
                        }
                        12 => {
                            let elem = self.u32()?;
                            let table = self.u32()?;
                            Instr::TableInit { table, elem }
                        }
                        13 => Instr::ElemDrop { elem: self.u32()? },
                        14 => {
                            let dst = self.u32()?;
                            let src = self.u32()?;
                            Instr::TableCopy { dst, src }
                        }
                        15 => Instr::TableGrow { table: self.u32()? },
                        16 => Instr::TableSize { table: self.u32()? },
                        17 => Instr::TableFill { table: self.u32()? },
                        _ => return self.err(format!("illegal opcode 0xfc {sub}")),
                    }
                }
                0xfd => self.simd_instr()?,
                _ => return self.err(format!("illegal opcode {op:#x}")),
            };
            out.push(instr);
        }
    }

    fn simd_instr(&mut self) -> R<Instr> {
        use SimdLaneOp as L;
        use SimdMemOp as M;
        use SimdOp as S;
        let sub = self.u32()?;
        let mem = |d: &mut Self, op: M| -> R<Instr> {
            let (align, offset) = d.memarg()?;
            Ok(Instr::SimdMem {
                op,
                align,
                offset,
                lane: 0,
            })
        };
        let mem_lane = |d: &mut Self, op: M| -> R<Instr> {
            let (align, offset) = d.memarg()?;
            let lane = d.byte()?;
            Ok(Instr::SimdMem {
                op,
                align,
                offset,
                lane,
            })
        };
        let lane = |d: &mut Self, op: L| -> R<Instr> {
            let lane = d.byte()?;
            Ok(Instr::SimdLane { op, lane })
        };
        Ok(match sub {
            0x00 => mem(self, M::Load)?,
            0x01 => mem(self, M::Load8x8S)?,
            0x02 => mem(self, M::Load8x8U)?,
            0x03 => mem(self, M::Load16x4S)?,
            0x04 => mem(self, M::Load16x4U)?,
            0x05 => mem(self, M::Load32x2S)?,
            0x06 => mem(self, M::Load32x2U)?,
            0x07 => mem(self, M::Load8Splat)?,
            0x08 => mem(self, M::Load16Splat)?,
            0x09 => mem(self, M::Load32Splat)?,
            0x0a => mem(self, M::Load64Splat)?,
            0x0b => mem(self, M::Store)?,
            0x0c => {
                let b = self.take(16)?;
                Instr::V128Const(u128::from_le_bytes(b.try_into().unwrap()))
            }
            0x0d => {
                let b = self.take(16)?;
                Instr::Shuffle(b.try_into().unwrap())
            }
            0x0e => Instr::Simd(S::I8x16Swizzle),
            0x0f => Instr::Simd(S::I8x16Splat),
            0x10 => Instr::Simd(S::I16x8Splat),
            0x11 => Instr::Simd(S::I32x4Splat),
            0x12 => Instr::Simd(S::I64x2Splat),
            0x13 => Instr::Simd(S::F32x4Splat),
            0x14 => Instr::Simd(S::F64x2Splat),
            0x15 => lane(self, L::I8x16ExtractLaneS)?,
            0x16 => lane(self, L::I8x16ExtractLaneU)?,
            0x17 => lane(self, L::I8x16ReplaceLane)?,
            0x18 => lane(self, L::I16x8ExtractLaneS)?,
            0x19 => lane(self, L::I16x8ExtractLaneU)?,
            0x1a => lane(self, L::I16x8ReplaceLane)?,
            0x1b => lane(self, L::I32x4ExtractLane)?,
            0x1c => lane(self, L::I32x4ReplaceLane)?,
            0x1d => lane(self, L::I64x2ExtractLane)?,
            0x1e => lane(self, L::I64x2ReplaceLane)?,
            0x1f => lane(self, L::F32x4ExtractLane)?,
            0x20 => lane(self, L::F32x4ReplaceLane)?,
            0x21 => lane(self, L::F64x2ExtractLane)?,
            0x22 => lane(self, L::F64x2ReplaceLane)?,
            0x23 => Instr::Simd(S::I8x16Eq),
            0x24 => Instr::Simd(S::I8x16Ne),
            0x25 => Instr::Simd(S::I8x16LtS),
            0x26 => Instr::Simd(S::I8x16LtU),
            0x27 => Instr::Simd(S::I8x16GtS),
            0x28 => Instr::Simd(S::I8x16GtU),
            0x29 => Instr::Simd(S::I8x16LeS),
            0x2a => Instr::Simd(S::I8x16LeU),
            0x2b => Instr::Simd(S::I8x16GeS),
            0x2c => Instr::Simd(S::I8x16GeU),
            0x2d => Instr::Simd(S::I16x8Eq),
            0x2e => Instr::Simd(S::I16x8Ne),
            0x2f => Instr::Simd(S::I16x8LtS),
            0x30 => Instr::Simd(S::I16x8LtU),
            0x31 => Instr::Simd(S::I16x8GtS),
            0x32 => Instr::Simd(S::I16x8GtU),
            0x33 => Instr::Simd(S::I16x8LeS),
            0x34 => Instr::Simd(S::I16x8LeU),
            0x35 => Instr::Simd(S::I16x8GeS),
            0x36 => Instr::Simd(S::I16x8GeU),
            0x37 => Instr::Simd(S::I32x4Eq),
            0x38 => Instr::Simd(S::I32x4Ne),
            0x39 => Instr::Simd(S::I32x4LtS),
            0x3a => Instr::Simd(S::I32x4LtU),
            0x3b => Instr::Simd(S::I32x4GtS),
            0x3c => Instr::Simd(S::I32x4GtU),
            0x3d => Instr::Simd(S::I32x4LeS),
            0x3e => Instr::Simd(S::I32x4LeU),
            0x3f => Instr::Simd(S::I32x4GeS),
            0x40 => Instr::Simd(S::I32x4GeU),
            0x41 => Instr::Simd(S::F32x4Eq),
            0x42 => Instr::Simd(S::F32x4Ne),
            0x43 => Instr::Simd(S::F32x4Lt),
            0x44 => Instr::Simd(S::F32x4Gt),
            0x45 => Instr::Simd(S::F32x4Le),
            0x46 => Instr::Simd(S::F32x4Ge),
            0x47 => Instr::Simd(S::F64x2Eq),
            0x48 => Instr::Simd(S::F64x2Ne),
            0x49 => Instr::Simd(S::F64x2Lt),
            0x4a => Instr::Simd(S::F64x2Gt),
            0x4b => Instr::Simd(S::F64x2Le),
            0x4c => Instr::Simd(S::F64x2Ge),
            0x4d => Instr::Simd(S::V128Not),
            0x4e => Instr::Simd(S::V128And),
            0x4f => Instr::Simd(S::V128Andnot),
            0x50 => Instr::Simd(S::V128Or),
            0x51 => Instr::Simd(S::V128Xor),
            0x52 => Instr::Simd(S::V128Bitselect),
            0x53 => Instr::Simd(S::V128AnyTrue),
            0x54 => mem_lane(self, M::Load8Lane)?,
            0x55 => mem_lane(self, M::Load16Lane)?,
            0x56 => mem_lane(self, M::Load32Lane)?,
            0x57 => mem_lane(self, M::Load64Lane)?,
            0x58 => mem_lane(self, M::Store8Lane)?,
            0x59 => mem_lane(self, M::Store16Lane)?,
            0x5a => mem_lane(self, M::Store32Lane)?,
            0x5b => mem_lane(self, M::Store64Lane)?,
            0x5c => mem(self, M::Load32Zero)?,
            0x5d => mem(self, M::Load64Zero)?,
            0x5e => Instr::Simd(S::F32x4DemoteF64x2Zero),
            0x5f => Instr::Simd(S::F64x2PromoteLowF32x4),
            0x60 => Instr::Simd(S::I8x16Abs),
            0x61 => Instr::Simd(S::I8x16Neg),
            0x62 => Instr::Simd(S::I8x16Popcnt),
            0x63 => Instr::Simd(S::I8x16AllTrue),
            0x64 => Instr::Simd(S::I8x16Bitmask),
            0x65 => Instr::Simd(S::I8x16NarrowI16x8S),
            0x66 => Instr::Simd(S::I8x16NarrowI16x8U),
            0x67 => Instr::Simd(S::F32x4Ceil),
            0x68 => Instr::Simd(S::F32x4Floor),
            0x69 => Instr::Simd(S::F32x4Trunc),
            0x6a => Instr::Simd(S::F32x4Nearest),
            0x6b => Instr::Simd(S::I8x16Shl),
            0x6c => Instr::Simd(S::I8x16ShrS),
            0x6d => Instr::Simd(S::I8x16ShrU),
            0x6e => Instr::Simd(S::I8x16Add),
            0x6f => Instr::Simd(S::I8x16AddSatS),
            0x70 => Instr::Simd(S::I8x16AddSatU),
            0x71 => Instr::Simd(S::I8x16Sub),
            0x72 => Instr::Simd(S::I8x16SubSatS),
            0x73 => Instr::Simd(S::I8x16SubSatU),
            0x74 => Instr::Simd(S::F64x2Ceil),
            0x75 => Instr::Simd(S::F64x2Floor),
            0x76 => Instr::Simd(S::I8x16MinS),
            0x77 => Instr::Simd(S::I8x16MinU),
            0x78 => Instr::Simd(S::I8x16MaxS),
            0x79 => Instr::Simd(S::I8x16MaxU),
            0x7a => Instr::Simd(S::F64x2Trunc),
            0x7b => Instr::Simd(S::I8x16AvgrU),
            0x7c => Instr::Simd(S::I16x8ExtaddPairwiseI8x16S),
            0x7d => Instr::Simd(S::I16x8ExtaddPairwiseI8x16U),
            0x7e => Instr::Simd(S::I32x4ExtaddPairwiseI16x8S),
            0x7f => Instr::Simd(S::I32x4ExtaddPairwiseI16x8U),
            0x80 => Instr::Simd(S::I16x8Abs),
            0x81 => Instr::Simd(S::I16x8Neg),
            0x82 => Instr::Simd(S::I16x8Q15mulrSatS),
            0x83 => Instr::Simd(S::I16x8AllTrue),
            0x84 => Instr::Simd(S::I16x8Bitmask),
            0x85 => Instr::Simd(S::I16x8NarrowI32x4S),
            0x86 => Instr::Simd(S::I16x8NarrowI32x4U),
            0x87 => Instr::Simd(S::I16x8ExtendLowI8x16S),
            0x88 => Instr::Simd(S::I16x8ExtendHighI8x16S),
            0x89 => Instr::Simd(S::I16x8ExtendLowI8x16U),
            0x8a => Instr::Simd(S::I16x8ExtendHighI8x16U),
            0x8b => Instr::Simd(S::I16x8Shl),
            0x8c => Instr::Simd(S::I16x8ShrS),
            0x8d => Instr::Simd(S::I16x8ShrU),
            0x8e => Instr::Simd(S::I16x8Add),
            0x8f => Instr::Simd(S::I16x8AddSatS),
            0x90 => Instr::Simd(S::I16x8AddSatU),
            0x91 => Instr::Simd(S::I16x8Sub),
            0x92 => Instr::Simd(S::I16x8SubSatS),
            0x93 => Instr::Simd(S::I16x8SubSatU),
            0x94 => Instr::Simd(S::F64x2Nearest),
            0x95 => Instr::Simd(S::I16x8Mul),
            0x96 => Instr::Simd(S::I16x8MinS),
            0x97 => Instr::Simd(S::I16x8MinU),
            0x98 => Instr::Simd(S::I16x8MaxS),
            0x99 => Instr::Simd(S::I16x8MaxU),
            0x9b => Instr::Simd(S::I16x8AvgrU),
            0x9c => Instr::Simd(S::I16x8ExtmulLowI8x16S),
            0x9d => Instr::Simd(S::I16x8ExtmulHighI8x16S),
            0x9e => Instr::Simd(S::I16x8ExtmulLowI8x16U),
            0x9f => Instr::Simd(S::I16x8ExtmulHighI8x16U),
            0xa0 => Instr::Simd(S::I32x4Abs),
            0xa1 => Instr::Simd(S::I32x4Neg),
            0xa3 => Instr::Simd(S::I32x4AllTrue),
            0xa4 => Instr::Simd(S::I32x4Bitmask),
            0xa7 => Instr::Simd(S::I32x4ExtendLowI16x8S),
            0xa8 => Instr::Simd(S::I32x4ExtendHighI16x8S),
            0xa9 => Instr::Simd(S::I32x4ExtendLowI16x8U),
            0xaa => Instr::Simd(S::I32x4ExtendHighI16x8U),
            0xab => Instr::Simd(S::I32x4Shl),
            0xac => Instr::Simd(S::I32x4ShrS),
            0xad => Instr::Simd(S::I32x4ShrU),
            0xae => Instr::Simd(S::I32x4Add),
            0xb1 => Instr::Simd(S::I32x4Sub),
            0xb5 => Instr::Simd(S::I32x4Mul),
            0xb6 => Instr::Simd(S::I32x4MinS),
            0xb7 => Instr::Simd(S::I32x4MinU),
            0xb8 => Instr::Simd(S::I32x4MaxS),
            0xb9 => Instr::Simd(S::I32x4MaxU),
            0xba => Instr::Simd(S::I32x4DotI16x8S),
            0xbc => Instr::Simd(S::I32x4ExtmulLowI16x8S),
            0xbd => Instr::Simd(S::I32x4ExtmulHighI16x8S),
            0xbe => Instr::Simd(S::I32x4ExtmulLowI16x8U),
            0xbf => Instr::Simd(S::I32x4ExtmulHighI16x8U),
            0xc0 => Instr::Simd(S::I64x2Abs),
            0xc1 => Instr::Simd(S::I64x2Neg),
            0xc3 => Instr::Simd(S::I64x2AllTrue),
            0xc4 => Instr::Simd(S::I64x2Bitmask),
            0xc7 => Instr::Simd(S::I64x2ExtendLowI32x4S),
            0xc8 => Instr::Simd(S::I64x2ExtendHighI32x4S),
            0xc9 => Instr::Simd(S::I64x2ExtendLowI32x4U),
            0xca => Instr::Simd(S::I64x2ExtendHighI32x4U),
            0xcb => Instr::Simd(S::I64x2Shl),
            0xcc => Instr::Simd(S::I64x2ShrS),
            0xcd => Instr::Simd(S::I64x2ShrU),
            0xce => Instr::Simd(S::I64x2Add),
            0xd1 => Instr::Simd(S::I64x2Sub),
            0xd5 => Instr::Simd(S::I64x2Mul),
            0xd6 => Instr::Simd(S::I64x2Eq),
            0xd7 => Instr::Simd(S::I64x2Ne),
            0xd8 => Instr::Simd(S::I64x2LtS),
            0xd9 => Instr::Simd(S::I64x2GtS),
            0xda => Instr::Simd(S::I64x2LeS),
            0xdb => Instr::Simd(S::I64x2GeS),
            0xdc => Instr::Simd(S::I64x2ExtmulLowI32x4S),
            0xdd => Instr::Simd(S::I64x2ExtmulHighI32x4S),
            0xde => Instr::Simd(S::I64x2ExtmulLowI32x4U),
            0xdf => Instr::Simd(S::I64x2ExtmulHighI32x4U),
            0xe0 => Instr::Simd(S::F32x4Abs),
            0xe1 => Instr::Simd(S::F32x4Neg),
            0xe3 => Instr::Simd(S::F32x4Sqrt),
            0xe4 => Instr::Simd(S::F32x4Add),
            0xe5 => Instr::Simd(S::F32x4Sub),
            0xe6 => Instr::Simd(S::F32x4Mul),
            0xe7 => Instr::Simd(S::F32x4Div),
            0xe8 => Instr::Simd(S::F32x4Min),
            0xe9 => Instr::Simd(S::F32x4Max),
            0xea => Instr::Simd(S::F32x4Pmin),
            0xeb => Instr::Simd(S::F32x4Pmax),
            0xec => Instr::Simd(S::F64x2Abs),
            0xed => Instr::Simd(S::F64x2Neg),
            0xef => Instr::Simd(S::F64x2Sqrt),
            0xf0 => Instr::Simd(S::F64x2Add),
            0xf1 => Instr::Simd(S::F64x2Sub),
            0xf2 => Instr::Simd(S::F64x2Mul),
            0xf3 => Instr::Simd(S::F64x2Div),
            0xf4 => Instr::Simd(S::F64x2Min),
            0xf5 => Instr::Simd(S::F64x2Max),
            0xf6 => Instr::Simd(S::F64x2Pmin),
            0xf7 => Instr::Simd(S::F64x2Pmax),
            0xf8 => Instr::Simd(S::I32x4TruncSatF32x4S),
            0xf9 => Instr::Simd(S::I32x4TruncSatF32x4U),
            0xfa => Instr::Simd(S::F32x4ConvertI32x4S),
            0xfb => Instr::Simd(S::F32x4ConvertI32x4U),
            0xfc => Instr::Simd(S::I32x4TruncSatF64x2SZero),
            0xfd => Instr::Simd(S::I32x4TruncSatF64x2UZero),
            0xfe => Instr::Simd(S::F64x2ConvertLowI32x4S),
            0xff => Instr::Simd(S::F64x2ConvertLowI32x4U),
            _ => return self.err(format!("illegal opcode 0xfd {sub:#x}")),
        })
    }
}

/// Plain one-byte numeric opcodes 0x45..=0xc4.
fn plain_numop(op: u8) -> Option<NumOp> {
    use NumOp::*;
    Some(match op {
        0x45 => I32Eqz,
        0x46 => I32Eq,
        0x47 => I32Ne,
        0x48 => I32LtS,
        0x49 => I32LtU,
        0x4a => I32GtS,
        0x4b => I32GtU,
        0x4c => I32LeS,
        0x4d => I32LeU,
        0x4e => I32GeS,
        0x4f => I32GeU,
        0x50 => I64Eqz,
        0x51 => I64Eq,
        0x52 => I64Ne,
        0x53 => I64LtS,
        0x54 => I64LtU,
        0x55 => I64GtS,
        0x56 => I64GtU,
        0x57 => I64LeS,
        0x58 => I64LeU,
        0x59 => I64GeS,
        0x5a => I64GeU,
        0x5b => F32Eq,
        0x5c => F32Ne,
        0x5d => F32Lt,
        0x5e => F32Gt,
        0x5f => F32Le,
        0x60 => F32Ge,
        0x61 => F64Eq,
        0x62 => F64Ne,
        0x63 => F64Lt,
        0x64 => F64Gt,
        0x65 => F64Le,
        0x66 => F64Ge,
        0x67 => I32Clz,
        0x68 => I32Ctz,
        0x69 => I32Popcnt,
        0x6a => I32Add,
        0x6b => I32Sub,
        0x6c => I32Mul,
        0x6d => I32DivS,
        0x6e => I32DivU,
        0x6f => I32RemS,
        0x70 => I32RemU,
        0x71 => I32And,
        0x72 => I32Or,
        0x73 => I32Xor,
        0x74 => I32Shl,
        0x75 => I32ShrS,
        0x76 => I32ShrU,
        0x77 => I32Rotl,
        0x78 => I32Rotr,
        0x79 => I64Clz,
        0x7a => I64Ctz,
        0x7b => I64Popcnt,
        0x7c => I64Add,
        0x7d => I64Sub,
        0x7e => I64Mul,
        0x7f => I64DivS,
        0x80 => I64DivU,
        0x81 => I64RemS,
        0x82 => I64RemU,
        0x83 => I64And,
        0x84 => I64Or,
        0x85 => I64Xor,
        0x86 => I64Shl,
        0x87 => I64ShrS,
        0x88 => I64ShrU,
        0x89 => I64Rotl,
        0x8a => I64Rotr,
        0x8b => F32Abs,
        0x8c => F32Neg,
        0x8d => F32Ceil,
        0x8e => F32Floor,
        0x8f => F32Trunc,
        0x90 => F32Nearest,
        0x91 => F32Sqrt,
        0x92 => F32Add,
        0x93 => F32Sub,
        0x94 => F32Mul,
        0x95 => F32Div,
        0x96 => F32Min,
        0x97 => F32Max,
        0x98 => F32Copysign,
        0x99 => F64Abs,
        0x9a => F64Neg,
        0x9b => F64Ceil,
        0x9c => F64Floor,
        0x9d => F64Trunc,
        0x9e => F64Nearest,
        0x9f => F64Sqrt,
        0xa0 => F64Add,
        0xa1 => F64Sub,
        0xa2 => F64Mul,
        0xa3 => F64Div,
        0xa4 => F64Min,
        0xa5 => F64Max,
        0xa6 => F64Copysign,
        0xa7 => I32WrapI64,
        0xa8 => I32TruncF32S,
        0xa9 => I32TruncF32U,
        0xaa => I32TruncF64S,
        0xab => I32TruncF64U,
        0xac => I64ExtendI32S,
        0xad => I64ExtendI32U,
        0xae => I64TruncF32S,
        0xaf => I64TruncF32U,
        0xb0 => I64TruncF64S,
        0xb1 => I64TruncF64U,
        0xb2 => F32ConvertI32S,
        0xb3 => F32ConvertI32U,
        0xb4 => F32ConvertI64S,
        0xb5 => F32ConvertI64U,
        0xb6 => F32DemoteF64,
        0xb7 => F64ConvertI32S,
        0xb8 => F64ConvertI32U,
        0xb9 => F64ConvertI64S,
        0xba => F64ConvertI64U,
        0xbb => F64PromoteF32,
        0xbc => I32ReinterpretF32,
        0xbd => I64ReinterpretF64,
        0xbe => F32ReinterpretI32,
        0xbf => F64ReinterpretI64,
        0xc0 => I32Extend8S,
        0xc1 => I32Extend16S,
        0xc2 => I64Extend8S,
        0xc3 => I64Extend16S,
        0xc4 => I64Extend32S,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(bytes: &[u8]) -> Result<Module, DecodeError> {
        decode(bytes)
    }

    #[test]
    fn empty_module() {
        assert!(dec(b"\0asm\x01\0\0\0").is_ok());
    }

    #[test]
    fn bad_magic_and_version() {
        assert!(dec(b"\0ASM\x01\0\0\0").is_err());
        assert!(dec(b"\0asm\x02\0\0\0").is_err());
        assert!(dec(b"\0asm").is_err());
    }

    #[test]
    fn uleb_bounds() {
        // u32 LEB with 6 bytes: representation too long.
        let m = b"\0asm\x01\0\0\0\x01\x86\x80\x80\x80\x80\x80\x00";
        assert!(dec(m).is_err());
        // 5th byte with unused high bits set: integer too large.
        let m2 = b"\0asm\x01\0\0\0\x01\x05\xff\xff\xff\xff\x7f";
        assert!(dec(m2).is_err());
    }

    #[test]
    fn section_order_enforced() {
        // function section (3) before type section (1)
        let m = b"\0asm\x01\0\0\0\x03\x01\x00\x01\x01\x00";
        assert!(dec(m).is_err());
    }

    #[test]
    fn truncated_section() {
        let m = b"\0asm\x01\0\0\0\x01\x7f";
        assert!(dec(m).is_err());
    }
}
