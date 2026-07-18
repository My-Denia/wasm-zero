//! WebAssembly 2.0 validator: module-level checks plus the operand-stack /
//! control-stack type checker from the spec appendix (with bottom values
//! for unreachable polymorphism).

use crate::error::ValidateError;
use crate::module::*;
use crate::types::*;

type R<T> = Result<T, ValidateError>;

fn err<T>(msg: impl Into<String>) -> R<T> {
    Err(ValidateError { msg: msg.into() })
}

/// Module-wide view with imports and local definitions merged.
struct Ctx<'m> {
    m: &'m Module,
    funcs: Vec<u32>, // type indices
    tables: Vec<TableType>,
    mems: Vec<MemType>,
    globals: Vec<GlobalType>,
    num_imported_globals: usize,
    refs: Vec<bool>, // "declared" functions (usable by ref.func)
}

pub fn validate(m: &Module) -> R<()> {
    let mut funcs = Vec::new();
    let mut tables = Vec::new();
    let mut mems = Vec::new();
    let mut globals = Vec::new();

    for imp in &m.imports {
        match imp.desc {
            ImportDesc::Func(t) => {
                if t as usize >= m.types.len() {
                    return err("unknown type");
                }
                funcs.push(t);
            }
            ImportDesc::Table(t) => {
                check_tabletype(&t)?;
                tables.push(t);
            }
            ImportDesc::Mem(t) => {
                check_memtype(&t)?;
                mems.push(t);
            }
            ImportDesc::Global(t) => globals.push(t),
        }
    }
    let num_imported_globals = globals.len();

    for &t in &m.funcs {
        if t as usize >= m.types.len() {
            return err("unknown type");
        }
        funcs.push(t);
    }
    for t in &m.tables {
        check_tabletype(t)?;
        tables.push(*t);
    }
    for t in &m.mems {
        check_memtype(t)?;
        mems.push(*t);
    }
    for g in &m.globals {
        globals.push(g.ty);
    }

    if mems.len() > 1 {
        return err("multiple memories");
    }

    // Collect declared function references (elem segments, global inits,
    // exports) before validating bodies.
    let mut refs = vec![false; funcs.len()];
    let mark = |e: &ConstExpr, refs: &mut Vec<bool>| {
        for i in &e.instrs {
            if let Instr::RefFunc { func } = i {
                if (*func as usize) < refs.len() {
                    refs[*func as usize] = true;
                }
            }
        }
    };
    for g in &m.globals {
        mark(&g.init, &mut refs);
    }
    for e in &m.elems {
        for init in &e.init {
            mark(init, &mut refs);
        }
        if let ElemMode::Active { offset, .. } = &e.mode {
            mark(offset, &mut refs);
        }
    }
    for ex in &m.exports {
        if let ExportDesc::Func(i) = ex.desc {
            if (i as usize) < refs.len() {
                refs[i as usize] = true;
            }
        }
    }

    let ctx = Ctx {
        m,
        funcs,
        tables,
        mems,
        globals,
        num_imported_globals,
        refs,
    };

    // Globals: init must be constant and match the declared type. The
    // context for initializers only contains the *imported* globals.
    for g in &m.globals {
        check_const_expr(&ctx, &g.init, g.ty.val)?;
    }

    // Element segments.
    for e in &m.elems {
        for init in &e.init {
            check_const_expr(&ctx, init, e.ty.into())?;
        }
        if let ElemMode::Active { table, offset } = &e.mode {
            let Some(tt) = ctx.tables.get(*table as usize) else {
                return err("unknown table");
            };
            if tt.elem != e.ty {
                return err("type mismatch");
            }
            check_const_expr(&ctx, offset, ValType::I32)?;
        }
    }

    // Data segments.
    for d in &m.datas {
        if let DataMode::Active { mem, offset } = &d.mode {
            if *mem as usize >= ctx.mems.len() {
                return err("unknown memory");
            }
            check_const_expr(&ctx, offset, ValType::I32)?;
        }
    }

    // Start function.
    if let Some(s) = m.start {
        let Some(&t) = ctx.funcs.get(s as usize) else {
            return err("unknown function");
        };
        let ft = &m.types[t as usize];
        if !ft.params.is_empty() || !ft.results.is_empty() {
            return err("start function");
        }
    }

    // Exports: bounds and uniqueness.
    let mut names: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for ex in &m.exports {
        if !names.insert(ex.name.as_str()) {
            return err("duplicate export name");
        }
        let ok = match ex.desc {
            ExportDesc::Func(i) => (i as usize) < ctx.funcs.len(),
            ExportDesc::Table(i) => (i as usize) < ctx.tables.len(),
            ExportDesc::Mem(i) => (i as usize) < ctx.mems.len(),
            ExportDesc::Global(i) => (i as usize) < ctx.globals.len(),
        };
        if !ok {
            return match ex.desc {
                ExportDesc::Func(_) => err("unknown function"),
                ExportDesc::Table(_) => err("unknown table"),
                ExportDesc::Mem(_) => err("unknown memory"),
                ExportDesc::Global(_) => err("unknown global"),
            };
        }
    }

    // Code bodies.
    let num_imported_funcs = ctx.funcs.len() - m.funcs.len();
    for (i, code) in m.codes.iter().enumerate() {
        let ty = &m.types[m.funcs[i] as usize];
        check_body(&ctx, ty, code).map_err(|e| ValidateError {
            msg: format!("func {}: {}", num_imported_funcs + i, e.msg),
        })?;
    }

    Ok(())
}

