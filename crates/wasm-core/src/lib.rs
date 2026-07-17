//! wasm-core: a from-scratch WebAssembly 2.0 (wg-2.0) engine:
//! binary decoder, validator, and interpreter.
//!
//! Implementation surface is the binary format; the text format is out of
//! scope (handled upstream by the pinned wast2json toolchain).

pub mod decode;
pub mod error;
pub mod module;
pub mod store;
pub mod types;
pub mod validate;
pub mod values;

pub use decode::decode;
pub use error::{DecodeError, InstError, InvokeError, Trap, ValidateError};
pub use store::{ExternVal, HostFunc, HostModule, InstanceId, Store};
pub use types::{FuncType, GlobalType, Limits, MemType, RefType, TableType, ValType};
pub use validate::validate;
pub use values::Value;
