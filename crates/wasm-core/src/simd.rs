//! SIMD (v128) execution semantics for the full wg-2.0 operator set.
//! v128 values travel as u128 in little-endian byte order; helpers view
//! them as lane arrays.

use crate::error::Trap;
use crate::exec::{
    ceil32, ceil64, floor32, floor64, fmax32, fmax64, fmin32, fmin64, nearest32, nearest64,
    trunc32, trunc64, Machine,
};
use crate::module::{SimdLaneOp, SimdMemOp, SimdOp};
use crate::values::Value;

type Exec = Result<(), Trap>;

// ---- lane views ----

fn b16(v: u128) -> [u8; 16] {
    v.to_le_bytes()
}

fn from_b16(a: [u8; 16]) -> u128 {
    u128::from_le_bytes(a)
}

fn u16x8(v: u128) -> [u16; 8] {
    let b = b16(v);
    std::array::from_fn(|i| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]))
}

fn from_u16x8(a: [u16; 8]) -> u128 {
    let mut b = [0u8; 16];
    for (i, x) in a.iter().enumerate() {
        b[2 * i..2 * i + 2].copy_from_slice(&x.to_le_bytes());
    }
    from_b16(b)
}

fn u32x4(v: u128) -> [u32; 4] {
    let b = b16(v);
    std::array::from_fn(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap()))
}

fn from_u32x4(a: [u32; 4]) -> u128 {
    let mut b = [0u8; 16];
    for (i, x) in a.iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
    }
    from_b16(b)
}

fn u64x2(v: u128) -> [u64; 2] {
    let b = b16(v);
    std::array::from_fn(|i| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().unwrap()))
}

fn from_u64x2(a: [u64; 2]) -> u128 {
    let mut b = [0u8; 16];
    for (i, x) in a.iter().enumerate() {
        b[8 * i..8 * i + 8].copy_from_slice(&x.to_le_bytes());
    }
    from_b16(b)
}

fn f32x4(v: u128) -> [f32; 4] {
    u32x4(v).map(f32::from_bits)
}

fn from_f32x4(a: [f32; 4]) -> u128 {
    from_u32x4(a.map(f32::to_bits))
}

fn f64x2(v: u128) -> [f64; 2] {
    u64x2(v).map(f64::from_bits)
}

fn from_f64x2(a: [f64; 2]) -> u128 {
    from_u64x2(a.map(f64::to_bits))
}

// ---- generic lane combinators ----

fn map8(v: u128, f: impl Fn(u8) -> u8) -> u128 {
    from_b16(b16(v).map(f))
}

fn zip8(a: u128, b: u128, f: impl Fn(u8, u8) -> u8) -> u128 {
    let (x, y) = (b16(a), b16(b));
    from_b16(std::array::from_fn(|i| f(x[i], y[i])))
}

fn map16(v: u128, f: impl Fn(u16) -> u16) -> u128 {
    from_u16x8(u16x8(v).map(f))
}

fn zip16(a: u128, b: u128, f: impl Fn(u16, u16) -> u16) -> u128 {
    let (x, y) = (u16x8(a), u16x8(b));
    from_u16x8(std::array::from_fn(|i| f(x[i], y[i])))
}

fn map32(v: u128, f: impl Fn(u32) -> u32) -> u128 {
    from_u32x4(u32x4(v).map(f))
}

fn zip32(a: u128, b: u128, f: impl Fn(u32, u32) -> u32) -> u128 {
    let (x, y) = (u32x4(a), u32x4(b));
    from_u32x4(std::array::from_fn(|i| f(x[i], y[i])))
}

fn map64(v: u128, f: impl Fn(u64) -> u64) -> u128 {
    from_u64x2(u64x2(v).map(f))
}

fn zip64(a: u128, b: u128, f: impl Fn(u64, u64) -> u64) -> u128 {
    let (x, y) = (u64x2(a), u64x2(b));
    from_u64x2(std::array::from_fn(|i| f(x[i], y[i])))
}

fn mapf32(v: u128, f: impl Fn(f32) -> f32) -> u128 {
    from_f32x4(f32x4(v).map(f))
}

fn zipf32(a: u128, b: u128, f: impl Fn(f32, f32) -> f32) -> u128 {
    let (x, y) = (f32x4(a), f32x4(b));
    from_f32x4(std::array::from_fn(|i| f(x[i], y[i])))
}

fn mapf64(v: u128, f: impl Fn(f64) -> f64) -> u128 {
    from_f64x2(f64x2(v).map(f))
}

fn zipf64(a: u128, b: u128, f: impl Fn(f64, f64) -> f64) -> u128 {
    let (x, y) = (f64x2(a), f64x2(b));
    from_f64x2(std::array::from_fn(|i| f(x[i], y[i])))
}

fn bool8(b: bool) -> u8 {
    if b {
        0xff
    } else {
        0
    }
}

