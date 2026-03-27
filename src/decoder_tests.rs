use super::*;

fn u32_to_regs_be(val: u32) -> Vec<u16> {
    let b = val.to_be_bytes();
    vec![
        u16::from_be_bytes([b[0], b[1]]),
        u16::from_be_bytes([b[2], b[3]]),
    ]
}

fn u64_to_regs_be(val: u64) -> Vec<u16> {
    let b = val.to_be_bytes();
    vec![
        u16::from_be_bytes([b[0], b[1]]),
        u16::from_be_bytes([b[2], b[3]]),
        u16::from_be_bytes([b[4], b[5]]),
        u16::from_be_bytes([b[6], b[7]]),
    ]
}

fn u32_to_regs_le(val: u32) -> Vec<u16> {
    let be = u32_to_regs_be(val);
    vec![be[1].swap_bytes(), be[0].swap_bytes()]
}

fn u32_to_regs_mbe(val: u32) -> Vec<u16> {
    let be = u32_to_regs_be(val);
    vec![be[1], be[0]]
}

fn u32_to_regs_mle(val: u32) -> Vec<u16> {
    let be = u32_to_regs_be(val);
    vec![be[0].swap_bytes(), be[1].swap_bytes()]
}

fn u64_to_regs_le(val: u64) -> Vec<u16> {
    let be = u64_to_regs_be(val);
    vec![
        be[3].swap_bytes(),
        be[2].swap_bytes(),
        be[1].swap_bytes(),
        be[0].swap_bytes(),
    ]
}

fn u64_to_regs_mbe(val: u64) -> Vec<u16> {
    let be = u64_to_regs_be(val);
    vec![be[2], be[3], be[0], be[1]]
}

fn u64_to_regs_mle(val: u64) -> Vec<u16> {
    let be = u64_to_regs_be(val);
    vec![
        be[0].swap_bytes(),
        be[1].swap_bytes(),
        be[2].swap_bytes(),
        be[3].swap_bytes(),
    ]
}

