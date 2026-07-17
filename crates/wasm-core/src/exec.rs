//! The interpreter: an iterative machine (explicit frame stack, no native
//! recursion) so call-stack exhaustion is a deterministic trap.

use std::rc::Rc;

use crate::error::{InvokeError, Trap};
use crate::module::{BlockType, Code, Instr, LoadOp, NumOp, StoreOp};
use crate::store::{mem_bytes, FuncInst, Store, MAX_PAGES, PAGE_SIZE, TABLE_IMPL_LIMIT};
use crate::values::Value;

const MAX_CALL_DEPTH: usize = 2000;
const MAX_VALUE_STACK: usize = 4_000_000;

struct Ctrl {
    /// Branch target pc: for loops, the loop instruction; for blocks/ifs,
    /// one past the matching End.
    target: usize,

    /// Values transferred on branch (loop: params, block/if: results).
    arity: usize,
    /// Value-stack height at block entry (block arguments popped).
    height: usize,
}

struct Frame {
    code: Rc<Code>,
    inst: usize,
    locals: Vec<Value>,
    pc: usize,
    /// Value-stack base for this frame (args already consumed).
    base: usize,
    ctrl_base: usize,
    results: usize,
}

pub(crate) fn invoke(
    store: &mut Store,
    func_addr: usize,
    args: &[Value],
) -> Result<Vec<Value>, InvokeError> {
    match &store.funcs[func_addr] {
        FuncInst::Host { func, .. } => {
            let f = *func;
            f(args).map_err(InvokeError::Trap)
        }
        FuncInst::Wasm { .. } => {
            let mut m = Machine {
                store,
                stack: Vec::new(),
                frames: Vec::new(),
                ctrls: Vec::new(),
            };
            m.stack.extend_from_slice(args);
            m.push_frame(func_addr)
                .and_then(|_| m.run())
                .map_err(InvokeError::Trap)
        }
    }
}

pub(crate) struct Machine<'s> {
    pub store: &'s mut Store,
    stack: Vec<Value>,
    frames: Vec<Frame>,
    ctrls: Vec<Ctrl>,
}

type Exec = Result<(), Trap>;

fn trap(msg: &str) -> Trap {
    Trap::new(msg)
}

