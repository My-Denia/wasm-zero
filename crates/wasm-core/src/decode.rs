//! Binary format decoder (M2). Currently a fail-closed stub: every module
//! is rejected, which drives the M1 all-FAIL baseline.

use crate::error::DecodeError;
use crate::module::Module;

pub fn decode(_bytes: &[u8]) -> Result<Module, DecodeError> {
    Err(DecodeError {
        offset: 0,
        msg: "decoder not implemented (fail-closed baseline)".into(),
    })
}