// Bool
#[test]
fn test_bool_true() {
    assert_eq!(
        decode(&[1], DataType::Bool, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        1.0
    );
}

#[test]
fn test_bool_false() {
    assert_eq!(
        decode(&[0], DataType::Bool, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        0.0
    );
}

#[test]
fn test_bool_nonzero() {
    assert_eq!(
        decode(&[255], DataType::Bool, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        1.0
    );
}

#[test]
fn test_bool_inversion() {
    assert_eq!(
        decode(&[1], DataType::Bool, ByteOrder::BigEndian, -1.0, 1.0).unwrap(),
        0.0
    );
    assert_eq!(
        decode(&[0], DataType::Bool, ByteOrder::BigEndian, -1.0, 1.0).unwrap(),
        1.0
    );
}

// U16
#[test]
fn test_u16_basic() {
    assert_eq!(
        decode(&[12345], DataType::U16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        12345.0
    );
}

#[test]
fn test_u16_max() {
    assert_eq!(
        decode(&[65535], DataType::U16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        65535.0
    );
}

#[test]
fn test_u16_zero() {
    assert_eq!(
        decode(&[0], DataType::U16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        0.0
    );
}

#[test]
fn test_u16_scale_offset() {
    assert_eq!(
        decode(&[100], DataType::U16, ByteOrder::BigEndian, 0.1, 5.0).unwrap(),
        15.0
    );
}

// I16
#[test]
fn test_i16_positive() {
    assert_eq!(
        decode(&[100], DataType::I16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        100.0
    );
}

#[test]
fn test_i16_negative() {
    assert_eq!(
        decode(&[65535], DataType::I16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        -1.0
    );
}

#[test]
fn test_i16_min() {
    assert_eq!(
        decode(&[32768], DataType::I16, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        -32768.0
    );
}

// U32 — all byte orders
#[test]
fn test_u32_be() {
    let regs = u32_to_regs_be(123456);
    assert_eq!(
        decode(&regs, DataType::U32, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        123456.0
    );
}

#[test]
fn test_u32_le() {
    let regs = u32_to_regs_le(123456);
    assert_eq!(
        decode(&regs, DataType::U32, ByteOrder::LittleEndian, 1.0, 0.0).unwrap(),
        123456.0
    );
}

#[test]
fn test_u32_mbe() {
    let regs = u32_to_regs_mbe(123456);
    assert_eq!(
        decode(&regs, DataType::U32, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap(),
        123456.0
    );
}

#[test]
fn test_u32_mle() {
    let regs = u32_to_regs_mle(123456);
    assert_eq!(
        decode(&regs, DataType::U32, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap(),
        123456.0
    );
}

#[test]
fn test_u32_max() {
    let regs = u32_to_regs_be(u32::MAX);
    assert_eq!(
        decode(&regs, DataType::U32, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        u32::MAX as f64
    );
}

// I32 — all byte orders
#[test]
fn test_i32_be_pos() {
    let regs = u32_to_regs_be(123456_u32);
    assert_eq!(
        decode(&regs, DataType::I32, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        123456.0
    );
}

#[test]
fn test_i32_be_neg() {
    let regs = u32_to_regs_be((-123456_i32) as u32);
    assert_eq!(
        decode(&regs, DataType::I32, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        -123456.0
    );
}

#[test]
fn test_i32_le() {
    let regs = u32_to_regs_le((-99999_i32) as u32);
    assert_eq!(
        decode(&regs, DataType::I32, ByteOrder::LittleEndian, 1.0, 0.0).unwrap(),
        -99999.0
    );
}

#[test]
fn test_i32_mbe() {
    let regs = u32_to_regs_mbe((-99999_i32) as u32);
    assert_eq!(
        decode(&regs, DataType::I32, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap(),
        -99999.0
    );
}

#[test]
fn test_i32_mle() {
    let regs = u32_to_regs_mle((-99999_i32) as u32);
    assert_eq!(
        decode(&regs, DataType::I32, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap(),
        -99999.0
    );
}

// F32 — all byte orders
#[test]
fn test_f32_be() {
    let regs = u32_to_regs_be(3.14_f32.to_bits());
    let r = decode(&regs, DataType::F32, ByteOrder::BigEndian, 1.0, 0.0).unwrap();
    assert!((r - 3.14).abs() < 1e-5);
}

#[test]
fn test_f32_le() {
    let regs = u32_to_regs_le((-273.15_f32).to_bits());
    let r = decode(&regs, DataType::F32, ByteOrder::LittleEndian, 1.0, 0.0).unwrap();
    assert!((r - (-273.15)).abs() < 0.01);
}

#[test]
fn test_f32_mbe() {
    let regs = u32_to_regs_mbe(42.5_f32.to_bits());
    let r = decode(&regs, DataType::F32, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap();
    assert!((r - 42.5).abs() < 1e-5);
}

#[test]
fn test_f32_mle() {
    let regs = u32_to_regs_mle(42.5_f32.to_bits());
    let r = decode(&regs, DataType::F32, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap();
    assert!((r - 42.5).abs() < 1e-5);
}

#[test]
fn test_f32_nan() {
    let regs = u32_to_regs_be(f32::NAN.to_bits());
    assert!(decode(&regs, DataType::F32, ByteOrder::BigEndian, 1.0, 0.0)
        .unwrap()
        .is_nan());
}

#[test]
fn test_f32_inf() {
    let regs = u32_to_regs_be(f32::INFINITY.to_bits());
    let r = decode(&regs, DataType::F32, ByteOrder::BigEndian, 1.0, 0.0).unwrap();
    assert!(r.is_infinite() && r > 0.0);
}

// U64 — all byte orders
#[test]
fn test_u64_be() {
    let regs = u64_to_regs_be(1_000_000_000);
    assert_eq!(
        decode(&regs, DataType::U64, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        1e9
    );
}

#[test]
fn test_u64_le() {
    let regs = u64_to_regs_le(1_000_000_000);
    assert_eq!(
        decode(&regs, DataType::U64, ByteOrder::LittleEndian, 1.0, 0.0).unwrap(),
        1e9
    );
}

#[test]
fn test_u64_mbe() {
    let regs = u64_to_regs_mbe(1_000_000_000);
    assert_eq!(
        decode(&regs, DataType::U64, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap(),
        1e9
    );
}

#[test]
fn test_u64_mle() {
    let regs = u64_to_regs_mle(1_000_000_000);
    assert_eq!(
        decode(&regs, DataType::U64, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap(),
        1e9
    );
}

// I64 — all byte orders
#[test]
fn test_i64_be() {
    let regs = u64_to_regs_be((-1_000_000_000_i64) as u64);
    assert_eq!(
        decode(&regs, DataType::I64, ByteOrder::BigEndian, 1.0, 0.0).unwrap(),
        -1e9
    );
}

#[test]
fn test_i64_le() {
    let regs = u64_to_regs_le((-1_000_000_000_i64) as u64);
    assert_eq!(
        decode(&regs, DataType::I64, ByteOrder::LittleEndian, 1.0, 0.0).unwrap(),
        -1e9
    );
}

#[test]
fn test_i64_mbe() {
    let regs = u64_to_regs_mbe((-1_000_000_000_i64) as u64);
    assert_eq!(
        decode(&regs, DataType::I64, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap(),
        -1e9
    );
}

#[test]
fn test_i64_mle() {
    let regs = u64_to_regs_mle((-1_000_000_000_i64) as u64);
    assert_eq!(
        decode(&regs, DataType::I64, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap(),
        -1e9
    );
}

// F64 — all byte orders
#[test]
fn test_f64_be() {
    let regs = u64_to_regs_be(std::f64::consts::PI.to_bits());
    let r = decode(&regs, DataType::F64, ByteOrder::BigEndian, 1.0, 0.0).unwrap();
    assert!((r - std::f64::consts::PI).abs() < 1e-15);
}

#[test]
fn test_f64_le() {
    let regs = u64_to_regs_le((-273.15_f64).to_bits());
    let r = decode(&regs, DataType::F64, ByteOrder::LittleEndian, 1.0, 0.0).unwrap();
    assert!((r - (-273.15)).abs() < 1e-10);
}

#[test]
fn test_f64_mbe() {
    let regs = u64_to_regs_mbe(99999.99_f64.to_bits());
    let r = decode(&regs, DataType::F64, ByteOrder::MidBigEndian, 1.0, 0.0).unwrap();
    assert!((r - 99999.99).abs() < 1e-10);
}

#[test]
fn test_f64_mle() {
    let regs = u64_to_regs_mle(99999.99_f64.to_bits());
    let r = decode(&regs, DataType::F64, ByteOrder::MidLittleEndian, 1.0, 0.0).unwrap();
    assert!((r - 99999.99).abs() < 1e-10);
}

#[test]
fn test_f64_nan() {
    let regs = u64_to_regs_be(f64::NAN.to_bits());
    assert!(decode(&regs, DataType::F64, ByteOrder::BigEndian, 1.0, 0.0)
        .unwrap()
        .is_nan());
}

// Scale and offset
#[test]
fn test_scale_only() {
    assert_eq!(
        decode(&[500], DataType::U16, ByteOrder::BigEndian, 0.01, 0.0).unwrap(),
        5.0
    );
}

#[test]
fn test_offset_only() {
    assert_eq!(
        decode(&[100], DataType::U16, ByteOrder::BigEndian, 1.0, -50.0).unwrap(),
        50.0
    );
}

#[test]
fn test_scale_and_offset() {
    assert_eq!(
        decode(&[200], DataType::U16, ByteOrder::BigEndian, 0.1, 10.0).unwrap(),
        30.0
    );
}

// Errors
#[test]
fn test_insufficient_u32() {
    assert_eq!(
        decode(&[1], DataType::U32, ByteOrder::BigEndian, 1.0, 0.0),
        Err(DecodeError::InsufficientRegisters {
            expected: 2,
            got: 1
        })
    );
}

#[test]
fn test_insufficient_u64() {
    assert_eq!(
        decode(&[1, 2], DataType::U64, ByteOrder::BigEndian, 1.0, 0.0),
        Err(DecodeError::InsufficientRegisters {
            expected: 4,
            got: 2
        })
    );
}

#[test]
fn test_empty() {
    assert_eq!(
        decode(&[], DataType::U16, ByteOrder::BigEndian, 1.0, 0.0),
        Err(DecodeError::InsufficientRegisters {
            expected: 1,
            got: 0
        })
    );
}

#[test]
fn test_extra_regs_ignored() {
    assert_eq!(
        decode(
            &[42, 99, 100, 200],
            DataType::U16,
            ByteOrder::BigEndian,
            1.0,
            0.0
        )
        .unwrap(),
        42.0
    );
}