impl<'s> Machine<'s> {
    // ---- value stack helpers (types guaranteed by validation) ----
    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }
    fn pop(&mut self) -> Value {
        self.stack.pop().expect("validated stack")
    }
    fn pop_u32(&mut self) -> u32 {
        match self.pop() {
            Value::I32(v) => v,
            v => unreachable!("expected i32, got {v:?}"),
        }
    }
    fn pop_u64(&mut self) -> u64 {
        match self.pop() {
            Value::I64(v) => v,
            v => unreachable!("expected i64, got {v:?}"),
        }
    }
    fn pop_f32(&mut self) -> f32 {
        match self.pop() {
            Value::F32(v) => f32::from_bits(v),
            v => unreachable!("expected f32, got {v:?}"),
        }
    }
    fn pop_f64(&mut self) -> f64 {
        match self.pop() {
            Value::F64(v) => f64::from_bits(v),
            v => unreachable!("expected f64, got {v:?}"),
        }
    }
    pub(crate) fn pop_v128(&mut self) -> u128 {
        match self.pop() {
            Value::V128(v) => v,
            v => unreachable!("expected v128, got {v:?}"),
        }
    }
    fn push_i32(&mut self, v: u32) {
        self.push(Value::I32(v));
    }
    fn push_i64(&mut self, v: u64) {
        self.push(Value::I64(v));
    }
    fn push_f32(&mut self, v: f32) {
        self.push(Value::F32(v.to_bits()));
    }
    fn push_f32_bits(&mut self, v: u32) {
        self.push(Value::F32(v));
    }
    fn push_f64(&mut self, v: f64) {
        self.push(Value::F64(v.to_bits()));
    }
    fn push_f64_bits(&mut self, v: u64) {
        self.push(Value::F64(v));
    }
    pub(crate) fn push_v128(&mut self, v: u128) {
        self.push(Value::V128(v));
    }
    pub(crate) fn pop_value(&mut self) -> Value {
        self.pop()
    }
    pub(crate) fn push_value(&mut self, v: Value) {
        self.push(v);
    }
    fn push_bool(&mut self, b: bool) {
        self.push_i32(b as u32);
    }

    fn frame(&self) -> &Frame {
        self.frames.last().expect("active frame")
    }

    fn push_frame(&mut self, func_addr: usize) -> Exec {
        if self.frames.len() >= MAX_CALL_DEPTH {
            return Err(trap("call stack exhausted"));
        }
        let FuncInst::Wasm { ty, inst, code } = &self.store.funcs[func_addr] else {
            unreachable!("push_frame on host func");
        };
        let nparams = ty.params.len();
        let results = ty.results.len();
        let inst = *inst;
        let code = Rc::clone(code);
        let base = self.stack.len() - nparams;
        let mut locals: Vec<Value> = self.stack.split_off(base);
        for lt in &code.locals {
            locals.push(Value::default_for(*lt));
        }
        if self.stack.len() + locals.len() > MAX_VALUE_STACK {
            return Err(trap("call stack exhausted"));
        }
        let body_len = code.body.len();
        self.frames.push(Frame {
            code,
            inst,
            locals,
            pc: 0,
            base,
            ctrl_base: self.ctrls.len(),
            results,
        });
        // Function-level ctrl: branching to it returns from the function.
        self.ctrls.push(Ctrl {
            target: body_len,
            arity: results,
            height: self.stack.len(),
        });
        Ok(())
    }

    /// Pop the current frame, transferring `results` top values.
    fn pop_frame(&mut self) {
        let f = self.frames.pop().expect("frame");
        let keep = self.stack.split_off(self.stack.len() - f.results);
        self.stack.truncate(f.base);
        self.stack.extend(keep);
        self.ctrls.truncate(f.ctrl_base);
    }

    fn block_arity(&self, bt: &BlockType) -> (usize, usize) {
        match bt {
            BlockType::Empty => (0, 0),
            BlockType::Val(_) => (0, 1),
            BlockType::Type(i) => {
                let ft = &self.store.instances[self.frame().inst].types[*i as usize];
                (ft.params.len(), ft.results.len())
            }
        }
    }

    /// Branch to relative depth `d` within the current frame.
    fn do_branch(&mut self, d: u32) -> Option<usize> {
        let idx = self.ctrls.len() - 1 - d as usize;
        let target_is_function = idx == self.frame().ctrl_base;
        let ctrl = &self.ctrls[idx];
        let arity = ctrl.arity;
        let height = ctrl.height;
        let target = ctrl.target;
        let keep = self.stack.split_off(self.stack.len() - arity);
        self.stack.truncate(height);
        self.stack.extend(keep);
        self.ctrls.truncate(idx);
        if target_is_function {
            // Function return via branch.
            self.pop_frame_via_branch(arity);
            return None;
        }
        Some(target)
    }

    fn pop_frame_via_branch(&mut self, _arity: usize) {
        let f = self.frames.pop().expect("frame");
        let keep = self.stack.split_off(self.stack.len() - f.results);
        self.stack.truncate(f.base);
        self.stack.extend(keep);
        self.ctrls.truncate(f.ctrl_base);
    }

    // ---- memory helpers ----
    fn mem_addr(&self) -> usize {
        self.store.instances[self.frame().inst].mem_addrs[0]
    }

    /// Pop the base address, apply `offset`, and load `width` bytes,
    /// returned as a little-endian-packed u128 (low bytes significant).
    pub(crate) fn mem_load(&mut self, offset: u32, width: usize) -> Result<u128, Trap> {
        let base = self.pop_u32();
        let a = self.mem_addr();
        let data = &self.store.mems[a].data;
        let ea = u64::from(base) + u64::from(offset);
        if ea + width as u64 > data.len() as u64 {
            return Err(trap("out of bounds memory access"));
        }
        let mut buf = [0u8; 16];
        buf[..width].copy_from_slice(&data[ea as usize..ea as usize + width]);
        Ok(u128::from_le_bytes(buf))
    }

    pub(crate) fn mem_store(&mut self, offset: u32, bytes: &[u8]) -> Exec {
        let base = self.pop_u32();
        let a = self.mem_addr();
        let data = &mut self.store.mems[a].data;
        let ea = u64::from(base) + u64::from(offset);
        if ea + bytes.len() as u64 > data.len() as u64 {
            return Err(trap("out of bounds memory access"));
        }
        data[ea as usize..ea as usize + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    // ---- main loop ----
    pub(crate) fn run(&mut self) -> Result<Vec<Value>, Trap> {
        loop {
            let frame = self.frames.last_mut().expect("frame");
            if frame.pc >= frame.code.body.len() {
                // Implicit end of function body.
                self.pop_frame();
                if self.frames.is_empty() {
                    return Ok(std::mem::take(&mut self.stack));
                }
                continue;
            }
            let pc = frame.pc;
            frame.pc += 1;
            // The body is behind Rc; clone the handle to sidestep borrow
            // conflicts with the store during execution.
            let code = Rc::clone(&frame.code);
            let instr = &code.body[pc];
            match self.step(instr, pc)? {
                Flow::Next => {}
                Flow::Jump(t) => self.frames.last_mut().unwrap().pc = t,
                Flow::Returned => {
                    if self.frames.is_empty() {
                        return Ok(std::mem::take(&mut self.stack));
                    }
                }
            }
        }
    }

    fn step(&mut self, instr: &Instr, pc: usize) -> Result<Flow, Trap> {
        match instr {
            Instr::Unreachable => return Err(trap("unreachable")),
            Instr::Nop => {}
            Instr::Block { bt, end } => {
                let (params, results) = self.block_arity(bt);
                self.ctrls.push(Ctrl {
                    target: *end as usize,
                    arity: results,
                    height: self.stack.len() - params,
                });
            }
            Instr::Loop { bt, .. } => {
                let (params, _results) = self.block_arity(bt);
                self.ctrls.push(Ctrl {
                    target: pc,
                    arity: params,
                    height: self.stack.len() - params,
                });
            }
            Instr::If { bt, else_, end } => {
                let cond = self.pop_u32();
                let (params, results) = self.block_arity(bt);
                if cond != 0 {
                    self.ctrls.push(Ctrl {
                        target: *end as usize,
                        arity: results,
                        height: self.stack.len() - params,
                    });
                } else if *else_ == *end {
                    // No else arm: skip the whole construct (identity).
                    return Ok(Flow::Jump(*end as usize));
                } else {
                    self.ctrls.push(Ctrl {
                        target: *end as usize,
                        arity: results,
                        height: self.stack.len() - params,
                    });
                    return Ok(Flow::Jump(*else_ as usize));
                }
            }
            Instr::Else { end } => {
                // Fell off the true arm: exit the construct.
                self.ctrls.pop();
                return Ok(Flow::Jump(*end as usize));
            }
            Instr::End => {
                self.ctrls.pop();
            }
            Instr::Br { depth } => {
                return Ok(match self.do_branch(*depth) {
                    Some(t) => Flow::Jump(t),
                    None => Flow::Returned,
                });
            }
            Instr::BrIf { depth } => {
                if self.pop_u32() != 0 {
                    return Ok(match self.do_branch(*depth) {
                        Some(t) => Flow::Jump(t),
                        None => Flow::Returned,
                    });
                }
            }
            Instr::BrTable { depths, default } => {
                let i = self.pop_u32() as usize;
                let d = depths.get(i).copied().unwrap_or(*default);
                return Ok(match self.do_branch(d) {
                    Some(t) => Flow::Jump(t),
                    None => Flow::Returned,
                });
            }
            Instr::Return => {
                self.pop_frame();
                return Ok(Flow::Returned);
            }
            Instr::Call { func } => {
                let addr = self.store.instances[self.frame().inst].func_addrs[*func as usize];
                self.call(addr)?;
            }
            Instr::CallIndirect { table, ty } => {
                let i = self.pop_u32();
                let inst = self.frame().inst;
                let taddr = self.store.instances[inst].table_addrs[*table as usize];
                let tbl = &self.store.tables[taddr];
                let Some(v) = tbl.elems.get(i as usize) else {
                    return Err(trap("undefined element"));
                };
                let Value::FuncRef(r) = v else {
                    unreachable!("funcref table");
                };
                let Some(addr) = r else {
                    return Err(trap("uninitialized element"));
                };
                let addr = *addr as usize;
                let want = &self.store.instances[inst].types[*ty as usize];
                if self.store.funcs[addr].ty() != want {
                    return Err(trap("indirect call type mismatch"));
                }
                self.call(addr)?;
            }
            Instr::Drop => {
                self.pop();
            }
            Instr::Select { .. } | Instr::SelectT { .. } => {
                let c = self.pop_u32();
                let b = self.pop();
                let a = self.pop();
                self.push(if c != 0 { a } else { b });
            }
            Instr::LocalGet { idx } => {
                let v = self.frame().locals[*idx as usize];
                self.push(v);
            }
            Instr::LocalSet { idx } => {
                let v = self.pop();
                let f = self.frames.last_mut().unwrap();
                f.locals[*idx as usize] = v;
            }
            Instr::LocalTee { idx } => {
                let v = *self.stack.last().expect("validated");
                let f = self.frames.last_mut().unwrap();
                f.locals[*idx as usize] = v;
            }
            Instr::GlobalGet { idx } => {
                let a = self.store.instances[self.frame().inst].global_addrs[*idx as usize];
                self.push(self.store.globals[a].val);
            }
            Instr::GlobalSet { idx } => {
                let a = self.store.instances[self.frame().inst].global_addrs[*idx as usize];
                self.store.globals[a].val = self.pop();
            }
            Instr::TableGet { table } => {
                let a = self.store.instances[self.frame().inst].table_addrs[*table as usize];
                let i = self.pop_u32();
                match self.store.tables[a].elems.get(i as usize) {
                    Some(v) => self.push(*v),
                    None => return Err(trap("out of bounds table access")),
                }
            }
            Instr::TableSet { table } => {
                let a = self.store.instances[self.frame().inst].table_addrs[*table as usize];
                let v = self.pop();
                let i = self.pop_u32();
                match self.store.tables[a].elems.get_mut(i as usize) {
                    Some(slot) => *slot = v,
                    None => return Err(trap("out of bounds table access")),
                }
            }
            Instr::TableInit { table, elem } => {
                let inst = self.frame().inst;
                let ta = self.store.instances[inst].table_addrs[*table as usize];
                let n = self.pop_u32();
                let s = self.pop_u32();
                let d = self.pop_u32();
                let seg = &self.store.instances[inst].elems[*elem as usize].elems;
                if u64::from(s) + u64::from(n) > seg.len() as u64
                    || u64::from(d) + u64::from(n) > self.store.tables[ta].elems.len() as u64
                {
                    return Err(trap("out of bounds table access"));
                }
                let src: Vec<Value> = seg[s as usize..(s + n) as usize].to_vec();
                self.store.tables[ta].elems[d as usize..(d + n) as usize].copy_from_slice(&src);
            }
            Instr::ElemDrop { elem } => {
                let inst = self.frame().inst;
                self.store.instances[inst].elems[*elem as usize]
                    .elems
                    .clear();
            }
            Instr::TableCopy { dst, src } => {
                let inst = self.frame().inst;
                let da = self.store.instances[inst].table_addrs[*dst as usize];
                let sa = self.store.instances[inst].table_addrs[*src as usize];
                let n = self.pop_u32();
                let s = self.pop_u32();
                let d = self.pop_u32();
                if u64::from(s) + u64::from(n) > self.store.tables[sa].elems.len() as u64
                    || u64::from(d) + u64::from(n) > self.store.tables[da].elems.len() as u64
                {
                    return Err(trap("out of bounds table access"));
                }
                let chunk: Vec<Value> =
                    self.store.tables[sa].elems[s as usize..(s + n) as usize].to_vec();
                self.store.tables[da].elems[d as usize..(d + n) as usize].copy_from_slice(&chunk);
            }
            Instr::TableGrow { table } => {
                let a = self.store.instances[self.frame().inst].table_addrs[*table as usize];
                let n = self.pop_u32();
                let v = self.pop();
                let tbl = &mut self.store.tables[a];
                let old = tbl.elems.len() as u64;
                let new = old + u64::from(n);
                let max = tbl.ty.limits.max.map_or(u64::from(u32::MAX), u64::from);
                // Growth beyond the implementation limit fails with -1,
                // which the spec explicitly permits for table.grow.
                if new > max || new > TABLE_IMPL_LIMIT {
                    self.push_i32(u32::MAX);
                } else {
                    tbl.elems.resize(new as usize, v);
                    self.push_i32(old as u32);
                }
            }
            Instr::TableSize { table } => {
                let a = self.store.instances[self.frame().inst].table_addrs[*table as usize];
                self.push_i32(self.store.tables[a].elems.len() as u32);
            }
            Instr::TableFill { table } => {
                let a = self.store.instances[self.frame().inst].table_addrs[*table as usize];
                let n = self.pop_u32();
                let v = self.pop();
                let i = self.pop_u32();
                let tbl = &mut self.store.tables[a];
                if u64::from(i) + u64::from(n) > tbl.elems.len() as u64 {
                    return Err(trap("out of bounds table access"));
                }
                tbl.elems[i as usize..(i + n) as usize].fill(v);
            }
            Instr::Load { op, offset, .. } => {
                use LoadOp::*;
                let v = match op {
                    I32 => Value::I32(self.mem_load(*offset, 4)? as u32),
                    I64 => Value::I64(self.mem_load(*offset, 8)? as u64),
                    F32 => Value::F32(self.mem_load(*offset, 4)? as u32),
                    F64 => Value::F64(self.mem_load(*offset, 8)? as u64),
                    I32L8S => Value::I32(self.mem_load(*offset, 1)? as u8 as i8 as i32 as u32),
                    I32L8U => Value::I32(self.mem_load(*offset, 1)? as u8 as u32),
                    I32L16S => Value::I32(self.mem_load(*offset, 2)? as u16 as i16 as i32 as u32),
                    I32L16U => Value::I32(self.mem_load(*offset, 2)? as u16 as u32),
                    I64L8S => Value::I64(self.mem_load(*offset, 1)? as u8 as i8 as i64 as u64),
                    I64L8U => Value::I64(self.mem_load(*offset, 1)? as u8 as u64),
                    I64L16S => Value::I64(self.mem_load(*offset, 2)? as u16 as i16 as i64 as u64),
                    I64L16U => Value::I64(self.mem_load(*offset, 2)? as u16 as u64),
                    I64L32S => Value::I64(self.mem_load(*offset, 4)? as u32 as i32 as i64 as u64),
                    I64L32U => Value::I64(self.mem_load(*offset, 4)? as u32 as u64),
                };
                self.push(v);
            }
            Instr::Store { op, offset, .. } => {
                use StoreOp::*;
                match op {
                    I32 => {
                        let v = self.pop_u32();
                        self.mem_store(*offset, &v.to_le_bytes())?;
                    }
                    I64 => {
                        let v = self.pop_u64();
                        self.mem_store(*offset, &v.to_le_bytes())?;
                    }
                    F32 => {
                        let v = match self.pop() {
                            Value::F32(b) => b,
                            _ => unreachable!(),
                        };
                        self.mem_store(*offset, &v.to_le_bytes())?;
                    }
                    F64 => {
                        let v = match self.pop() {
                            Value::F64(b) => b,
                            _ => unreachable!(),
                        };
                        self.mem_store(*offset, &v.to_le_bytes())?;
                    }
                    I32S8 => {
                        let v = self.pop_u32();
                        self.mem_store(*offset, &[v as u8])?;
                    }
                    I32S16 => {
                        let v = self.pop_u32();
                        self.mem_store(*offset, &(v as u16).to_le_bytes())?;
                    }
                    I64S8 => {
                        let v = self.pop_u64();
                        self.mem_store(*offset, &[v as u8])?;
                    }
                    I64S16 => {
                        let v = self.pop_u64();
                        self.mem_store(*offset, &(v as u16).to_le_bytes())?;
                    }
                    I64S32 => {
                        let v = self.pop_u64();
                        self.mem_store(*offset, &(v as u32).to_le_bytes())?;
                    }
                }
            }
            Instr::MemorySize => {
                let a = self.mem_addr();
                self.push_i32((self.store.mems[a].data.len() / PAGE_SIZE) as u32);
            }
            Instr::MemoryGrow => {
                let a = self.mem_addr();
                let n = self.pop_u32();
                let mem = &mut self.store.mems[a];
                let old = (mem.data.len() / PAGE_SIZE) as u32;
                let new = u64::from(old) + u64::from(n);
                let max = u64::from(mem.ty.limits.max.unwrap_or(MAX_PAGES).min(MAX_PAGES));
                match mem_bytes(new as u32) {
                    Some(bytes) if new <= max => {
                        mem.data.resize(bytes, 0);
                        self.push_i32(old);
                    }
                    _ => self.push_i32(u32::MAX),
                }
            }
            Instr::MemoryInit { data } => {
                let inst = self.frame().inst;
                let a = self.mem_addr();
                let n = self.pop_u32();
                let s = self.pop_u32();
                let d = self.pop_u32();
                let seg = &self.store.instances[inst].datas[*data as usize].bytes;
                if u64::from(s) + u64::from(n) > seg.len() as u64
                    || u64::from(d) + u64::from(n) > self.store.mems[a].data.len() as u64
                {
                    return Err(trap("out of bounds memory access"));
                }
                let chunk: Vec<u8> = seg[s as usize..(s + n) as usize].to_vec();
                self.store.mems[a].data[d as usize..(d + n) as usize].copy_from_slice(&chunk);
            }
            Instr::DataDrop { data } => {
                let inst = self.frame().inst;
                self.store.instances[inst].datas[*data as usize]
                    .bytes
                    .clear();
            }
            Instr::MemoryCopy => {
                let a = self.mem_addr();
                let n = self.pop_u32();
                let s = self.pop_u32();
                let d = self.pop_u32();
                let mem = &mut self.store.mems[a];
                if u64::from(s) + u64::from(n) > mem.data.len() as u64
                    || u64::from(d) + u64::from(n) > mem.data.len() as u64
                {
                    return Err(trap("out of bounds memory access"));
                }
                mem.data
                    .copy_within(s as usize..(s + n) as usize, d as usize);
            }
            Instr::MemoryFill => {
                let a = self.mem_addr();
                let n = self.pop_u32();
                let v = self.pop_u32() as u8;
                let d = self.pop_u32();
                let mem = &mut self.store.mems[a];
                if u64::from(d) + u64::from(n) > mem.data.len() as u64 {
                    return Err(trap("out of bounds memory access"));
                }
                mem.data[d as usize..(d + n) as usize].fill(v);
            }
            Instr::I32Const(v) => self.push_i32(*v),
            Instr::I64Const(v) => self.push_i64(*v),
            Instr::F32Const(v) => self.push_f32_bits(*v),
            Instr::F64Const(v) => self.push_f64_bits(*v),
            Instr::V128Const(v) => self.push_v128(*v),
            Instr::RefNull(t) => self.push(Value::null_of(*t)),
            Instr::RefIsNull => {
                let v = self.pop();
                let is_null = matches!(v, Value::FuncRef(None) | Value::ExternRef(None));
                self.push_bool(is_null);
            }
            Instr::RefFunc { func } => {
                let addr = self.store.instances[self.frame().inst].func_addrs[*func as usize];
                self.push(Value::FuncRef(Some(addr as u32)));
            }
            Instr::Num(op) => self.num(*op)?,
            Instr::Simd(op) => crate::simd::simd_op(self, *op)?,
            Instr::SimdLane { op, lane } => crate::simd::simd_lane(self, *op, *lane)?,
            Instr::Shuffle(lanes) => crate::simd::shuffle(self, lanes)?,
            Instr::SimdMem {
                op, offset, lane, ..
            } => crate::simd::simd_mem(self, *op, *offset, *lane)?,
        }
        Ok(Flow::Next)
    }

    fn call(&mut self, addr: usize) -> Exec {
        match &self.store.funcs[addr] {
            FuncInst::Host { ty, func } => {
                let f = *func;
                let n = ty.params.len();
                let args: Vec<Value> = self.stack.split_off(self.stack.len() - n);
                let results = f(&args)?;
                self.stack.extend(results);
                Ok(())
            }
            FuncInst::Wasm { .. } => self.push_frame(addr),
        }
    }

    // ---- numeric ops ----
    #[allow(clippy::too_many_lines)]
    fn num(&mut self, op: NumOp) -> Exec {
        use NumOp::*;
        match op {
            I32Eqz => {
                let a = self.pop_u32();
                self.push_bool(a == 0);
            }
            I64Eqz => {
                let a = self.pop_u64();
                self.push_bool(a == 0);
            }
            I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS
            | I32GeU => {
                let b = self.pop_u32();
                let a = self.pop_u32();
                let (sa, sb) = (a as i32, b as i32);
                self.push_bool(match op {
                    I32Eq => a == b,
                    I32Ne => a != b,
                    I32LtS => sa < sb,
                    I32LtU => a < b,
                    I32GtS => sa > sb,
                    I32GtU => a > b,
                    I32LeS => sa <= sb,
                    I32LeU => a <= b,
                    I32GeS => sa >= sb,
                    _ => a >= b,
                });
            }
            I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS
            | I64GeU => {
                let b = self.pop_u64();
                let a = self.pop_u64();
                let (sa, sb) = (a as i64, b as i64);
                self.push_bool(match op {
                    I64Eq => a == b,
                    I64Ne => a != b,
                    I64LtS => sa < sb,
                    I64LtU => a < b,
                    I64GtS => sa > sb,
                    I64GtU => a > b,
                    I64LeS => sa <= sb,
                    I64LeU => a <= b,
                    I64GeS => sa >= sb,
                    _ => a >= b,
                });
            }
            F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge => {
                let b = self.pop_f32();
                let a = self.pop_f32();
                self.push_bool(match op {
                    F32Eq => a == b,
                    F32Ne => a != b,
                    F32Lt => a < b,
                    F32Gt => a > b,
                    F32Le => a <= b,
                    _ => a >= b,
                });
            }
            F64Eq | F64Ne | F64Lt | F64Gt | F64Le | F64Ge => {
                let b = self.pop_f64();
                let a = self.pop_f64();
                self.push_bool(match op {
                    F64Eq => a == b,
                    F64Ne => a != b,
                    F64Lt => a < b,
                    F64Gt => a > b,
                    F64Le => a <= b,
                    _ => a >= b,
                });
            }
            I32Clz => {
                let a = self.pop_u32();
                self.push_i32(a.leading_zeros());
            }
            I32Ctz => {
                let a = self.pop_u32();
                self.push_i32(a.trailing_zeros());
            }
            I32Popcnt => {
                let a = self.pop_u32();
                self.push_i32(a.count_ones());
            }
            I32Add | I32Sub | I32Mul | I32And | I32Or | I32Xor | I32Shl | I32ShrS | I32ShrU
            | I32Rotl | I32Rotr => {
                let b = self.pop_u32();
                let a = self.pop_u32();
                self.push_i32(match op {
                    I32Add => a.wrapping_add(b),
                    I32Sub => a.wrapping_sub(b),
                    I32Mul => a.wrapping_mul(b),
                    I32And => a & b,
                    I32Or => a | b,
                    I32Xor => a ^ b,
                    I32Shl => a.wrapping_shl(b),
                    I32ShrS => ((a as i32).wrapping_shr(b)) as u32,
                    I32ShrU => a.wrapping_shr(b),
                    I32Rotl => a.rotate_left(b & 31),
                    _ => a.rotate_right(b & 31),
                });
            }
            I32DivS => {
                let b = self.pop_u32() as i32;
                let a = self.pop_u32() as i32;
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                let (q, ov) = a.overflowing_div(b);
                if ov {
                    return Err(trap("integer overflow"));
                }
                self.push_i32(q as u32);
            }
            I32DivU => {
                let b = self.pop_u32();
                let a = self.pop_u32();
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i32(a / b);
            }
            I32RemS => {
                let b = self.pop_u32() as i32;
                let a = self.pop_u32() as i32;
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i32(a.wrapping_rem(b) as u32);
            }
            I32RemU => {
                let b = self.pop_u32();
                let a = self.pop_u32();
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i32(a % b);
            }
            I64Clz => {
                let a = self.pop_u64();
                self.push_i64(u64::from(a.leading_zeros()));
            }
            I64Ctz => {
                let a = self.pop_u64();
                self.push_i64(u64::from(a.trailing_zeros()));
            }
            I64Popcnt => {
                let a = self.pop_u64();
                self.push_i64(u64::from(a.count_ones()));
            }
            I64Add | I64Sub | I64Mul | I64And | I64Or | I64Xor | I64Shl | I64ShrS | I64ShrU
            | I64Rotl | I64Rotr => {
                let b = self.pop_u64();
                let a = self.pop_u64();
                self.push_i64(match op {
                    I64Add => a.wrapping_add(b),
                    I64Sub => a.wrapping_sub(b),
                    I64Mul => a.wrapping_mul(b),
                    I64And => a & b,
                    I64Or => a | b,
                    I64Xor => a ^ b,
                    I64Shl => a.wrapping_shl(b as u32),
                    I64ShrS => ((a as i64).wrapping_shr(b as u32)) as u64,
                    I64ShrU => a.wrapping_shr(b as u32),
                    I64Rotl => a.rotate_left((b & 63) as u32),
                    _ => a.rotate_right((b & 63) as u32),
                });
            }
            I64DivS => {
                let b = self.pop_u64() as i64;
                let a = self.pop_u64() as i64;
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                let (q, ov) = a.overflowing_div(b);
                if ov {
                    return Err(trap("integer overflow"));
                }
                self.push_i64(q as u64);
            }
            I64DivU => {
                let b = self.pop_u64();
                let a = self.pop_u64();
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i64(a / b);
            }
            I64RemS => {
                let b = self.pop_u64() as i64;
                let a = self.pop_u64() as i64;
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i64(a.wrapping_rem(b) as u64);
            }
            I64RemU => {
                let b = self.pop_u64();
                let a = self.pop_u64();
                if b == 0 {
                    return Err(trap("integer divide by zero"));
                }
                self.push_i64(a % b);
            }
            F32Abs => {
                let a = self.pop_f32();
                self.push_f32_bits(a.to_bits() & 0x7fff_ffff);
            }
            F32Neg => {
                let a = self.pop_f32();
                self.push_f32_bits(a.to_bits() ^ 0x8000_0000);
            }
            F32Ceil => {
                let a = self.pop_f32();
                self.push_f32(ceil32(a));
            }
            F32Floor => {
                let a = self.pop_f32();
                self.push_f32(floor32(a));
            }
            F32Trunc => {
                let a = self.pop_f32();
                self.push_f32(trunc32(a));
            }
            F32Nearest => {
                let a = self.pop_f32();
                self.push_f32(nearest32(a));
            }
            F32Sqrt => {
                let a = self.pop_f32();
                self.push_f32(a.sqrt());
            }
            F32Add | F32Sub | F32Mul | F32Div => {
                let b = self.pop_f32();
                let a = self.pop_f32();
                self.push_f32(match op {
                    F32Add => a + b,
                    F32Sub => a - b,
                    F32Mul => a * b,
                    _ => a / b,
                });
            }
            F32Min => {
                let b = self.pop_f32();
                let a = self.pop_f32();
                self.push_f32(fmin32(a, b));
            }
            F32Max => {
                let b = self.pop_f32();
                let a = self.pop_f32();
                self.push_f32(fmax32(a, b));
            }
            F32Copysign => {
                let b = self.pop_f32();
                let a = self.pop_f32();
                self.push_f32_bits((a.to_bits() & 0x7fff_ffff) | (b.to_bits() & 0x8000_0000));
            }
            F64Abs => {
                let a = self.pop_f64();
                self.push_f64_bits(a.to_bits() & 0x7fff_ffff_ffff_ffff);
            }
            F64Neg => {
                let a = self.pop_f64();
                self.push_f64_bits(a.to_bits() ^ 0x8000_0000_0000_0000);
            }
            F64Ceil => {
                let a = self.pop_f64();
                self.push_f64(ceil64(a));
            }
            F64Floor => {
                let a = self.pop_f64();
                self.push_f64(floor64(a));
            }
            F64Trunc => {
                let a = self.pop_f64();
                self.push_f64(trunc64(a));
            }
            F64Nearest => {
                let a = self.pop_f64();
                self.push_f64(nearest64(a));
            }
            F64Sqrt => {
                let a = self.pop_f64();
                self.push_f64(a.sqrt());
            }
            F64Add | F64Sub | F64Mul | F64Div => {
                let b = self.pop_f64();
                let a = self.pop_f64();
                self.push_f64(match op {
                    F64Add => a + b,
                    F64Sub => a - b,
                    F64Mul => a * b,
                    _ => a / b,
                });
            }
            F64Min => {
                let b = self.pop_f64();
                let a = self.pop_f64();
                self.push_f64(fmin64(a, b));
            }
            F64Max => {
                let b = self.pop_f64();
                let a = self.pop_f64();
                self.push_f64(fmax64(a, b));
            }
            F64Copysign => {
                let b = self.pop_f64();
                let a = self.pop_f64();
                self.push_f64_bits(
                    (a.to_bits() & 0x7fff_ffff_ffff_ffff) | (b.to_bits() & 0x8000_0000_0000_0000),
                );
            }
            I32WrapI64 => {
                let a = self.pop_u64();
                self.push_i32(a as u32);
            }
            I32TruncF32S => {
                let a = self.pop_f32();
                self.push_i32(trunc_to_i32(f64::from(a))? as u32);
            }
            I32TruncF32U => {
                let a = self.pop_f32();
                self.push_i32(trunc_to_u32(f64::from(a))?);
            }
            I32TruncF64S => {
                let a = self.pop_f64();
                self.push_i32(trunc_to_i32(a)? as u32);
            }
            I32TruncF64U => {
                let a = self.pop_f64();
                self.push_i32(trunc_to_u32(a)?);
            }
            I64ExtendI32S => {
                let a = self.pop_u32();
                self.push_i64(a as i32 as i64 as u64);
            }
            I64ExtendI32U => {
                let a = self.pop_u32();
                self.push_i64(u64::from(a));
            }
            I64TruncF32S => {
                let a = self.pop_f32();
                self.push_i64(trunc_f32_to_i64(a)? as u64);
            }
            I64TruncF32U => {
                let a = self.pop_f32();
                self.push_i64(trunc_f32_to_u64(a)?);
            }
            I64TruncF64S => {
                let a = self.pop_f64();
                self.push_i64(trunc_f64_to_i64(a)? as u64);
            }
            I64TruncF64U => {
                let a = self.pop_f64();
                self.push_i64(trunc_f64_to_u64(a)?);
            }
            F32ConvertI32S => {
                let a = self.pop_u32();
                self.push_f32(a as i32 as f32);
            }
            F32ConvertI32U => {
                let a = self.pop_u32();
                self.push_f32(a as f32);
            }
            F32ConvertI64S => {
                let a = self.pop_u64();
                self.push_f32(a as i64 as f32);
            }
            F32ConvertI64U => {
                let a = self.pop_u64();
                self.push_f32(a as f32);
            }
            F32DemoteF64 => {
                let a = self.pop_f64();
                self.push_f32(a as f32);
            }
            F64ConvertI32S => {
                let a = self.pop_u32();
                self.push_f64(f64::from(a as i32));
            }
            F64ConvertI32U => {
                let a = self.pop_u32();
                self.push_f64(f64::from(a));
            }
            F64ConvertI64S => {
                let a = self.pop_u64();
                self.push_f64(a as i64 as f64);
            }
            F64ConvertI64U => {
                let a = self.pop_u64();
                self.push_f64(a as f64);
            }
            F64PromoteF32 => {
                let a = self.pop_f32();
                self.push_f64(f64::from(a));
            }
            I32ReinterpretF32 => {
                let v = match self.pop() {
                    Value::F32(b) => b,
                    _ => unreachable!(),
                };
                self.push_i32(v);
            }
            I64ReinterpretF64 => {
                let v = match self.pop() {
                    Value::F64(b) => b,
                    _ => unreachable!(),
                };
                self.push_i64(v);
            }
            F32ReinterpretI32 => {
                let v = self.pop_u32();
                self.push_f32_bits(v);
            }
            F64ReinterpretI64 => {
                let v = self.pop_u64();
                self.push_f64_bits(v);
            }
            I32Extend8S => {
                let a = self.pop_u32();
                self.push_i32(a as i8 as i32 as u32);
            }
            I32Extend16S => {
                let a = self.pop_u32();
                self.push_i32(a as i16 as i32 as u32);
            }
            I64Extend8S => {
                let a = self.pop_u64();
                self.push_i64(a as i8 as i64 as u64);
            }
            I64Extend16S => {
                let a = self.pop_u64();
                self.push_i64(a as i16 as i64 as u64);
            }
            I64Extend32S => {
                let a = self.pop_u64();
                self.push_i64(a as i32 as i64 as u64);
            }
            I32TruncSatF32S => {
                let a = self.pop_f32();
                self.push_i32(a as i32 as u32);
            }
            I32TruncSatF32U => {
                let a = self.pop_f32();
                self.push_i32(a as u32);
            }
            I32TruncSatF64S => {
                let a = self.pop_f64();
                self.push_i32(a as i32 as u32);
            }
            I32TruncSatF64U => {
                let a = self.pop_f64();
                self.push_i32(a as u32);
            }
            I64TruncSatF32S => {
                let a = self.pop_f32();
                self.push_i64(a as i64 as u64);
            }
            I64TruncSatF32U => {
                let a = self.pop_f32();
                self.push_i64(a as u64);
            }
            I64TruncSatF64S => {
                let a = self.pop_f64();
                self.push_i64(a as i64 as u64);
            }
            I64TruncSatF64U => {
                let a = self.pop_f64();
                self.push_i64(a as u64);
            }
        }
        Ok(())
    }
}

enum Flow {
    Next,
    Jump(usize),
    Returned,
}

// ---- float min/max per spec (NaN wins, -0 < +0) ----
pub(crate) fn fmin32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return quiet32(a);
    }
    if b.is_nan() {
        return quiet32(b);
    }
    if a == b {
        // Break the 0.0 == -0.0 tie towards the negative zero.
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b {
        a
    } else {
        b
    }
}

