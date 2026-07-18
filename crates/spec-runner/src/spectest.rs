//! The spectest host module, as defined by the reference interpreter's
//! host environment (spec/interpreter/host/spectest.ml at wg-2.0).

use wasm_core::{
    FuncType, GlobalType, HostModule, Limits, MemType, RefType, TableType, ValType, Value,
};

fn ft(params: &[ValType], results: &[ValType]) -> FuncType {
    FuncType {
        params: params.to_vec(),
        results: results.to_vec(),
    }
}

pub fn spectest_module() -> HostModule {
    use ValType::*;
    HostModule {
        funcs: vec![
            ("print".into(), ft(&[], &[]), |_| Ok(vec![])),
            ("print_i32".into(), ft(&[I32], &[]), |_| Ok(vec![])),
            ("print_i64".into(), ft(&[I64], &[]), |_| Ok(vec![])),
            ("print_f32".into(), ft(&[F32], &[]), |_| Ok(vec![])),
            ("print_f64".into(), ft(&[F64], &[]), |_| Ok(vec![])),
            ("print_i32_f32".into(), ft(&[I32, F32], &[]), |_| Ok(vec![])),
            ("print_f64_f64".into(), ft(&[F64, F64], &[]), |_| Ok(vec![])),
        ],
        globals: vec![
            (
                "global_i32".into(),
                GlobalType {
                    val: I32,
                    mutable: false,
                },
                Value::I32(666),
            ),
            (
                "global_i64".into(),
                GlobalType {
                    val: I64,
                    mutable: false,
                },
                Value::I64(666),
            ),
            (
                "global_f32".into(),
                GlobalType {
                    val: F32,
                    mutable: false,
                },
                Value::F32(666.6f32.to_bits()),
            ),
            (
                "global_f64".into(),
                GlobalType {
                    val: F64,
                    mutable: false,
                },
                Value::F64(666.6f64.to_bits()),
            ),
        ],
        tables: vec![(
            "table".into(),
            TableType {
                elem: RefType::FuncRef,
                limits: Limits {
                    min: 10,
                    max: Some(20),
                },
            },
        )],
        mems: vec![(
            "memory".into(),
            MemType {
                limits: Limits {
                    min: 1,
                    max: Some(2),
                },
            },
        )],
    }
}
