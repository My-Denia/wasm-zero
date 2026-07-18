//! JSON value <-> engine value conversion and expected-result matching.
//! All unknown shapes return Err, which the caller books as FAIL.

use serde_json::Value as J;
use wasm_core::Value;

/// Parse an argument/constant value object like {"type":"i32","value":"7"}.
pub fn parse_value(v: &J) -> Result<Value, String> {
    let ty = str_field(v, "type")?;
    match ty {
        "i32" => Ok(Value::I32(parse_u32(str_field(v, "value")?)?)),
        "i64" => Ok(Value::I64(parse_u64(str_field(v, "value")?)?)),
        "f32" => Ok(Value::F32(parse_u32(str_field(v, "value")?)?)),
        "f64" => Ok(Value::F64(parse_u64(str_field(v, "value")?)?)),
        "v128" => Ok(Value::V128(parse_v128(v)?)),
        "externref" => {
            let s = str_field(v, "value")?;
            if s == "null" {
                Ok(Value::ExternRef(None))
            } else {
                Ok(Value::ExternRef(Some(parse_u32(s)?)))
            }
        }
        "funcref" => {
            let s = str_field(v, "value")?;
            if s == "null" {
                Ok(Value::FuncRef(None))
            } else {
                Err(format!("non-null funcref argument unsupported: {s}"))
            }
        }
        other => Err(format!("unknown value type {other:?} (fail-closed)")),
    }
}

/// Match one actual result against one expected-value object
/// (which may contain nan:canonical / nan:arithmetic patterns).
pub fn match_expected(actual: &Value, exp: &J) -> Result<bool, String> {
    let ty = str_field(exp, "type")?;
    match (ty, actual) {
        ("i32", Value::I32(a)) => Ok(*a == parse_u32(str_field(exp, "value")?)?),
        ("i64", Value::I64(a)) => Ok(*a == parse_u64(str_field(exp, "value")?)?),
        ("f32", Value::F32(a)) => match_f32(*a, str_field(exp, "value")?),
        ("f64", Value::F64(a)) => match_f64(*a, str_field(exp, "value")?),
        ("v128", Value::V128(a)) => match_v128(*a, exp),
        ("externref", Value::ExternRef(a)) => {
            let s = str_field(exp, "value")?;
            if s == "null" {
                Ok(a.is_none())
            } else {
                Ok(*a == Some(parse_u32(s)?))
            }
        }
        ("funcref", Value::FuncRef(a)) => {
            let s = str_field(exp, "value")?;
            if s == "null" {
                Ok(a.is_none())
            } else {
                Err(format!("non-null funcref expectation unsupported: {s}"))
            }
        }
        _ => Ok(false), // type mismatch between actual and expected
    }
}

fn match_f32(bits: u32, exp: &str) -> Result<bool, String> {
    match exp {
        "nan:canonical" => Ok(bits & 0x7fff_ffff == 0x7fc0_0000),
        "nan:arithmetic" => Ok(bits & 0x7fc0_0000 == 0x7fc0_0000),
        s => Ok(bits == parse_u32(s)?),
    }
}

fn match_f64(bits: u64, exp: &str) -> Result<bool, String> {
    match exp {
        "nan:canonical" => Ok(bits & 0x7fff_ffff_ffff_ffff == 0x7ff8_0000_0000_0000),
        "nan:arithmetic" => Ok(bits & 0x7ff8_0000_0000_0000 == 0x7ff8_0000_0000_0000),
        s => Ok(bits == parse_u64(s)?),
    }
}

fn parse_v128(v: &J) -> Result<u128, String> {
    let lane_type = str_field(v, "lane_type")?;
    let lanes = v
        .get("value")
        .and_then(|x| x.as_array())
        .ok_or("v128 value must be an array")?;
    let mut bytes = [0u8; 16];
    match lane_type {
        "i8" => pack(lanes, 16, 1, &mut bytes)?,
        "i16" => pack(lanes, 8, 2, &mut bytes)?,
        "i32" | "f32" => pack(lanes, 4, 4, &mut bytes)?,
        "i64" | "f64" => pack(lanes, 2, 8, &mut bytes)?,
        other => return Err(format!("unknown v128 lane_type {other:?}")),
    }
    Ok(u128::from_le_bytes(bytes))
}

fn pack(lanes: &[J], count: usize, width: usize, out: &mut [u8; 16]) -> Result<(), String> {
    if lanes.len() != count {
        return Err(format!("v128 expects {count} lanes, got {}", lanes.len()));
    }
    for (i, l) in lanes.iter().enumerate() {
        let s = l.as_str().ok_or("v128 lane must be a string")?;
        let n = parse_u64(s)?;
        out[i * width..(i + 1) * width].copy_from_slice(&n.to_le_bytes()[..width]);
    }
    Ok(())
}

fn match_v128(actual: u128, exp: &J) -> Result<bool, String> {
    let lane_type = str_field(exp, "lane_type")?;
    let lanes = exp
        .get("value")
        .and_then(|x| x.as_array())
        .ok_or("v128 value must be an array")?;
    let bytes = actual.to_le_bytes();
    let (count, width): (usize, usize) = match lane_type {
        "i8" => (16, 1),
        "i16" => (8, 2),
        "i32" | "f32" => (4, 4),
        "i64" | "f64" => (2, 8),
        other => return Err(format!("unknown v128 lane_type {other:?}")),
    };
    if lanes.len() != count {
        return Err(format!("v128 expects {count} lanes, got {}", lanes.len()));
    }
    for (i, l) in lanes.iter().enumerate() {
        let s = l.as_str().ok_or("v128 lane must be a string")?;
        let mut buf = [0u8; 8];
        buf[..width].copy_from_slice(&bytes[i * width..(i + 1) * width]);
        let lane_bits = u64::from_le_bytes(buf);
        let ok = match (lane_type, s) {
            ("f32", "nan:canonical") => lane_bits as u32 & 0x7fff_ffff == 0x7fc0_0000,
            ("f32", "nan:arithmetic") => lane_bits as u32 & 0x7fc0_0000 == 0x7fc0_0000,
            ("f64", "nan:canonical") => lane_bits & 0x7fff_ffff_ffff_ffff == 0x7ff8_0000_0000_0000,
            ("f64", "nan:arithmetic") => lane_bits & 0x7ff8_0000_0000_0000 == 0x7ff8_0000_0000_0000,
            (_, s) => lane_bits == parse_u64(s)?,
        };
        if !ok {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn str_field<'a>(v: &'a J, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("missing/non-string field {key:?}"))
}

fn parse_u32(s: &str) -> Result<u32, String> {
    s.parse::<u32>().map_err(|e| format!("bad u32 {s:?}: {e}"))
}

fn parse_u64(s: &str) -> Result<u64, String> {
    s.parse::<u64>().map_err(|e| format!("bad u64 {s:?}: {e}"))
}