fn check_tabletype(t: &TableType) -> R<()> {
    if let Some(max) = t.limits.max {
        if max < t.limits.min {
            return err("size minimum must not be greater than maximum");
        }
    }
    Ok(())
}

fn check_memtype(t: &MemType) -> R<()> {
    const MAX_PAGES: u32 = 65536;
    if t.limits.min > MAX_PAGES {
        return err("memory size must be at most 65536 pages");
    }
    if let Some(max) = t.limits.max {
        if max > MAX_PAGES {
            return err("memory size must be at most 65536 pages");
        }
        if max < t.limits.min {
            return err("size minimum must not be greater than maximum");
        }
    }
    Ok(())
}

/// Constant expression check: only const operators, typing to exactly [want].
fn check_const_expr(ctx: &Ctx, e: &ConstExpr, want: ValType) -> R<()> {
    let mut stack: Vec<ValType> = Vec::new();
    let n = e.instrs.len();
    for (i, instr) in e.instrs.iter().enumerate() {
        match instr {
            Instr::End => {
                if i != n - 1 {
                    return err("constant expression required");
                }
            }
            Instr::I32Const(_) => stack.push(ValType::I32),
            Instr::I64Const(_) => stack.push(ValType::I64),
            Instr::F32Const(_) => stack.push(ValType::F32),
            Instr::F64Const(_) => stack.push(ValType::F64),
            Instr::V128Const(_) => stack.push(ValType::V128),
            Instr::RefNull(t) => stack.push((*t).into()),
            Instr::RefFunc { func } => {
                if *func as usize >= ctx.funcs.len() {
                    return err("unknown function");
                }
                stack.push(ValType::FuncRef);
            }
            Instr::GlobalGet { idx } => {
                let i = *idx as usize;
                if i >= ctx.num_imported_globals {
                    // The initializer context only has imported globals.
                    return if i < ctx.globals.len() {
                        err("constant expression required")
                    } else {
                        err("unknown global")
                    };
                }
                if ctx.globals[i].mutable {
                    return err("constant expression required");
                }
                stack.push(ctx.globals[i].val);
            }
            _ => return err("constant expression required"),
        }
    }
    if stack.len() != 1 || stack[0] != want {
        return err("type mismatch");
    }
    Ok(())
}

// ---- body type checking ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Block,
    Loop,
    If,
    Else,
}

struct Frame {
    kind: Kind,
    ins: Vec<ValType>,
    outs: Vec<ValType>,
    height: usize,
    unreachable: bool,
}

