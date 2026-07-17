//! Store: runtime instances of modules and host modules. Execution (M3)
//! plugs into this; in the M1 baseline, wasm instantiation is unreachable
//! because the decoder rejects everything.

use crate::error::{InstError, InvokeError, Trap};
use crate::module::Module;
use crate::types::{FuncType, GlobalType, MemType, TableType};
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
    #[allow(dead_code)]
    Wasm {
        ty: FuncType,
    },
}

#[allow(dead_code)] // read by the M3 interpreter
pub(crate) struct TableInst {
    pub ty: TableType,
    pub elems: Vec<Value>,
}

#[allow(dead_code)] // read by the M3 interpreter
pub(crate) struct MemInst {
    pub ty: MemType,
    pub data: Vec<u8>,
}

#[allow(dead_code)] // read by the M3 interpreter
pub(crate) struct GlobalInst {
    pub ty: GlobalType,
    pub val: Value,
}

#[derive(Default)]
pub(crate) struct Instance {
    pub exports: Vec<(String, ExternVal)>,
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

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    pub fn add_host_module(&mut self, desc: HostModule) -> InstanceId {
        let mut inst = Instance::default();
        for (name, ty, func) in desc.funcs {
            let addr = self.funcs.len();
            self.funcs.push(FuncInst::Host { ty, func });
            inst.exports.push((name, ExternVal::Func(addr)));
        }
        for (name, ty, val) in desc.globals {
            let addr = self.globals.len();
            self.globals.push(GlobalInst { ty, val });
            inst.exports.push((name, ExternVal::Global(addr)));
        }
        for (name, ty) in desc.tables {
            let addr = self.tables.len();
            self.tables.push(TableInst {
                ty,
                elems: vec![Value::FuncRef(None); ty.limits.min as usize],
            });
            inst.exports.push((name, ExternVal::Table(addr)));
        }
        for (name, ty) in desc.mems {
            let addr = self.mems.len();
            self.mems.push(MemInst {
                ty,
                data: vec![0; ty.limits.min as usize * PAGE_SIZE],
            });
            inst.exports.push((name, ExternVal::Mem(addr)));
        }
        self.instances.push(inst);
        self.instances.len() - 1
    }

    pub fn instantiate(
        &mut self,
        _module: &Module,
        _resolve: &mut dyn FnMut(&str, &str) -> Option<ExternVal>,
    ) -> Result<InstanceId, InstError> {
        Err(InstError::Link(
            "instantiation not implemented (fail-closed baseline)".into(),
        ))
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
        self.invoke_func(addr, args)
    }

    pub(crate) fn invoke_func(
        &mut self,
        addr: usize,
        args: &[Value],
    ) -> Result<Vec<Value>, InvokeError> {
        match &self.funcs[addr] {
            FuncInst::Host { ty, func } => {
                check_args(ty, args)?;
                let f = *func;
                f(args).map_err(InvokeError::Trap)
            }
            FuncInst::Wasm { .. } => Err(InvokeError::Trap(Trap::new(
                "wasm execution not implemented (fail-closed baseline)",
            ))),
        }
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

fn check_args(ty: &FuncType, args: &[Value]) -> Result<(), InvokeError> {
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
    Ok(())
}