pub(crate) fn fmax32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return quiet32(a);
    }
    if b.is_nan() {
        return quiet32(b);
    }
    if a == b {
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b {
        a
    } else {
        b
    }
}

pub(crate) fn fmin64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return quiet64(a);
    }
    if b.is_nan() {
        return quiet64(b);
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b {
        a
    } else {
        b
    }
}

pub(crate) fn fmax64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return quiet64(a);
    }
    if b.is_nan() {
        return quiet64(b);
    }
    if a == b {
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b {
        a
    } else {
        b
    }
}

fn quiet32(a: f32) -> f32 {
    f32::from_bits(a.to_bits() | 0x0040_0000)
}

fn quiet64(a: f64) -> f64 {
    f64::from_bits(a.to_bits() | 0x0008_0000_0000_0000)
}

// Rounding operators must return an *arithmetic* (quiet) NaN for NaN
// inputs. Hardware round instructions do this, but the libm fallbacks on
// some platforms (e.g. glibc ceilf on SSE2 baseline) return signaling
// NaNs unchanged — so the quieting is made explicit here.
pub(crate) fn ceil32(a: f32) -> f32 {
    if a.is_nan() {
        quiet32(a)
    } else {
        a.ceil()
    }
}
pub(crate) fn floor32(a: f32) -> f32 {
    if a.is_nan() {
        quiet32(a)
    } else {
        a.floor()
    }
}
pub(crate) fn trunc32(a: f32) -> f32 {
    if a.is_nan() {
        quiet32(a)
    } else {
        a.trunc()
    }
}
pub(crate) fn nearest32(a: f32) -> f32 {
    if a.is_nan() {
        quiet32(a)
    } else {
        a.round_ties_even()
    }
}
pub(crate) fn ceil64(a: f64) -> f64 {
    if a.is_nan() {
        quiet64(a)
    } else {
        a.ceil()
    }
}
pub(crate) fn floor64(a: f64) -> f64 {
    if a.is_nan() {
        quiet64(a)
    } else {
        a.floor()
    }
}
pub(crate) fn trunc64(a: f64) -> f64 {
    if a.is_nan() {
        quiet64(a)
    } else {
        a.trunc()
    }
}
pub(crate) fn nearest64(a: f64) -> f64 {
    if a.is_nan() {
        quiet64(a)
    } else {
        a.round_ties_even()
    }
}