struct Checker<'c, 'm> {
    ctx: &'c Ctx<'m>,
    locals: Vec<ValType>,
    stack: Vec<Option<ValType>>, // None = bottom
    ctrls: Vec<Frame>,
}

impl<'c, 'm> Checker<'c, 'm> {
    fn push(&mut self, t: ValType) {
        self.stack.push(Some(t));
    }

    fn push_bot(&mut self) {
        self.stack.push(None);
    }

    fn pop_any(&mut self) -> R<Option<ValType>> {
        let f = self.ctrls.last().unwrap();
        if self.stack.len() == f.height {
            if f.unreachable {
                return Ok(None);
            }
            return err("type mismatch");
        }
        Ok(self.stack.pop().unwrap())
    }

    fn pop(&mut self, want: ValType) -> R<()> {
        match self.pop_any()? {
            None => Ok(()),
            Some(t) if t == want => Ok(()),
            Some(_) => err("type mismatch"),
        }
    }

    fn pop_ref(&mut self) -> R<()> {
        match self.pop_any()? {
            None => Ok(()),
            Some(t) if t.is_ref() => Ok(()),
            Some(_) => err("type mismatch"),
        }
    }

    fn pop_vals(&mut self, ts: &[ValType]) -> R<()> {
        for t in ts.iter().rev() {
            self.pop(*t)?;
        }
        Ok(())
    }

    fn push_vals(&mut self, ts: &[ValType]) {
        for t in ts {
            self.push(*t);
        }
    }

    fn push_ctrl(&mut self, kind: Kind, ins: Vec<ValType>, outs: Vec<ValType>) {
        let height = self.stack.len();
        self.push_vals(&ins.clone());
        self.ctrls.push(Frame {
            kind,
            ins,
            outs,
            height,
            unreachable: false,
        });
    }

    fn pop_ctrl(&mut self) -> R<Frame> {
        let Some(f) = self.ctrls.last() else {
            return err("type mismatch");
        };
        let outs = f.outs.clone();
        let height = f.height;
        self.pop_vals(&outs)?;
        if self.stack.len() != height {
            return err("type mismatch");
        }
        Ok(self.ctrls.pop().unwrap())
    }

    fn unreachable(&mut self) {
        let f = self.ctrls.last_mut().unwrap();
        self.stack.truncate(f.height);
        f.unreachable = true;
    }

    fn label_types(&self, depth: u32) -> R<Vec<ValType>> {
        let n = self.ctrls.len();
        if depth as usize >= n {
            return err("unknown label");
        }
        let f = &self.ctrls[n - 1 - depth as usize];
        Ok(if f.kind == Kind::Loop {
            f.ins.clone()
        } else {
            f.outs.clone()
        })
    }

    fn block_type(&self, bt: &BlockType) -> R<(Vec<ValType>, Vec<ValType>)> {
        match bt {
            BlockType::Empty => Ok((vec![], vec![])),
            BlockType::Val(t) => Ok((vec![], vec![*t])),
            BlockType::Type(i) => {
                let Some(ft) = self.ctx.m.types.get(*i as usize) else {
                    return err("unknown type");
                };
                Ok((ft.params.clone(), ft.results.clone()))
            }
        }
    }

    fn local(&self, idx: u32) -> R<ValType> {
        self.locals
            .get(idx as usize)
            .copied()
            .ok_or_else(|| ValidateError {
                msg: "unknown local".into(),
            })
    }

    fn global(&self, idx: u32) -> R<GlobalType> {
        self.ctx
            .globals
            .get(idx as usize)
            .copied()
            .ok_or_else(|| ValidateError {
                msg: "unknown global".into(),
            })
    }

    fn table(&self, idx: u32) -> R<TableType> {
        self.ctx
            .tables
            .get(idx as usize)
            .copied()
            .ok_or_else(|| ValidateError {
                msg: "unknown table".into(),
            })
    }

    fn need_mem(&self) -> R<()> {
        if self.ctx.mems.is_empty() {
            return err("unknown memory 0");
        }
        Ok(())
    }

