# Decoder Specification

## Overview

The decoder converts raw Modbus register values (or raw I2C/SPI bytes) into typed metric values, applying byte order reordering, type casting, and scale+offset transformation. It lives at `src/reader/decoder.rs` inside the reader module.

## Return Type

Both `decode()` and `decode_bytes()` return `Result<(f64, f64), DecodeError>` — a tuple of `(raw_value, scaled_value)`:

- **`raw_value`** — The decoded numeric value before scale/offset is applied.
- **`scaled_value`** — `raw_value * scale + offset`.

```rust
pub fn decode(registers: &[u16], data_type: DataType, byte_order: ByteOrder,
              scale: f64, offset: f64) -> Result<(f64, f64), DecodeError>;

pub fn decode_bytes(bytes: &[u8], data_type: DataType, byte_order: ByteOrder,
                    scale: f64, offset: f64) -> Result<(f64, f64), DecodeError>;
```

## Config Mapping Functions

These functions (moved from the former `bus.rs`) map config types to decoder types:

- **`map_byte_order(config::ByteOrder) -> ByteOrder`** — Maps config byte order enum to decoder byte order.
- **`map_data_type(config::DataType) -> DataType`** — Maps config data type enum to decoder data type.

## Byte Order Support

For multi-register types (32-bit = 2 registers, 64-bit = 4 registers):

| Byte Order | 16-bit | 32-bit (regs R0, R1) | 64-bit (regs R0, R1, R2, R3) |
|------------|--------|----------------------|-------------------------------|
| `big_endian` | AB | AB CD (R0·R1) | AB CD EF GH (R0·R1·R2·R3) |
| `little_endian` | BA | DC BA (R1·R0 swapped) | HG FE DC BA (R3·R2·R1·R0) |
| `mid_big_endian` | AB | CD AB (R1·R0) | EF GH AB CD (R2·R3·R0·R1) |
| `mid_little_endian` | BA | BA DC (R0-swap·R1-swap) | BA DC FE HG (R0·R1·R2·R3 each swapped) |

Note: For single-register types (`u16`, `i16`), byte order is ignored (Modbus defines big-endian wire format).

## Data Type Conversions

| `data_type` | Registers | Conversion |
|-------------|-----------|------------|
| `u16` | 1 | Direct u16 |
| `i16` | 1 | Reinterpret as i16 |
| `u32` | 2 | Combine per byte order, interpret as u32 |
| `i32` | 2 | Combine per byte order, interpret as i32 |
| `f32` | 2 | Combine per byte order, interpret as IEEE 754 f32 |
| `u64` | 4 | Combine per byte order, interpret as u64 |
| `i64` | 4 | Combine per byte order, interpret as i64 |
| `f64` | 4 | Combine per byte order, interpret as IEEE 754 f64 |
| `bool` | coil/discrete | `true` = 1.0, `false` = 0.0 |

## Scale and Offset

After type conversion to `f64`:

```text
output = raw_value * scale + offset
```

- `scale` default: `1.0`
- `offset` default: `0.0`
- All metric values are stored as `f64` internally.

## Bool Handling

- `coil` and `discrete` register types return boolean values.
- Converted to `f64`: `true` → `1.0`, `false` → `0.0`.
- Scale and offset are still applied (allows inversion: `scale: -1.0, offset: 1.0`).

## Error Cases

- Insufficient registers returned for the data type → error.
- NaN/Inf f32/f64 values → logged as warning, value is still stored.
