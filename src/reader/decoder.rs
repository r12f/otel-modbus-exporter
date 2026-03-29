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
    U8,
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
        DataType::U8 | DataType::U16 | DataType::I16 | DataType::Bool => 1,
        DataType::U32 | DataType::I32 | DataType::F32 => 2,
        DataType::U64 | DataType::I64 | DataType::F64 => 4,
    }
}

/// Returns the number of raw bytes needed for an I2C read of this data type.
pub fn byte_count(data_type: DataType) -> usize {
    match data_type {
        DataType::Bool | DataType::U8 => 1,
        DataType::U16 | DataType::I16 => 2,
        DataType::U32 | DataType::I32 | DataType::F32 => 4,
        DataType::U64 | DataType::I64 | DataType::F64 => 8,
    }
}

/// Decode raw I2C bytes into an `f64` metric value.
pub fn decode_bytes(
    bytes: &[u8],
    data_type: DataType,
    byte_order: ByteOrder,
    scale: f64,
    offset: f64,
) -> Result<(f64, f64), DecodeError> {
    let needed = byte_count(data_type);
    if bytes.len() < needed {
        return Err(DecodeError::InsufficientRegisters {
            expected: needed,
            got: bytes.len(),
        });
    }

    let raw: f64 = match data_type {
        DataType::Bool => {
            if bytes[0] & 0x01 != 0 {
                1.0
            } else {
                0.0
            }
        }
        DataType::U8 => f64::from(bytes[0]),
        DataType::U16 => {
            let b = &bytes[..2];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => u16::from_be_bytes([b[0], b[1]]),
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    u16::from_le_bytes([b[0], b[1]])
                }
            };
            f64::from(val)
        }
        DataType::I16 => {
            let b = &bytes[..2];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => i16::from_be_bytes([b[0], b[1]]),
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    i16::from_le_bytes([b[0], b[1]])
                }
            };
            f64::from(val)
        }
        DataType::U32 => {
            let b = &bytes[..4];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }
            };
            f64::from(val)
        }
        DataType::I32 => {
            let b = &bytes[..4];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    i32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }
            };
            f64::from(val)
        }
        DataType::F32 => {
            let b = &bytes[..4];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    f32::from_be_bytes([b[0], b[1], b[2], b[3]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                }
            };
            f64::from(val)
        }
        DataType::U64 => {
            let b = &bytes[..8];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
            };
            val as f64
        }
        DataType::I64 => {
            let b = &bytes[..8];
            let val = match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
            };
            val as f64
        }
        DataType::F64 => {
            let b = &bytes[..8];
            match byte_order {
                ByteOrder::BigEndian | ByteOrder::MidBigEndian => {
                    f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
                ByteOrder::LittleEndian | ByteOrder::MidLittleEndian => {
                    f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
                }
            }
        }
    };

    Ok((raw, raw * scale + offset))
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
) -> Result<(f64, f64), DecodeError> {
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
        DataType::U8 => f64::from(registers[0] as u8),
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

    Ok((raw, raw * scale + offset))
}

#[cfg(test)]
#[path = "decoder_tests.rs"]
mod decoder_tests;

// ── Config → decoder mappings (moved from bus.rs) ─────────────────────

use crate::config;

/// Map config byte order to decoder byte order.
pub fn map_byte_order(bo: config::ByteOrder) -> ByteOrder {
    match bo {
        config::ByteOrder::BigEndian => ByteOrder::BigEndian,
        config::ByteOrder::LittleEndian => ByteOrder::LittleEndian,
        config::ByteOrder::MidBigEndian => ByteOrder::MidBigEndian,
        config::ByteOrder::MidLittleEndian => ByteOrder::MidLittleEndian,
    }
}

/// Map config data type to decoder data type.
pub fn map_data_type(dt: config::DataType) -> DataType {
    match dt {
        config::DataType::U8 => DataType::U8,
        config::DataType::U16 => DataType::U16,
        config::DataType::I16 => DataType::I16,
        config::DataType::U32 => DataType::U32,
        config::DataType::I32 => DataType::I32,
        config::DataType::F32 => DataType::F32,
        config::DataType::U64 => DataType::U64,
        config::DataType::I64 => DataType::I64,
        config::DataType::F64 => DataType::F64,
        config::DataType::Bool => DataType::Bool,
    }
}