    fn check_align(&self, align: u32, natural: u32) -> R<()> {
        if align >= 32 || (1u64 << align) > u64::from(natural) {
            return err("alignment must not be larger than natural");
        }
        Ok(())
    }
}

fn check_body(ctx: &Ctx, ty: &FuncType, code: &Code) -> R<()> {
    let mut locals = ty.params.clone();
    locals.extend_from_slice(&code.locals);
    let mut c = Checker {
        ctx,
        locals,
        stack: Vec::new(),
        ctrls: Vec::new(),
    };
    c.push_ctrl(Kind::Block, vec![], ty.results.clone());
    for instr in &code.body {
        step(&mut c, instr)?;
        if c.ctrls.is_empty() {
            break;
        }
    }
    if !c.ctrls.is_empty() {
        return err("type mismatch"); // body not terminated (decode prevents)
    }
    Ok(())
}

fn step(c: &mut Checker, instr: &Instr) -> R<()> {
    use ValType::*;
    match instr {
        Instr::Unreachable => c.unreachable(),
        Instr::Nop => {}
        Instr::Block { bt, .. } => {
            let (ins, outs) = c.block_type(bt)?;
            c.pop_vals(&ins)?;
            c.push_ctrl(Kind::Block, ins, outs);
        }
        Instr::Loop { bt, .. } => {
            let (ins, outs) = c.block_type(bt)?;
            c.pop_vals(&ins)?;
            c.push_ctrl(Kind::Loop, ins, outs);
        }
        Instr::If { bt, .. } => {
            c.pop(I32)?;
            let (ins, outs) = c.block_type(bt)?;
            c.pop_vals(&ins)?;
            c.push_ctrl(Kind::If, ins, outs);
        }
        Instr::Else { .. } => {
            let f = c.pop_ctrl()?;
            if f.kind != Kind::If {
                return err("type mismatch");
            }
            c.push_ctrl(Kind::Else, f.ins, f.outs);
        }
        Instr::End => {
            let f = c.pop_ctrl()?;
            if f.kind == Kind::If && f.ins != f.outs {
                // Missing else arm must be an identity.
                return err("type mismatch");
            }
            c.push_vals(&f.outs);
        }
        Instr::Br { depth } => {
            let ts = c.label_types(*depth)?;
            c.pop_vals(&ts)?;
            c.unreachable();
        }
        Instr::BrIf { depth } => {
            c.pop(I32)?;
            let ts = c.label_types(*depth)?;
            c.pop_vals(&ts)?;
            c.push_vals(&ts);
        }
        Instr::BrTable { depths, default } => {
            c.pop(I32)?;
            let dts = c.label_types(*default)?;
            for d in depths {
                let ts = c.label_types(*d)?;
                if ts.len() != dts.len() {
                    return err("type mismatch");
                }
                // Push back exactly what was popped: bottom values must stay
                // bottom so heterogeneous targets meet in unreachable code.
                let mut popped = Vec::with_capacity(ts.len());
                for t in ts.iter().rev() {
                    match c.pop_any()? {
                        None => popped.push(None),
                        Some(v) if v == *t => popped.push(Some(v)),
                        Some(_) => return err("type mismatch"),
                    }
                }
                for v in popped.into_iter().rev() {
                    match v {
                        Some(t) => c.push(t),
                        None => c.push_bot(),
                    }
                }
            }
            c.pop_vals(&dts)?;
            c.unreachable();
        }
        Instr::Return => {
            let outs = c.ctrls[0].outs.clone();
            c.pop_vals(&outs)?;
            c.unreachable();
        }
        Instr::Call { func } => {
            let Some(&t) = c.ctx.funcs.get(*func as usize) else {
                return err("unknown function");
            };
            let ft = c.ctx.m.types[t as usize].clone();
            c.pop_vals(&ft.params)?;
            c.push_vals(&ft.results);
        }
        Instr::CallIndirect { table, ty } => {
            let tt = c.table(*table)?;
            if tt.elem != RefType::FuncRef {
                return err("type mismatch");
            }
            let Some(ft) = c.ctx.m.types.get(*ty as usize).cloned() else {
                return err("unknown type");
            };
            c.pop(I32)?;
            c.pop_vals(&ft.params)?;
            c.push_vals(&ft.results);
        }
        Instr::Drop => {
            c.pop_any()?;
        }
        Instr::SelectT { tys } => {
            if tys.len() != 1 {
                return err("invalid result arity");
            }
            c.pop(I32)?;
            c.pop(tys[0])?;
            c.pop(tys[0])?;
            c.push(tys[0]);
        }
        Instr::Select { ty } => {
            c.pop(I32)?;
            match ty {
                Some(t) => {
                    c.pop(*t)?;
                    c.pop(*t)?;
                    c.push(*t);
                }
                None => {
                    let t1 = c.pop_any()?;
                    let t2 = c.pop_any()?;
                    for t in [t1, t2].into_iter().flatten() {
                        if t.is_ref() {
                            return err("type mismatch");
                        }
                    }
                    match (t1, t2) {
                        (Some(a), Some(b)) if a != b => return err("type mismatch"),
                        (Some(a), _) => c.push(a),
                        (None, Some(b)) => c.push(b),
                        (None, None) => c.push_bot(),
                    }
                }
            }
        }
        Instr::LocalGet { idx } => {
            let t = c.local(*idx)?;
            c.push(t);
        }
        Instr::LocalSet { idx } => {
            let t = c.local(*idx)?;
            c.pop(t)?;
        }
        Instr::LocalTee { idx } => {
            let t = c.local(*idx)?;
            c.pop(t)?;
            c.push(t);
        }
        Instr::GlobalGet { idx } => {
            let g = c.global(*idx)?;
            c.push(g.val);
        }
        Instr::GlobalSet { idx } => {
            let g = c.global(*idx)?;
            if !g.mutable {
                return err("global is immutable");
            }
            c.pop(g.val)?;
        }
        Instr::TableGet { table } => {
            let tt = c.table(*table)?;
            c.pop(I32)?;
            c.push(tt.elem.into());
        }
        Instr::TableSet { table } => {
            let tt = c.table(*table)?;
            c.pop(tt.elem.into())?;
            c.pop(I32)?;
        }
        Instr::TableInit { table, elem } => {
            let tt = c.table(*table)?;
            let Some(es) = c.ctx.m.elems.get(*elem as usize) else {
                return err("unknown elem segment");
            };
            if es.ty != tt.elem {
                return err("type mismatch");
            }
            c.pop(I32)?;
            c.pop(I32)?;
            c.pop(I32)?;
        }
        Instr::ElemDrop { elem } => {
            if *elem as usize >= c.ctx.m.elems.len() {
                return err("unknown elem segment");
            }
        }
        Instr::TableCopy { dst, src } => {
            let d = c.table(*dst)?;
            let s = c.table(*src)?;
            if d.elem != s.elem {
                return err("type mismatch");
            }
            c.pop(I32)?;
            c.pop(I32)?;
            c.pop(I32)?;
        }
        Instr::TableGrow { table } => {
            let tt = c.table(*table)?;
            c.pop(I32)?;
            c.pop(tt.elem.into())?;
            c.push(I32);
        }
        Instr::TableSize { table } => {
            c.table(*table)?;
            c.push(I32);
        }
        Instr::TableFill { table } => {
            let tt = c.table(*table)?;
            c.pop(I32)?;
            c.pop(tt.elem.into())?;
            c.pop(I32)?;
        }
        Instr::Load { op, align, .. } => {
            c.need_mem()?;
            let (natural, t) = match op {
                LoadOp::I32 => (4, I32),
                LoadOp::I64 => (8, I64),
                LoadOp::F32 => (4, F32),
                LoadOp::F64 => (8, F64),
                LoadOp::I32L8S | LoadOp::I32L8U => (1, I32),
                LoadOp::I32L16S | LoadOp::I32L16U => (2, I32),
                LoadOp::I64L8S | LoadOp::I64L8U => (1, I64),
                LoadOp::I64L16S | LoadOp::I64L16U => (2, I64),
                LoadOp::I64L32S | LoadOp::I64L32U => (4, I64),
            };
            c.check_align(*align, natural)?;
            c.pop(I32)?;
            c.push(t);
        }
        Instr::Store { op, align, .. } => {
            c.need_mem()?;
            let (natural, t) = match op {
                StoreOp::I32 => (4, I32),
                StoreOp::I64 => (8, I64),
                StoreOp::F32 => (4, F32),
                StoreOp::F64 => (8, F64),
                StoreOp::I32S8 => (1, I32),
                StoreOp::I32S16 => (2, I32),
                StoreOp::I64S8 => (1, I64),
                StoreOp::I64S16 => (2, I64),
                StoreOp::I64S32 => (4, I64),
            };
            c.check_align(*align, natural)?;
            c.pop(t)?;
            c.pop(I32)?;
        }
        Instr::MemorySize => {
            c.need_mem()?;
            c.push(I32);
        }
        Instr::MemoryGrow => {
            c.need_mem()?;
            c.pop(I32)?;
            c.push(I32);
        }
        Instr::MemoryInit { data } => {
            c.need_mem()?;
            if *data as usize >= c.ctx.m.datas.len() {
                return err("unknown data segment");
            }
            c.pop(I32)?;
            c.pop(I32)?;
            c.pop(I32)?;
        }
        Instr::DataDrop { data } => {
            if *data as usize >= c.ctx.m.datas.len() {
                return err("unknown data segment");
            }
        }
        Instr::MemoryCopy | Instr::MemoryFill => {
            c.need_mem()?;
            c.pop(I32)?;
            c.pop(I32)?;
            c.pop(I32)?;
        }
        Instr::I32Const(_) => c.push(I32),
        Instr::I64Const(_) => c.push(I64),
        Instr::F32Const(_) => c.push(F32),
        Instr::F64Const(_) => c.push(F64),
        Instr::V128Const(_) => c.push(V128),
        Instr::RefNull(t) => c.push((*t).into()),
        Instr::RefIsNull => {
            c.pop_ref()?;
            c.push(I32);
        }
        Instr::RefFunc { func } => {
            if *func as usize >= c.ctx.funcs.len() {
                return err("unknown function");
            }
            if !c.ctx.refs[*func as usize] {
                return err("undeclared function reference");
            }
            c.push(FuncRef);
        }
        Instr::Num(op) => num_step(c, *op)?,
        Instr::Simd(op) => simd_step(c, *op)?,
        Instr::SimdLane { op, lane } => {
            use SimdLaneOp::*;
            let (lanes, scalar) = match op {
                I8x16ExtractLaneS | I8x16ExtractLaneU | I8x16ReplaceLane => (16, I32),
                I16x8ExtractLaneS | I16x8ExtractLaneU | I16x8ReplaceLane => (8, I32),
                I32x4ExtractLane | I32x4ReplaceLane => (4, I32),
                I64x2ExtractLane | I64x2ReplaceLane => (2, I64),
                F32x4ExtractLane | F32x4ReplaceLane => (4, F32),
                F64x2ExtractLane | F64x2ReplaceLane => (2, F64),
            };
            if u32::from(*lane) >= lanes {
                return err("invalid lane index");
            }
            let is_replace = matches!(
                op,
                I8x16ReplaceLane
                    | I16x8ReplaceLane
                    | I32x4ReplaceLane
                    | I64x2ReplaceLane
                    | F32x4ReplaceLane
                    | F64x2ReplaceLane
            );
            if is_replace {
                c.pop(scalar)?;
                c.pop(V128)?;
                c.push(V128);
            } else {
                c.pop(V128)?;
                c.push(scalar);
            }
        }
        Instr::Shuffle(lanes) => {
            if lanes.iter().any(|&l| l >= 32) {
                return err("invalid lane index");
            }
            c.pop(V128)?;
            c.pop(V128)?;
            c.push(V128);
        }
        Instr::SimdMem {
            op, align, lane, ..
        } => {
            c.need_mem()?;
            use SimdMemOp::*;
            let natural: u32 = match op {
                Load | Store => 16,
                Load8x8S | Load8x8U | Load16x4S | Load16x4U | Load32x2S | Load32x2U => 8,
                Load8Splat => 1,
                Load16Splat => 2,
                Load32Splat => 4,
                Load64Splat => 8,
                Load32Zero => 4,
                Load64Zero => 8,
                Load8Lane | Store8Lane => 1,
                Load16Lane | Store16Lane => 2,
                Load32Lane | Store32Lane => 4,
                Load64Lane | Store64Lane => 8,
            };
            c.check_align(*align, natural)?;
            let lanes: u32 = match op {
                Load8Lane | Store8Lane => 16,
                Load16Lane | Store16Lane => 8,
                Load32Lane | Store32Lane => 4,
                Load64Lane | Store64Lane => 2,
                _ => 0,
            };
            if lanes > 0 && u32::from(*lane) >= lanes {
                return err("invalid lane index");
            }
            match op {
                Store => {
                    c.pop(V128)?;
                    c.pop(I32)?;
                }
                Load8Lane | Load16Lane | Load32Lane | Load64Lane => {
                    c.pop(V128)?;
                    c.pop(I32)?;
                    c.push(V128);
                }
                Store8Lane | Store16Lane | Store32Lane | Store64Lane => {
                    c.pop(V128)?;
                    c.pop(I32)?;
                }
                _ => {
                    c.pop(I32)?;
                    c.push(V128);
                }
            }
        }
    }
    Ok(())
}