fn bool16(b: bool) -> u16 {
    if b {
        0xffff
    } else {
        0
    }
}

fn bool32(b: bool) -> u32 {
    if b {
        u32::MAX
    } else {
        0
    }
}

fn bool64(b: bool) -> u64 {
    if b {
        u64::MAX
    } else {
        0
    }
}

fn sat_i32_from_f32(x: f32) -> u32 {
    (x as i32) as u32 // Rust float->int casts saturate; NaN -> 0
}

fn sat_u32_from_f32(x: f32) -> u32 {
    x as u32
}

fn sat_i32_from_f64(x: f64) -> u32 {
    (x as i32) as u32
}

fn sat_u32_from_f64(x: f64) -> u32 {
    x as u32
}

pub(crate) fn simd_op(m: &mut Machine, op: SimdOp) -> Exec {
    use SimdOp::*;
    // Unary scalar->v128 splats.
    match op {
        I8x16Splat => {
            let x = m.pop_value();
            let Value::I32(v) = x else { unreachable!() };
            m.push_v128(from_b16([v as u8; 16]));
            return Ok(());
        }
        I16x8Splat => {
            let Value::I32(v) = m.pop_value() else {
                unreachable!()
            };
            m.push_v128(from_u16x8([v as u16; 8]));
            return Ok(());
        }
        I32x4Splat => {
            let Value::I32(v) = m.pop_value() else {
                unreachable!()
            };
            m.push_v128(from_u32x4([v; 4]));
            return Ok(());
        }
        I64x2Splat => {
            let Value::I64(v) = m.pop_value() else {
                unreachable!()
            };
            m.push_v128(from_u64x2([v; 2]));
            return Ok(());
        }
        F32x4Splat => {
            let Value::F32(v) = m.pop_value() else {
                unreachable!()
            };
            m.push_v128(from_u32x4([v; 4]));
            return Ok(());
        }
        F64x2Splat => {
            let Value::F64(v) = m.pop_value() else {
                unreachable!()
            };
            m.push_v128(from_u64x2([v; 2]));
            return Ok(());
        }
        _ => {}
    }

    // v128 -> i32 reductions.
    match op {
        V128AnyTrue => {
            let a = m.pop_v128();
            m.push_value(Value::I32((a != 0) as u32));
            return Ok(());
        }
        I8x16AllTrue => {
            let a = m.pop_v128();
            m.push_value(Value::I32(b16(a).iter().all(|&x| x != 0) as u32));
            return Ok(());
        }
        I16x8AllTrue => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u16x8(a).iter().all(|&x| x != 0) as u32));
            return Ok(());
        }
        I32x4AllTrue => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u32x4(a).iter().all(|&x| x != 0) as u32));
            return Ok(());
        }
        I64x2AllTrue => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u64x2(a).iter().all(|&x| x != 0) as u32));
            return Ok(());
        }
        I8x16Bitmask => {
            let a = m.pop_v128();
            let mut r = 0u32;
            for (i, x) in b16(a).iter().enumerate() {
                r |= u32::from(x >> 7) << i;
            }
            m.push_value(Value::I32(r));
            return Ok(());
        }
        I16x8Bitmask => {
            let a = m.pop_v128();
            let mut r = 0u32;
            for (i, x) in u16x8(a).iter().enumerate() {
                r |= u32::from(x >> 15) << i;
            }
            m.push_value(Value::I32(r));
            return Ok(());
        }
        I32x4Bitmask => {
            let a = m.pop_v128();
            let mut r = 0u32;
            for (i, x) in u32x4(a).iter().enumerate() {
                r |= (x >> 31) << i;
            }
            m.push_value(Value::I32(r));
            return Ok(());
        }
        I64x2Bitmask => {
            let a = m.pop_v128();
            let mut r = 0u32;
            for (i, x) in u64x2(a).iter().enumerate() {
                r |= ((x >> 63) as u32) << i;
            }
            m.push_value(Value::I32(r));
            return Ok(());
        }
        _ => {}
    }

    // Shifts: v128 x i32 -> v128.
    match op {
        I8x16Shl | I8x16ShrS | I8x16ShrU | I16x8Shl | I16x8ShrS | I16x8ShrU | I32x4Shl
        | I32x4ShrS | I32x4ShrU | I64x2Shl | I64x2ShrS | I64x2ShrU => {
            let Value::I32(s) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let r = match op {
                I8x16Shl => map8(a, |x| x.wrapping_shl(s & 7)),
                I8x16ShrS => map8(a, |x| ((x as i8) >> (s & 7)) as u8),
                I8x16ShrU => map8(a, |x| x >> (s & 7)),
                I16x8Shl => map16(a, |x| x.wrapping_shl(s & 15)),
                I16x8ShrS => map16(a, |x| ((x as i16) >> (s & 15)) as u16),
                I16x8ShrU => map16(a, |x| x >> (s & 15)),
                I32x4Shl => map32(a, |x| x.wrapping_shl(s & 31)),
                I32x4ShrS => map32(a, |x| ((x as i32) >> (s & 31)) as u32),
                I32x4ShrU => map32(a, |x| x >> (s & 31)),
                I64x2Shl => map64(a, |x| x.wrapping_shl(s & 63)),
                I64x2ShrS => map64(a, |x| ((x as i64) >> (s & 63)) as u64),
                _ => map64(a, |x| x >> (s & 63)),
            };
            m.push_v128(r);
            return Ok(());
        }
        V128Bitselect => {
            let c = m.pop_v128();
            let b = m.pop_v128();
            let a = m.pop_v128();
            m.push_v128((a & c) | (b & !c));
            return Ok(());
        }
        _ => {}
    }

    // Unary v128 -> v128.
    let unary: Option<fn(u128) -> u128> = match op {
        V128Not => Some(|a| !a),
        I8x16Abs => Some(|a| map8(a, |x| (x as i8).wrapping_abs() as u8)),
        I8x16Neg => Some(|a| map8(a, |x| x.wrapping_neg())),
        I8x16Popcnt => Some(|a| map8(a, |x| x.count_ones() as u8)),
        I16x8Abs => Some(|a| map16(a, |x| (x as i16).wrapping_abs() as u16)),
        I16x8Neg => Some(|a| map16(a, |x| x.wrapping_neg())),
        I32x4Abs => Some(|a| map32(a, |x| (x as i32).wrapping_abs() as u32)),
        I32x4Neg => Some(|a| map32(a, |x| x.wrapping_neg())),
        I64x2Abs => Some(|a| map64(a, |x| (x as i64).wrapping_abs() as u64)),
        I64x2Neg => Some(|a| map64(a, |x| x.wrapping_neg())),
        F32x4Abs => Some(|a| map32(a, |x| x & 0x7fff_ffff)),
        F32x4Neg => Some(|a| map32(a, |x| x ^ 0x8000_0000)),
        F32x4Sqrt => Some(|a| mapf32(a, f32::sqrt)),
        F32x4Ceil => Some(|a| mapf32(a, ceil32)),
        F32x4Floor => Some(|a| mapf32(a, floor32)),
        F32x4Trunc => Some(|a| mapf32(a, trunc32)),
        F32x4Nearest => Some(|a| mapf32(a, nearest32)),
        F64x2Abs => Some(|a| map64(a, |x| x & 0x7fff_ffff_ffff_ffff)),
        F64x2Neg => Some(|a| map64(a, |x| x ^ 0x8000_0000_0000_0000)),
        F64x2Sqrt => Some(|a| mapf64(a, f64::sqrt)),
        F64x2Ceil => Some(|a| mapf64(a, ceil64)),
        F64x2Floor => Some(|a| mapf64(a, floor64)),
        F64x2Trunc => Some(|a| mapf64(a, trunc64)),
        F64x2Nearest => Some(|a| mapf64(a, nearest64)),
        I16x8ExtendLowI8x16S => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| x[i] as i8 as i16 as u16))
        }),
        I16x8ExtendHighI8x16S => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| x[i + 8] as i8 as i16 as u16))
        }),
        I16x8ExtendLowI8x16U => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| u16::from(x[i])))
        }),
        I16x8ExtendHighI8x16U => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| u16::from(x[i + 8])))
        }),
        I32x4ExtendLowI16x8S => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| x[i] as i16 as i32 as u32))
        }),
        I32x4ExtendHighI16x8S => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| x[i + 4] as i16 as i32 as u32))
        }),
        I32x4ExtendLowI16x8U => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| u32::from(x[i])))
        }),
        I32x4ExtendHighI16x8U => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| u32::from(x[i + 4])))
        }),
        I64x2ExtendLowI32x4S => Some(|a| {
            let x = u32x4(a);
            from_u64x2(std::array::from_fn(|i| x[i] as i32 as i64 as u64))
        }),
        I64x2ExtendHighI32x4S => Some(|a| {
            let x = u32x4(a);
            from_u64x2(std::array::from_fn(|i| x[i + 2] as i32 as i64 as u64))
        }),
        I64x2ExtendLowI32x4U => Some(|a| {
            let x = u32x4(a);
            from_u64x2(std::array::from_fn(|i| u64::from(x[i])))
        }),
        I64x2ExtendHighI32x4U => Some(|a| {
            let x = u32x4(a);
            from_u64x2(std::array::from_fn(|i| u64::from(x[i + 2])))
        }),
        I16x8ExtaddPairwiseI8x16S => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| {
                (i16::from(x[2 * i] as i8) + i16::from(x[2 * i + 1] as i8)) as u16
            }))
        }),
        I16x8ExtaddPairwiseI8x16U => Some(|a| {
            let x = b16(a);
            from_u16x8(std::array::from_fn(|i| {
                u16::from(x[2 * i]) + u16::from(x[2 * i + 1])
            }))
        }),
        I32x4ExtaddPairwiseI16x8S => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| {
                (i32::from(x[2 * i] as i16) + i32::from(x[2 * i + 1] as i16)) as u32
            }))
        }),
        I32x4ExtaddPairwiseI16x8U => Some(|a| {
            let x = u16x8(a);
            from_u32x4(std::array::from_fn(|i| {
                u32::from(x[2 * i]) + u32::from(x[2 * i + 1])
            }))
        }),
        I32x4TruncSatF32x4S => Some(|a| {
            let x = f32x4(a);
            from_u32x4(x.map(sat_i32_from_f32))
        }),
        I32x4TruncSatF32x4U => Some(|a| {
            let x = f32x4(a);
            from_u32x4(x.map(sat_u32_from_f32))
        }),
        F32x4ConvertI32x4S => Some(|a| {
            let x = u32x4(a);
            from_f32x4(x.map(|v| v as i32 as f32))
        }),
        F32x4ConvertI32x4U => Some(|a| {
            let x = u32x4(a);
            from_f32x4(x.map(|v| v as f32))
        }),
        I32x4TruncSatF64x2SZero => Some(|a| {
            let x = f64x2(a);
            from_u32x4([sat_i32_from_f64(x[0]), sat_i32_from_f64(x[1]), 0, 0])
        }),
        I32x4TruncSatF64x2UZero => Some(|a| {
            let x = f64x2(a);
            from_u32x4([sat_u32_from_f64(x[0]), sat_u32_from_f64(x[1]), 0, 0])
        }),
        F64x2ConvertLowI32x4S => Some(|a| {
            let x = u32x4(a);
            from_f64x2([f64::from(x[0] as i32), f64::from(x[1] as i32)])
        }),
        F64x2ConvertLowI32x4U => Some(|a| {
            let x = u32x4(a);
            from_f64x2([f64::from(x[0]), f64::from(x[1])])
        }),
        F32x4DemoteF64x2Zero => Some(|a| {
            let x = f64x2(a);
            from_f32x4([x[0] as f32, x[1] as f32, 0.0, 0.0])
        }),
        F64x2PromoteLowF32x4 => Some(|a| {
            let x = f32x4(a);
            from_f64x2([f64::from(x[0]), f64::from(x[1])])
        }),
        _ => None,
    };
    if let Some(f) = unary {
        let a = m.pop_v128();
        m.push_v128(f(a));
        return Ok(());
    }

    // Binary v128 x v128 -> v128.
    let b = m.pop_v128();
    let a = m.pop_v128();
    let r = match op {
        I8x16Swizzle => {
            let (x, idx) = (b16(a), b16(b));
            from_b16(std::array::from_fn(|i| {
                let j = idx[i] as usize;
                if j < 16 {
                    x[j]
                } else {
                    0
                }
            }))
        }
        I8x16Eq => zip8(a, b, |x, y| bool8(x == y)),
        I8x16Ne => zip8(a, b, |x, y| bool8(x != y)),
        I8x16LtS => zip8(a, b, |x, y| bool8((x as i8) < (y as i8))),
        I8x16LtU => zip8(a, b, |x, y| bool8(x < y)),
        I8x16GtS => zip8(a, b, |x, y| bool8((x as i8) > (y as i8))),
        I8x16GtU => zip8(a, b, |x, y| bool8(x > y)),
        I8x16LeS => zip8(a, b, |x, y| bool8((x as i8) <= (y as i8))),
        I8x16LeU => zip8(a, b, |x, y| bool8(x <= y)),
        I8x16GeS => zip8(a, b, |x, y| bool8((x as i8) >= (y as i8))),
        I8x16GeU => zip8(a, b, |x, y| bool8(x >= y)),
        I16x8Eq => zip16(a, b, |x, y| bool16(x == y)),
        I16x8Ne => zip16(a, b, |x, y| bool16(x != y)),
        I16x8LtS => zip16(a, b, |x, y| bool16((x as i16) < (y as i16))),
        I16x8LtU => zip16(a, b, |x, y| bool16(x < y)),
        I16x8GtS => zip16(a, b, |x, y| bool16((x as i16) > (y as i16))),
        I16x8GtU => zip16(a, b, |x, y| bool16(x > y)),
        I16x8LeS => zip16(a, b, |x, y| bool16((x as i16) <= (y as i16))),
        I16x8LeU => zip16(a, b, |x, y| bool16(x <= y)),
        I16x8GeS => zip16(a, b, |x, y| bool16((x as i16) >= (y as i16))),
        I16x8GeU => zip16(a, b, |x, y| bool16(x >= y)),
        I32x4Eq => zip32(a, b, |x, y| bool32(x == y)),
        I32x4Ne => zip32(a, b, |x, y| bool32(x != y)),
        I32x4LtS => zip32(a, b, |x, y| bool32((x as i32) < (y as i32))),
        I32x4LtU => zip32(a, b, |x, y| bool32(x < y)),
        I32x4GtS => zip32(a, b, |x, y| bool32((x as i32) > (y as i32))),
        I32x4GtU => zip32(a, b, |x, y| bool32(x > y)),
        I32x4LeS => zip32(a, b, |x, y| bool32((x as i32) <= (y as i32))),
        I32x4LeU => zip32(a, b, |x, y| bool32(x <= y)),
        I32x4GeS => zip32(a, b, |x, y| bool32((x as i32) >= (y as i32))),
        I32x4GeU => zip32(a, b, |x, y| bool32(x >= y)),
        I64x2Eq => zip64(a, b, |x, y| bool64(x == y)),
        I64x2Ne => zip64(a, b, |x, y| bool64(x != y)),
        I64x2LtS => zip64(a, b, |x, y| bool64((x as i64) < (y as i64))),
        I64x2GtS => zip64(a, b, |x, y| bool64((x as i64) > (y as i64))),
        I64x2LeS => zip64(a, b, |x, y| bool64((x as i64) <= (y as i64))),
        I64x2GeS => zip64(a, b, |x, y| bool64((x as i64) >= (y as i64))),
        F32x4Eq => zip32u_from_f32(a, b, |x, y| bool32(x == y)),
        F32x4Ne => zip32u_from_f32(a, b, |x, y| bool32(x != y)),
        F32x4Lt => zip32u_from_f32(a, b, |x, y| bool32(x < y)),
        F32x4Gt => zip32u_from_f32(a, b, |x, y| bool32(x > y)),
        F32x4Le => zip32u_from_f32(a, b, |x, y| bool32(x <= y)),
        F32x4Ge => zip32u_from_f32(a, b, |x, y| bool32(x >= y)),
        F64x2Eq => zip64u_from_f64(a, b, |x, y| bool64(x == y)),
        F64x2Ne => zip64u_from_f64(a, b, |x, y| bool64(x != y)),
        F64x2Lt => zip64u_from_f64(a, b, |x, y| bool64(x < y)),
        F64x2Gt => zip64u_from_f64(a, b, |x, y| bool64(x > y)),
        F64x2Le => zip64u_from_f64(a, b, |x, y| bool64(x <= y)),
        F64x2Ge => zip64u_from_f64(a, b, |x, y| bool64(x >= y)),
        V128And => a & b,
        V128Andnot => a & !b,
        V128Or => a | b,
        V128Xor => a ^ b,
        I8x16NarrowI16x8S => {
            let (x, y) = (u16x8(a), u16x8(b));
            let sat = |v: u16| (v as i16).clamp(-128, 127) as i8 as u8;
            let mut r = [0u8; 16];
            for i in 0..8 {
                r[i] = sat(x[i]);
                r[i + 8] = sat(y[i]);
            }
            from_b16(r)
        }
        I8x16NarrowI16x8U => {
            let (x, y) = (u16x8(a), u16x8(b));
            let sat = |v: u16| (v as i16).clamp(0, 255) as u8;
            let mut r = [0u8; 16];
            for i in 0..8 {
                r[i] = sat(x[i]);
                r[i + 8] = sat(y[i]);
            }
            from_b16(r)
        }
        I16x8NarrowI32x4S => {
            let (x, y) = (u32x4(a), u32x4(b));
            let sat = |v: u32| (v as i32).clamp(-32768, 32767) as i16 as u16;
            let mut r = [0u16; 8];
            for i in 0..4 {
                r[i] = sat(x[i]);
                r[i + 4] = sat(y[i]);
            }
            from_u16x8(r)
        }
        I16x8NarrowI32x4U => {
            let (x, y) = (u32x4(a), u32x4(b));
            let sat = |v: u32| (v as i32).clamp(0, 65535) as u16;
            let mut r = [0u16; 8];
            for i in 0..4 {
                r[i] = sat(x[i]);
                r[i + 4] = sat(y[i]);
            }
            from_u16x8(r)
        }
        I8x16Add => zip8(a, b, u8::wrapping_add),
        I8x16Sub => zip8(a, b, u8::wrapping_sub),
        I8x16AddSatS => zip8(a, b, |x, y| (x as i8).saturating_add(y as i8) as u8),
        I8x16AddSatU => zip8(a, b, u8::saturating_add),
        I8x16SubSatS => zip8(a, b, |x, y| (x as i8).saturating_sub(y as i8) as u8),
        I8x16SubSatU => zip8(a, b, u8::saturating_sub),
        I8x16MinS => zip8(a, b, |x, y| (x as i8).min(y as i8) as u8),
        I8x16MinU => zip8(a, b, |x, y| x.min(y)),
        I8x16MaxS => zip8(a, b, |x, y| (x as i8).max(y as i8) as u8),
        I8x16MaxU => zip8(a, b, |x, y| x.max(y)),
        I8x16AvgrU => zip8(a, b, |x, y| (u16::from(x) + u16::from(y)).div_ceil(2) as u8),
        I16x8Add => zip16(a, b, u16::wrapping_add),
        I16x8Sub => zip16(a, b, u16::wrapping_sub),
        I16x8Mul => zip16(a, b, u16::wrapping_mul),
        I16x8AddSatS => zip16(a, b, |x, y| (x as i16).saturating_add(y as i16) as u16),
        I16x8AddSatU => zip16(a, b, u16::saturating_add),
        I16x8SubSatS => zip16(a, b, |x, y| (x as i16).saturating_sub(y as i16) as u16),
        I16x8SubSatU => zip16(a, b, u16::saturating_sub),
        I16x8MinS => zip16(a, b, |x, y| (x as i16).min(y as i16) as u16),
        I16x8MinU => zip16(a, b, |x, y| x.min(y)),
        I16x8MaxS => zip16(a, b, |x, y| (x as i16).max(y as i16) as u16),
        I16x8MaxU => zip16(a, b, |x, y| x.max(y)),
        I16x8AvgrU => zip16(a, b, |x, y| {
            (u32::from(x) + u32::from(y)).div_ceil(2) as u16
        }),
        I16x8Q15mulrSatS => zip16(a, b, |x, y| {
            let p = (i32::from(x as i16) * i32::from(y as i16) + 0x4000) >> 15;
            p.clamp(-32768, 32767) as i16 as u16
        }),
        I32x4Add => zip32(a, b, u32::wrapping_add),
        I32x4Sub => zip32(a, b, u32::wrapping_sub),
        I32x4Mul => zip32(a, b, u32::wrapping_mul),
        I32x4MinS => zip32(a, b, |x, y| (x as i32).min(y as i32) as u32),
        I32x4MinU => zip32(a, b, |x, y| x.min(y)),
        I32x4MaxS => zip32(a, b, |x, y| (x as i32).max(y as i32) as u32),
        I32x4MaxU => zip32(a, b, |x, y| x.max(y)),
        I32x4DotI16x8S => {
            let (x, y) = (u16x8(a), u16x8(b));
            from_u32x4(std::array::from_fn(|i| {
                let p1 = i32::from(x[2 * i] as i16) * i32::from(y[2 * i] as i16);
                let p2 = i32::from(x[2 * i + 1] as i16) * i32::from(y[2 * i + 1] as i16);
                p1.wrapping_add(p2) as u32
            }))
        }
        I64x2Add => zip64(a, b, u64::wrapping_add),
        I64x2Sub => zip64(a, b, u64::wrapping_sub),
        I64x2Mul => zip64(a, b, u64::wrapping_mul),
        I16x8ExtmulLowI8x16S => {
            let (x, y) = (b16(a), b16(b));
            from_u16x8(std::array::from_fn(|i| {
                (i16::from(x[i] as i8) * i16::from(y[i] as i8)) as u16
            }))
        }
        I16x8ExtmulHighI8x16S => {
            let (x, y) = (b16(a), b16(b));
            from_u16x8(std::array::from_fn(|i| {
                (i16::from(x[i + 8] as i8) * i16::from(y[i + 8] as i8)) as u16
            }))
        }
        I16x8ExtmulLowI8x16U => {
            let (x, y) = (b16(a), b16(b));
            from_u16x8(std::array::from_fn(|i| u16::from(x[i]) * u16::from(y[i])))
        }
        I16x8ExtmulHighI8x16U => {
            let (x, y) = (b16(a), b16(b));
            from_u16x8(std::array::from_fn(|i| {
                u16::from(x[i + 8]) * u16::from(y[i + 8])
            }))
        }
        I32x4ExtmulLowI16x8S => {
            let (x, y) = (u16x8(a), u16x8(b));
            from_u32x4(std::array::from_fn(|i| {
                (i32::from(x[i] as i16) * i32::from(y[i] as i16)) as u32
            }))
        }
        I32x4ExtmulHighI16x8S => {
            let (x, y) = (u16x8(a), u16x8(b));
            from_u32x4(std::array::from_fn(|i| {
                (i32::from(x[i + 4] as i16) * i32::from(y[i + 4] as i16)) as u32
            }))
        }
        I32x4ExtmulLowI16x8U => {
            let (x, y) = (u16x8(a), u16x8(b));
            from_u32x4(std::array::from_fn(|i| u32::from(x[i]) * u32::from(y[i])))
        }
        I32x4ExtmulHighI16x8U => {
            let (x, y) = (u16x8(a), u16x8(b));
            from_u32x4(std::array::from_fn(|i| {
                u32::from(x[i + 4]) * u32::from(y[i + 4])
            }))
        }
        I64x2ExtmulLowI32x4S => {
            let (x, y) = (u32x4(a), u32x4(b));
            from_u64x2(std::array::from_fn(|i| {
                (i64::from(x[i] as i32) * i64::from(y[i] as i32)) as u64
            }))
        }
        I64x2ExtmulHighI32x4S => {
            let (x, y) = (u32x4(a), u32x4(b));
            from_u64x2(std::array::from_fn(|i| {
                (i64::from(x[i + 2] as i32) * i64::from(y[i + 2] as i32)) as u64
            }))
        }
        I64x2ExtmulLowI32x4U => {
            let (x, y) = (u32x4(a), u32x4(b));
            from_u64x2(std::array::from_fn(|i| u64::from(x[i]) * u64::from(y[i])))
        }
        I64x2ExtmulHighI32x4U => {
            let (x, y) = (u32x4(a), u32x4(b));
            from_u64x2(std::array::from_fn(|i| {
                u64::from(x[i + 2]) * u64::from(y[i + 2])
            }))
        }
        F32x4Add => zipf32(a, b, |x, y| x + y),
        F32x4Sub => zipf32(a, b, |x, y| x - y),
        F32x4Mul => zipf32(a, b, |x, y| x * y),
        F32x4Div => zipf32(a, b, |x, y| x / y),
        F32x4Min => zipf32(a, b, fmin32),
        F32x4Max => zipf32(a, b, fmax32),
        F32x4Pmin => zipf32(a, b, |x, y| if y < x { y } else { x }),
        F32x4Pmax => zipf32(a, b, |x, y| if x < y { y } else { x }),
        F64x2Add => zipf64(a, b, |x, y| x + y),
        F64x2Sub => zipf64(a, b, |x, y| x - y),
        F64x2Mul => zipf64(a, b, |x, y| x * y),
        F64x2Div => zipf64(a, b, |x, y| x / y),
        F64x2Min => zipf64(a, b, fmin64),
        F64x2Max => zipf64(a, b, fmax64),
        F64x2Pmin => zipf64(a, b, |x, y| if y < x { y } else { x }),
        F64x2Pmax => zipf64(a, b, |x, y| if x < y { y } else { x }),
        other => unreachable!("simd op {other:?} handled earlier"),
    };
    m.push_v128(r);
    Ok(())
}

