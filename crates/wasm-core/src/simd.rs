//! SIMD (v128) execution semantics. M3 stubs trap fail-closed; M4 fills
//! in the full operator set.

use crate::error::Trap;
use crate::exec::Machine;
use crate::module::{SimdLaneOp, SimdMemOp, SimdOp};

pub(crate) fn simd_op(_m: &mut Machine, op: SimdOp) -> Result<(), Trap> {
    Err(Trap::new(format!(
        "SIMD op {op:?} not implemented (fail-closed)"
    )))
}

pub(crate) fn simd_lane(_m: &mut Machine, op: SimdLaneOp, _lane: u8) -> Result<(), Trap> {
    Err(Trap::new(format!(
        "SIMD lane op {op:?} not implemented (fail-closed)"
    )))
}

pub(crate) fn shuffle(_m: &mut Machine, _lanes: &[u8; 16]) -> Result<(), Trap> {
    Err(Trap::new(
        "SIMD shuffle not implemented (fail-closed)".to_string(),
    ))
}

pub(crate) fn simd_mem(
    _m: &mut Machine,
    op: SimdMemOp,
    _offset: u32,
    _lane: u8,
) -> Result<(), Trap> {
    Err(Trap::new(format!(
        "SIMD mem op {op:?} not implemented (fail-closed)"
    )))
}