fn num_step(c: &mut Checker, op: NumOp) -> R<()> {
    use NumOp::*;
    use ValType::*;
    // (inputs, output)
    let (ins, out): (&[ValType], ValType) = match op {
        I32Eqz => (&[I32], I32),
        I64Eqz => (&[I64], I32),
        I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS | I32GeU => {
            (&[I32, I32], I32)
        }
        I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS | I64GeU => {
            (&[I64, I64], I32)
        }
        F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge => (&[F32, F32], I32),
        F64Eq | F64Ne | F64Lt | F64Gt | F64Le | F64Ge => (&[F64, F64], I32),
        I32Clz | I32Ctz | I32Popcnt | I32Extend8S | I32Extend16S => (&[I32], I32),
        I32Add | I32Sub | I32Mul | I32DivS | I32DivU | I32RemS | I32RemU | I32And | I32Or
        | I32Xor | I32Shl | I32ShrS | I32ShrU | I32Rotl | I32Rotr => (&[I32, I32], I32),
        I64Clz | I64Ctz | I64Popcnt | I64Extend8S | I64Extend16S | I64Extend32S => (&[I64], I64),
        I64Add | I64Sub | I64Mul | I64DivS | I64DivU | I64RemS | I64RemU | I64And | I64Or
        | I64Xor | I64Shl | I64ShrS | I64ShrU | I64Rotl | I64Rotr => (&[I64, I64], I64),
        F32Abs | F32Neg | F32Ceil | F32Floor | F32Trunc | F32Nearest | F32Sqrt => (&[F32], F32),
        F32Add | F32Sub | F32Mul | F32Div | F32Min | F32Max | F32Copysign => (&[F32, F32], F32),
        F64Abs | F64Neg | F64Ceil | F64Floor | F64Trunc | F64Nearest | F64Sqrt => (&[F64], F64),
        F64Add | F64Sub | F64Mul | F64Div | F64Min | F64Max | F64Copysign => (&[F64, F64], F64),
        I32WrapI64 => (&[I64], I32),
        I32TruncF32S | I32TruncF32U | I32TruncSatF32S | I32TruncSatF32U | I32ReinterpretF32 => {
            (&[F32], I32)
        }
        I32TruncF64S | I32TruncF64U | I32TruncSatF64S | I32TruncSatF64U => (&[F64], I32),
        I64ExtendI32S | I64ExtendI32U => (&[I32], I64),
        I64TruncF32S | I64TruncF32U | I64TruncSatF32S | I64TruncSatF32U => (&[F32], I64),
        I64TruncF64S | I64TruncF64U | I64TruncSatF64S | I64TruncSatF64U | I64ReinterpretF64 => {
            (&[F64], I64)
        }
        F32ConvertI32S | F32ConvertI32U | F32ReinterpretI32 => (&[I32], F32),
        F32ConvertI64S | F32ConvertI64U => (&[I64], F32),
        F32DemoteF64 => (&[F64], F32),
        F64ConvertI32S | F64ConvertI32U => (&[I32], F64),
        F64ConvertI64S | F64ConvertI64U | F64ReinterpretI64 => (&[I64], F64),
        F64PromoteF32 => (&[F32], F64),
    };
    c.pop_vals(ins)?;
    c.push(out);
    Ok(())
}

