//! Store: runtime instances of modules and host modules, instantiation
//! per the wg-2.0 semantics (import matching, global init evaluation,
//! in-order active segment application with partial effects, start).

use std::rc::Rc;

use crate::error::{InstError, InvokeError, Trap};
use crate::module::{Code, ConstExpr, DataMode, ElemMode, ExportDesc, ImportDesc, Instr, Module};
use crate::types::{FuncType, GlobalType, Limits, MemType, RefType, TableType};
use crate::values::Value;

pub type InstanceId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternVal {
    Func(usize),
    Table(usize),
    Mem(usize),
    Global(usize),
}

pub type HostFunc = fn(&[Value]) -> Result<Vec<Value>, Trap>;

/// Description of a host-provided module (e.g. spectest).
pub struct HostModule {
    pub funcs: Vec<(String, FuncType, HostFunc)>,
    pub globals: Vec<(String, GlobalType, Value)>,
    pub tables: Vec<(String, TableType)>,
    pub mems: Vec<(String, MemType)>,
}

pub(crate) enum FuncInst {
    Host {
        ty: FuncType,
        func: HostFunc,
    },
    Wasm {
        ty: FuncType,
        inst: InstanceId,
        code: Rc<Code>,
    },
}

impl FuncInst {
    pub(crate) fn ty(&self) -> &FuncType {
        match self {
            FuncInst::Host { ty, .. } => ty,
            FuncInst::Wasm { ty, .. } => ty,
        }
    }
}

pub(crate) struct TableInst {
    pub ty: TableType,
    pub elems: Vec<Value>,
}

pub(crate) struct MemInst {
    pub ty: MemType,
    pub data: Vec<u8>,
}

pub(crate) struct GlobalInst {
    pub ty: GlobalType,
    pub val: Value,
}

/// Per-instance element segment contents (droppable).
pub(crate) struct ElemInst {
    pub elems: Vec<Value>,
}

