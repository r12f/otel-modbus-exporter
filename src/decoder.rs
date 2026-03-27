use std::fmt;

/// Byte order for multi-register values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
    MidBigEndian,
    MidLittleEndian,
}

/// Supported data types for register decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    U16,
    I16,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
    Bool,
}

/// Errors that can occur during decoding.
#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    InsufficientRegisters { expected: usize, got: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::InsufficientRegisters { expected, got } => {
                write!(f, "insufficient registers: expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Required number of registers for a data type.
pub fn registers_needed(data_type: DataType) -> usize {
    match data_type {
        DataType::U16 | DataType::I16 | DataType::Bool => 1,
        DataType::U32 | DataType::I32 | DataType::F32 => 2,
        DataType::U64 | DataType::I64 | DataType::F64 => 4,
    }
}

fn reorder_32(regs: &[u16], byte_order: ByteOrder) -> [u8; 4] {
    let r0 = regs[0].to_be_bytes();
    let r1 = regs[1].to_be_bytes();
    match byte_order {
        ByteOrder::BigEndian => [r0[0], r0[1], r1[0], r1[1]],
        ByteOrder::LittleEndian => [r1[1], r1[0], r0[1], r0[0]],
        ByteOrder::MidBigEndian => [r1[0], r1[1], r0[0], r0[1]],
        ByteOrder::MidLittleEndian => [r0[1], r0[0], r1[1], r1[0]],
    }
}

fn reorder_64(regs: &[u16], byte_order: ByteOrder) -> [u8; 8] {
    let r0 = regs[0].to_be_bytes();
    let r1 = regs[1].to_be_bytes();
    let r2 = regs[2].to_be_bytes();
    let r3 = regs[3].to_be_bytes();
    match byte_order {
        ByteOrder::BigEndian => [r0[0], r0[1], r1[0], r1[1], r2[0], r2[1], r3[0], r3[1]],
        ByteOrder::LittleEndian => [r3[1], r3[0], r2[1], r2[0], r1[1], r1[0], r0[1], r0[0]],
        ByteOrder::MidBigEndian => [r2[0], r2[1], r3[0], r3[1], r0[0], r0[1], r1[0], r1[1]],
        ByteOrder::MidLittleEndian => [r0[1], r0[0], r1[1], r1[0], r2[1], r2[0], r3[1], r3[0]],
    }
}

/// Decode raw Modbus registers into an `f64` metric value.
pub fn decode(
    registers: &[u16],
    data_type: DataType,
    byte_order: ByteOrder,
    scale: f64,
    offset: f64,
) -> Result<f64, DecodeError> {
    let needed = registers_needed(data_type);
    if registers.len() < needed {
        return Err(DecodeError::InsufficientRegisters {
            expected: needed,
            got: registers.len(),
        });
    }

    let raw: f64 = match data_type {
        DataType::Bool => {
            if registers[0] != 0 {
                1.0
            } else {
                0.0
            }
        }
        DataType::U16 => f64::from(registers[0]),
        DataType::I16 => f64::from(registers[0] as i16),
        DataType::U32 => {
            let bytes = reorder_32(registers, byte_order);
            f64::from(u32::from_be_bytes(bytes))
        }
        DataType::I32 => {
            let bytes = reorder_32(registers, byte_order);
            f64::from(i32::from_be_bytes(bytes))
        }
        DataType::F32 => {
            let bytes = reorder_32(registers, byte_order);
            f64::from(f32::from_be_bytes(bytes))
        }
        DataType::U64 => {
            let bytes = reorder_64(registers, byte_order);
            u64::from_be_bytes(bytes) as f64
        }
        DataType::I64 => {
            let bytes = reorder_64(registers, byte_order);
            i64::from_be_bytes(bytes) as f64
        }
        DataType::F64 => {
            let bytes = reorder_64(registers, byte_order);
            f64::from_be_bytes(bytes)
        }
    };

    Ok(raw * scale + offset)
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod decoder_tests;