// ---- trapping float->int truncation ----
fn trunc_check(a: f64) -> Result<f64, Trap> {
    if a.is_nan() {
        return Err(trap("invalid conversion to integer"));
    }
    Ok(a.trunc())
}

fn trunc_to_i32(a: f64) -> Result<i32, Trap> {
    let t = trunc_check(a)?;
    if !(-2147483648.0..=2147483647.0).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as i32)
}

fn trunc_to_u32(a: f64) -> Result<u32, Trap> {
    let t = trunc_check(a)?;
    if !(0.0..=4294967295.0).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as u32)
}

fn trunc_f32_to_i64(a: f32) -> Result<i64, Trap> {
    if a.is_nan() {
        return Err(trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    // i64 range in f32: [-2^63, 2^63) — the upper bound is exact in f32.
    if !(-9223372036854775808.0f32..9223372036854775808.0f32).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as i64)
}

fn trunc_f32_to_u64(a: f32) -> Result<u64, Trap> {
    if a.is_nan() {
        return Err(trap("invalid conversion to integer"));
    }
    let t = a.trunc();
    if !(0.0f32..18446744073709551616.0f32).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as u64)
}

fn trunc_f64_to_i64(a: f64) -> Result<i64, Trap> {
    let t = trunc_check(a)?;
    if !(-9223372036854775808.0..9223372036854775808.0).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as i64)
}

fn trunc_f64_to_u64(a: f64) -> Result<u64, Trap> {
    let t = trunc_check(a)?;
    if !(0.0..18446744073709551616.0).contains(&t) {
        return Err(trap("integer overflow"));
    }
    Ok(t as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rounding a signaling NaN must yield an arithmetic (quiet) NaN on
    // every platform; the glibc SSE2-baseline libm fallbacks return
    // signaling NaNs unchanged, which CI caught as 32 corpus failures.
    #[test]
    fn rounding_quiets_signaling_nans() {
        let snan32 = f32::from_bits(0x7fa0_0000);
        for f in [ceil32, floor32, trunc32, nearest32] {
            let r = f(snan32);
            assert!(
                r.is_nan() && r.to_bits() & 0x0040_0000 != 0,
                "{:#x}",
                r.to_bits()
            );
        }
        let snan64 = f64::from_bits(0x7ff4_0000_0000_0000);
        for f in [ceil64, floor64, trunc64, nearest64] {
            let r = f(snan64);
            assert!(
                r.is_nan() && r.to_bits() & 0x0008_0000_0000_0000 != 0,
                "{:#x}",
                r.to_bits()
            );
        }
    }
}
