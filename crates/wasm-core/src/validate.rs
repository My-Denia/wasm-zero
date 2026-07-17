//! Validator (M2). Fail-closed stub.

use crate::error::ValidateError;
use crate::module::Module;

pub fn validate(_m: &Module) -> Result<(), ValidateError> {
    Err(ValidateError {
        msg: "validator not implemented (fail-closed baseline)".into(),
    })
}