pub(crate) struct DataInst {
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Instance {
    pub exports: Vec<(String, ExternVal)>,
    pub types: Vec<FuncType>,
    pub func_addrs: Vec<usize>,
    pub table_addrs: Vec<usize>,
    pub mem_addrs: Vec<usize>,
    pub global_addrs: Vec<usize>,
    pub elems: Vec<ElemInst>,
    pub datas: Vec<DataInst>,
}

#[derive(Default)]
pub struct Store {
    pub(crate) funcs: Vec<FuncInst>,
    pub(crate) tables: Vec<TableInst>,
    pub(crate) mems: Vec<MemInst>,
    pub(crate) globals: Vec<GlobalInst>,
    pub(crate) instances: Vec<Instance>,
}

pub const PAGE_SIZE: usize = 65536;
pub const MAX_PAGES: u32 = 65536;
/// Implementation limit on table element counts (the spec permits
/// allocation failure); keeps a 2^32-min table from OOMing the host.
pub const TABLE_IMPL_LIMIT: u64 = 1 << 27;

/// Byte size of `pages` memory pages, overflow-safe on 32-bit hosts.
pub(crate) fn mem_bytes(pages: u32) -> Option<usize> {
    usize::try_from(u64::from(pages) * PAGE_SIZE as u64).ok()
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    pub fn add_host_module(&mut self, desc: HostModule) -> Result<InstanceId, String> {
        let mut inst = Instance::default();
        for (name, ty, func) in desc.funcs {
            let addr = self.funcs.len();
            self.funcs.push(FuncInst::Host { ty, func });
            inst.exports.push((name, ExternVal::Func(addr)));
            inst.func_addrs.push(addr);
        }
        for (name, ty, val) in desc.globals {
            // Host-supplied values are untrusted with respect to engine
            // invariants: a declared/actual type mismatch would later
            // panic typed stack helpers inside validated code, and a
            // forged function reference would panic call_indirect.
            if val.ty() != ty.val {
                return Err(format!(
                    "host global {name}: value type {:?} != declared {:?}",
                    val.ty(),
                    ty.val
                ));
            }
            if let Value::FuncRef(Some(a)) = val {
                if a as usize >= self.funcs.len() {
                    return Err(format!("host global {name}: invalid function reference"));
                }
            }
            let addr = self.globals.len();
            self.globals.push(GlobalInst { ty, val });
            inst.exports.push((name, ExternVal::Global(addr)));
            inst.global_addrs.push(addr);
        }
        for (name, ty) in desc.tables {
            let addr = self.tables.len();
            if u64::from(ty.limits.min) > TABLE_IMPL_LIMIT {
                return Err(format!("host table {name}: exceeds implementation limit"));
            }
            let mut elems = Vec::new();
            if elems.try_reserve_exact(ty.limits.min as usize).is_err() {
                return Err(format!("host table {name}: cannot allocate"));
            }
            elems.resize(ty.limits.min as usize, Value::null_of(ty.elem));
            self.tables.push(TableInst { ty, elems });
            inst.exports.push((name, ExternVal::Table(addr)));
            inst.table_addrs.push(addr);
        }
        for (name, ty) in desc.mems {
            let addr = self.mems.len();
            let bytes = mem_bytes(ty.limits.min).ok_or("host memory size overflow")?;
            let mut data = Vec::new();
            if data.try_reserve_exact(bytes).is_err() {
                return Err(format!("host memory {name}: cannot allocate"));
            }
            data.resize(bytes, 0);
            self.mems.push(MemInst { ty, data });
            inst.exports.push((name, ExternVal::Mem(addr)));
            inst.mem_addrs.push(addr);
        }
        self.instances.push(inst);
        Ok(self.instances.len() - 1)
    }

    pub fn instantiate(
        &mut self,
        module: &Module,
        resolve: &mut dyn FnMut(&str, &str) -> Option<ExternVal>,
    ) -> Result<InstanceId, InstError> {
        let mut inst = Instance {
            types: module.types.clone(),
            ..Default::default()
        };

        // 1. Resolve and type-check imports.
        for imp in &module.imports {
            let Some(ev) = resolve(&imp.module, &imp.name) else {
                return Err(InstError::Link(format!(
                    "unknown import {}.{}",
                    imp.module, imp.name
                )));
            };
            // The resolver is an untrusted callback: its addresses must be
            // validated before indexing the store.
            let in_bounds = match ev {
                ExternVal::Func(a) => a < self.funcs.len(),
                ExternVal::Table(a) => a < self.tables.len(),
                ExternVal::Mem(a) => a < self.mems.len(),
                ExternVal::Global(a) => a < self.globals.len(),
            };
            if !in_bounds {
                return Err(InstError::Link(
                    "resolved import address out of bounds".into(),
                ));
            }
            match (imp.desc, ev) {
                (ImportDesc::Func(tidx), ExternVal::Func(addr)) => {
                    let want = &module.types[tidx as usize];
                    if self.funcs[addr].ty() != want {
                        return Err(InstError::Link("incompatible import type".into()));
                    }
                    inst.func_addrs.push(addr);
                }
                (ImportDesc::Table(want), ExternVal::Table(addr)) => {
                    let have = &self.tables[addr];
                    let have_limits = Limits {
                        min: have.elems.len() as u32,
                        max: have.ty.limits.max,
                    };
                    if have.ty.elem != want.elem || !limits_match(&have_limits, &want.limits) {
                        return Err(InstError::Link("incompatible import type".into()));
                    }
                    inst.table_addrs.push(addr);
                }
                (ImportDesc::Mem(want), ExternVal::Mem(addr)) => {
                    let have = &self.mems[addr];
                    let have_limits = Limits {
                        min: (have.data.len() / PAGE_SIZE) as u32,
                        max: have.ty.limits.max,
                    };
                    if !limits_match(&have_limits, &want.limits) {
                        return Err(InstError::Link("incompatible import type".into()));
                    }
                    inst.mem_addrs.push(addr);
                }
                (ImportDesc::Global(want), ExternVal::Global(addr)) => {
                    if self.globals[addr].ty != want {
                        return Err(InstError::Link("incompatible import type".into()));
                    }
                    inst.global_addrs.push(addr);
                }
                _ => return Err(InstError::Link("incompatible import type".into())),
            }
        }

        // Allocation failures below must not leave orphan store entries
        // behind: before the instance is published nothing can legally
        // reference them, so they are rolled back wholesale. (Failures
        // *after* publication — segment application, start — deliberately
        // keep their effects, as the spec requires.)
        let funcs_base = self.funcs.len();
        let globals_base = self.globals.len();
        let tables_base = self.tables.len();
        let mems_base = self.mems.len();
        let rollback = |store: &mut Store, e: InstError| {
            store.funcs.truncate(funcs_base);
            store.globals.truncate(globals_base);
            store.tables.truncate(tables_base);
            store.mems.truncate(mems_base);
            Err(e)
        };

        // 2. Allocate own functions (addresses must exist before globals /
        // element segments evaluate ref.func).
        for (i, &tidx) in module.funcs.iter().enumerate() {
            let addr = self.funcs.len();
            self.funcs.push(FuncInst::Wasm {
                ty: module.types[tidx as usize].clone(),
                inst: self.instances.len(), // this instance's future id
                code: Rc::new(module.codes[i].clone()),
            });
            inst.func_addrs.push(addr);
        }

        // 3. Globals: evaluate initializers (context: imported globals +
        // this module's functions), then allocate.
        for g in &module.globals {
            let val = self.eval_const(&inst, &g.init);
            let addr = self.globals.len();
            self.globals.push(GlobalInst { ty: g.ty, val });
            inst.global_addrs.push(addr);
        }

        // 4. Tables and memories.
        for t in &module.tables {
            let addr = self.tables.len();
            if u64::from(t.limits.min) > TABLE_IMPL_LIMIT {
                return rollback(
                    self,
                    InstError::Link("table size exceeds implementation limit".into()),
                );
            }
            let mut elems = Vec::new();
            if elems.try_reserve_exact(t.limits.min as usize).is_err() {
                return rollback(self, InstError::Link("cannot allocate table".into()));
            }
            elems.resize(t.limits.min as usize, Value::null_of(t.elem));
            self.tables.push(TableInst { ty: *t, elems });
            inst.table_addrs.push(addr);
        }
        for mt in &module.mems {
            let addr = self.mems.len();
            let Some(bytes) = mem_bytes(mt.limits.min) else {
                return rollback(
                    self,
                    InstError::Link("memory size exceeds addressable range".into()),
                );
            };
            let mut data = Vec::new();
            if data.try_reserve_exact(bytes).is_err() {
                return rollback(self, InstError::Link("cannot allocate memory".into()));
            }
            data.resize(bytes, 0);
            self.mems.push(MemInst { ty: *mt, data });
            inst.mem_addrs.push(addr);
        }

        // 5. Element/data segment instances.
        for e in &module.elems {
            let elems = e
                .init
                .iter()
                .map(|expr| self.eval_const(&inst, expr))
                .collect();
            inst.elems.push(ElemInst { elems });
        }
        for d in &module.datas {
            inst.datas.push(DataInst {
                bytes: d.bytes.clone(),
            });
        }

        // 6. Exports.
        for ex in &module.exports {
            let ev = match ex.desc {
                ExportDesc::Func(i) => ExternVal::Func(inst.func_addrs[i as usize]),
                ExportDesc::Table(i) => ExternVal::Table(inst.table_addrs[i as usize]),
                ExportDesc::Mem(i) => ExternVal::Mem(inst.mem_addrs[i as usize]),
                ExportDesc::Global(i) => ExternVal::Global(inst.global_addrs[i as usize]),
            };
            inst.exports.push((ex.name.clone(), ev));
        }

        let id = self.instances.len();
        self.instances.push(inst);

        // 7. Apply active element segments in order (partial effects on
        // earlier segments persist on trap).
        for (i, e) in module.elems.iter().enumerate() {
            match &e.mode {
                ElemMode::Active { table, offset } => {
                    let inst_ref = &self.instances[id];
                    let taddr = inst_ref.table_addrs[*table as usize];
                    let off = match self.eval_const(&self.instances[id], offset) {
                        Value::I32(v) => v,
                        _ => unreachable!("validated offset type"),
                    };
                    let seg: Vec<Value> = self.instances[id].elems[i].elems.clone();
                    let table = &mut self.tables[taddr];
                    let end = u64::from(off) + seg.len() as u64;
                    if end > table.elems.len() as u64 {
                        return Err(InstError::Trap(Trap::new("out of bounds table access")));
                    }
                    table.elems[off as usize..off as usize + seg.len()].copy_from_slice(&seg);
                    self.instances[id].elems[i].elems.clear();
                }
                ElemMode::Declarative => {
                    self.instances[id].elems[i].elems.clear();
                }
                ElemMode::Passive => {}
            }
        }

        // 8. Apply active data segments in order.
        for (i, d) in module.datas.iter().enumerate() {
            if let DataMode::Active { mem, offset } = &d.mode {
                let inst_ref = &self.instances[id];
                let maddr = inst_ref.mem_addrs[*mem as usize];
                let off = match self.eval_const(&self.instances[id], offset) {
                    Value::I32(v) => v,
                    _ => unreachable!("validated offset type"),
                };
                let mem = &mut self.mems[maddr];
                let end = u64::from(off) + d.bytes.len() as u64;
                if end > mem.data.len() as u64 {
                    return Err(InstError::Trap(Trap::new("out of bounds memory access")));
                }
                mem.data[off as usize..off as usize + d.bytes.len()].copy_from_slice(&d.bytes);
                self.instances[id].datas[i].bytes.clear();
            }
        }

        // 9. Start function.
        if let Some(s) = module.start {
            let addr = self.instances[id].func_addrs[s as usize];
            crate::exec::invoke(self, addr, &[]).map_err(|e| match e {
                InvokeError::Trap(t) => InstError::Trap(t),
                other => InstError::Link(other.to_string()),
            })?;
        }

        Ok(id)
    }

    /// Evaluate a validated constant expression.
    pub(crate) fn eval_const(&self, inst: &Instance, e: &ConstExpr) -> Value {
        let mut stack: Vec<Value> = Vec::new();
        for i in &e.instrs {
            match i {
                Instr::I32Const(v) => stack.push(Value::I32(*v)),
                Instr::I64Const(v) => stack.push(Value::I64(*v)),
                Instr::F32Const(v) => stack.push(Value::F32(*v)),
                Instr::F64Const(v) => stack.push(Value::F64(*v)),
                Instr::V128Const(v) => stack.push(Value::V128(*v)),
                Instr::RefNull(t) => stack.push(Value::null_of(*t)),
                Instr::RefFunc { func } => {
                    stack.push(Value::FuncRef(Some(inst.func_addrs[*func as usize] as u32)))
                }
                Instr::GlobalGet { idx } => {
                    stack.push(self.globals[inst.global_addrs[*idx as usize]].val)
                }
                Instr::End => {}
                _ => unreachable!("validated const expr"),
            }
        }
        stack.pop().expect("const expr yields one value")
    }

    pub fn exports(&self, inst: InstanceId) -> impl Iterator<Item = (&String, &ExternVal)> {
        self.instances[inst].exports.iter().map(|(n, v)| (n, v))
    }

    pub fn export_val(&self, inst: InstanceId, field: &str) -> Option<ExternVal> {
        self.instances[inst]
            .exports
            .iter()
            .find(|(n, _)| n == field)
            .map(|(_, v)| *v)
    }

    pub fn invoke_export(
        &mut self,
        inst: InstanceId,
        field: &str,
        args: &[Value],
    ) -> Result<Vec<Value>, InvokeError> {
        let ev = self
            .export_val(inst, field)
            .ok_or_else(|| InvokeError::NoSuchExport(field.into()))?;
        let ExternVal::Func(addr) = ev else {
            return Err(InvokeError::KindMismatch(format!(
                "export {field} is not a function"
            )));
        };
        let ty = self.funcs[addr].ty();
        if args.len() != ty.params.len() {
            return Err(InvokeError::ArgMismatch(format!(
                "expected {} args, got {}",
                ty.params.len(),
                args.len()
            )));
        }
        for (a, p) in args.iter().zip(&ty.params) {
            if a.ty() != *p {
                return Err(InvokeError::ArgMismatch(format!(
                    "arg type {:?} != param type {:?}",
                    a.ty(),
                    p
                )));
            }
        }
        crate::exec::invoke(self, addr, args)
    }

    pub fn get_global_export(&self, inst: InstanceId, field: &str) -> Result<Value, InvokeError> {
        let ev = self
            .export_val(inst, field)
            .ok_or_else(|| InvokeError::NoSuchExport(field.into()))?;
        let ExternVal::Global(addr) = ev else {
            return Err(InvokeError::KindMismatch(format!(
                "export {field} is not a global"
            )));
        };
        Ok(self.globals[addr].val)
    }
}

/// Import limits matching: candidate {n1,m1} matches required {n2,m2}
/// iff n1 >= n2 and (m2 = none or m1 <= m2).
fn limits_match(have: &Limits, want: &Limits) -> bool {
    if have.min < want.min {
        return false;
    }
    match want.max {
        None => true,
        Some(wm) => match have.max {
            Some(hm) => hm <= wm,
            None => false,
        },
    }
}

impl Value {
    pub fn null_of(t: RefType) -> Value {
        match t {
            RefType::FuncRef => Value::FuncRef(None),
            RefType::ExternRef => Value::ExternRef(None),
        }
    }
}