fn zip32u_from_f32(a: u128, b: u128, f: impl Fn(f32, f32) -> u32) -> u128 {
    let (x, y) = (f32x4(a), f32x4(b));
    from_u32x4(std::array::from_fn(|i| f(x[i], y[i])))
}

fn zip64u_from_f64(a: u128, b: u128, f: impl Fn(f64, f64) -> u64) -> u128 {
    let (x, y) = (f64x2(a), f64x2(b));
    from_u64x2(std::array::from_fn(|i| f(x[i], y[i])))
}

pub(crate) fn simd_lane(m: &mut Machine, op: SimdLaneOp, lane: u8) -> Exec {
    use SimdLaneOp::*;
    let l = lane as usize;
    match op {
        I8x16ExtractLaneS => {
            let a = m.pop_v128();
            m.push_value(Value::I32(b16(a)[l] as i8 as i32 as u32));
        }
        I8x16ExtractLaneU => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u32::from(b16(a)[l])));
        }
        I16x8ExtractLaneS => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u16x8(a)[l] as i16 as i32 as u32));
        }
        I16x8ExtractLaneU => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u32::from(u16x8(a)[l])));
        }
        I32x4ExtractLane => {
            let a = m.pop_v128();
            m.push_value(Value::I32(u32x4(a)[l]));
        }
        I64x2ExtractLane => {
            let a = m.pop_v128();
            m.push_value(Value::I64(u64x2(a)[l]));
        }
        F32x4ExtractLane => {
            let a = m.pop_v128();
            m.push_value(Value::F32(u32x4(a)[l]));
        }
        F64x2ExtractLane => {
            let a = m.pop_v128();
            m.push_value(Value::F64(u64x2(a)[l]));
        }
        I8x16ReplaceLane => {
            let Value::I32(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = b16(a);
            x[l] = v as u8;
            m.push_v128(from_b16(x));
        }
        I16x8ReplaceLane => {
            let Value::I32(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = u16x8(a);
            x[l] = v as u16;
            m.push_v128(from_u16x8(x));
        }
        I32x4ReplaceLane => {
            let Value::I32(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = u32x4(a);
            x[l] = v;
            m.push_v128(from_u32x4(x));
        }
        I64x2ReplaceLane => {
            let Value::I64(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = u64x2(a);
            x[l] = v;
            m.push_v128(from_u64x2(x));
        }
        F32x4ReplaceLane => {
            let Value::F32(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = u32x4(a);
            x[l] = v;
            m.push_v128(from_u32x4(x));
        }
        F64x2ReplaceLane => {
            let Value::F64(v) = m.pop_value() else {
                unreachable!()
            };
            let a = m.pop_v128();
            let mut x = u64x2(a);
            x[l] = v;
            m.push_v128(from_u64x2(x));
        }
    }
    Ok(())
}

pub(crate) fn shuffle(m: &mut Machine, lanes: &[u8; 16]) -> Exec {
    let b = m.pop_v128();
    let a = m.pop_v128();
    let (x, y) = (b16(a), b16(b));
    let r: [u8; 16] = std::array::from_fn(|i| {
        let j = lanes[i] as usize;
        if j < 16 {
            x[j]
        } else {
            y[j - 16]
        }
    });
    m.push_v128(from_b16(r));
    Ok(())
}

pub(crate) fn simd_mem(m: &mut Machine, op: SimdMemOp, offset: u32, lane: u8) -> Exec {
    use SimdMemOp::*;
    let l = lane as usize;
    match op {
        Load => {
            let v = m.mem_load(offset, 16)?;
            m.push_v128(v);
        }
        Load8x8S => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u16x8(std::array::from_fn(|i| {
                x[i] as i8 as i16 as u16
            })));
        }
        Load8x8U => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u16x8(std::array::from_fn(|i| u16::from(x[i]))));
        }
        Load16x4S => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u32x4(std::array::from_fn(|i| {
                i32::from(i16::from_le_bytes([x[2 * i], x[2 * i + 1]])) as u32
            })));
        }
        Load16x4U => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u32x4(std::array::from_fn(|i| {
                u32::from(u16::from_le_bytes([x[2 * i], x[2 * i + 1]]))
            })));
        }
        Load32x2S => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u64x2(std::array::from_fn(|i| {
                i64::from(i32::from_le_bytes(x[4 * i..4 * i + 4].try_into().unwrap())) as u64
            })));
        }
        Load32x2U => {
            let v = m.mem_load(offset, 8)? as u64;
            let x = v.to_le_bytes();
            m.push_v128(from_u64x2(std::array::from_fn(|i| {
                u64::from(u32::from_le_bytes(x[4 * i..4 * i + 4].try_into().unwrap()))
            })));
        }
        Load8Splat => {
            let v = m.mem_load(offset, 1)? as u8;
            m.push_v128(from_b16([v; 16]));
        }
        Load16Splat => {
            let v = m.mem_load(offset, 2)? as u16;
            m.push_v128(from_u16x8([v; 8]));
        }
        Load32Splat => {
            let v = m.mem_load(offset, 4)? as u32;
            m.push_v128(from_u32x4([v; 4]));
        }
        Load64Splat => {
            let v = m.mem_load(offset, 8)? as u64;
            m.push_v128(from_u64x2([v; 2]));
        }
        Load32Zero => {
            let v = m.mem_load(offset, 4)? as u32;
            m.push_v128(u128::from(v));
        }
        Load64Zero => {
            let v = m.mem_load(offset, 8)? as u64;
            m.push_v128(u128::from(v));
        }
        Store => {
            let v = m.pop_v128();
            m.mem_store(offset, &v.to_le_bytes())?;
        }
        Load8Lane => {
            let prev = m.pop_v128();
            let v = m.mem_load(offset, 1)? as u8;
            let mut x = b16(prev);
            x[l] = v;
            m.push_v128(from_b16(x));
        }
        Load16Lane => {
            let prev = m.pop_v128();
            let v = m.mem_load(offset, 2)? as u16;
            let mut x = u16x8(prev);
            x[l] = v;
            m.push_v128(from_u16x8(x));
        }
        Load32Lane => {
            let prev = m.pop_v128();
            let v = m.mem_load(offset, 4)? as u32;
            let mut x = u32x4(prev);
            x[l] = v;
            m.push_v128(from_u32x4(x));
        }
        Load64Lane => {
            let prev = m.pop_v128();
            let v = m.mem_load(offset, 8)? as u64;
            let mut x = u64x2(prev);
            x[l] = v;
            m.push_v128(from_u64x2(x));
        }
        Store8Lane => {
            let v = m.pop_v128();
            m.mem_store(offset, &[b16(v)[l]])?;
        }
        Store16Lane => {
            let v = m.pop_v128();
            m.mem_store(offset, &u16x8(v)[l].to_le_bytes())?;
        }
        Store32Lane => {
            let v = m.pop_v128();
            m.mem_store(offset, &u32x4(v)[l].to_le_bytes())?;
        }
        Store64Lane => {
            let v = m.pop_v128();
            m.mem_store(offset, &u64x2(v)[l].to_le_bytes())?;
        }
    }
    Ok(())
}