fn simd_step(c: &mut Checker, op: SimdOp) -> R<()> {
    use SimdOp::*;
    use ValType::*;
    // Splats take a scalar; everything else is v128-shaped.
    let (ins, out): (&[ValType], ValType) = match op {
        I8x16Splat | I16x8Splat | I32x4Splat => (&[I32], V128),
        I64x2Splat => (&[I64], V128),
        F32x4Splat => (&[F32], V128),
        F64x2Splat => (&[F64], V128),
        V128AnyTrue | I8x16AllTrue | I8x16Bitmask | I16x8AllTrue | I16x8Bitmask | I32x4AllTrue
        | I32x4Bitmask | I64x2AllTrue | I64x2Bitmask => (&[V128], I32),
        V128Bitselect => (&[V128, V128, V128], V128),
        I8x16Shl | I8x16ShrS | I8x16ShrU | I16x8Shl | I16x8ShrS | I16x8ShrU | I32x4Shl
        | I32x4ShrS | I32x4ShrU | I64x2Shl | I64x2ShrS | I64x2ShrU => (&[V128, I32], V128),
        V128Not
        | F32x4DemoteF64x2Zero
        | F64x2PromoteLowF32x4
        | I8x16Abs
        | I8x16Neg
        | I8x16Popcnt
        | F32x4Ceil
        | F32x4Floor
        | F32x4Trunc
        | F32x4Nearest
        | F64x2Ceil
        | F64x2Floor
        | F64x2Trunc
        | F64x2Nearest
        | I16x8ExtaddPairwiseI8x16S
        | I16x8ExtaddPairwiseI8x16U
        | I32x4ExtaddPairwiseI16x8S
        | I32x4ExtaddPairwiseI16x8U
        | I16x8Abs
        | I16x8Neg
        | I16x8ExtendLowI8x16S
        | I16x8ExtendHighI8x16S
        | I16x8ExtendLowI8x16U
        | I16x8ExtendHighI8x16U
        | I32x4Abs
        | I32x4Neg
        | I32x4ExtendLowI16x8S
        | I32x4ExtendHighI16x8S
        | I32x4ExtendLowI16x8U
        | I32x4ExtendHighI16x8U
        | I64x2Abs
        | I64x2Neg
        | I64x2ExtendLowI32x4S
        | I64x2ExtendHighI32x4S
        | I64x2ExtendLowI32x4U
        | I64x2ExtendHighI32x4U
        | F32x4Abs
        | F32x4Neg
        | F32x4Sqrt
        | F64x2Abs
        | F64x2Neg
        | F64x2Sqrt
        | I32x4TruncSatF32x4S
        | I32x4TruncSatF32x4U
        | F32x4ConvertI32x4S
        | F32x4ConvertI32x4U
        | I32x4TruncSatF64x2SZero
        | I32x4TruncSatF64x2UZero
        | F64x2ConvertLowI32x4S
        | F64x2ConvertLowI32x4U => (&[V128], V128),
        _ => (&[V128, V128], V128),
    };
    c.pop_vals(ins)?;
    c.push(out);
    Ok(())
}
