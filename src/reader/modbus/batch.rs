//! Batch read with register coalescing for Modbus.
//!
//! Groups metrics by register type, sorts by address, merges adjacent/overlapping
//! ranges (gap ≤ 10 registers), issues one Modbus read per merged range, then
//! splits the response back. Falls back to individual reads on failure.

#[cfg(test)]
mod batch_tests;

use anyhow::Result;
use tracing::warn;

use crate::bus;
use crate::config::{self, RegisterType};
use crate::decoder;

use super::{ModbusReader, MAX_REGISTERS_PER_READ};

/// Maximum gap (in registers) between two ranges to still merge them.
const MAX_COALESCE_GAP: u16 = 10;

/// A metric with its index in the original input slice (for result ordering).
#[derive(Debug, Clone)]
struct IndexedMetric<'a> {
    idx: usize,
    metric: &'a config::Metric,
    addr: u16,
    count: u16,
}

/// A merged range covering one or more metrics.
#[derive(Debug, Clone)]
struct MergedRange<'a> {
    start: u16,
    end: u16, // exclusive
    members: Vec<IndexedMetric<'a>>,
}

impl<'a> MergedRange<'a> {
    fn count(&self) -> u16 {
        self.end - self.start
    }
}

/// Coalesce sorted ranges into merged ranges.
///
/// `items` must already be sorted by `addr`.
fn coalesce<'a>(items: Vec<IndexedMetric<'a>>) -> Vec<MergedRange<'a>> {
    if items.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<MergedRange<'a>> = Vec::new();
    for item in items {
        let item_end = item.addr.saturating_add(item.count);
        if let Some(last) = ranges.last_mut() {
            // Merge if gap <= MAX_COALESCE_GAP and merged count fits in one read
            let gap = item.addr.saturating_sub(last.end);
            let merged_count = item_end.saturating_sub(last.start);
            if gap <= MAX_COALESCE_GAP && merged_count <= MAX_REGISTERS_PER_READ {
                last.end = last.end.max(item_end);
                last.members.push(item);
                continue;
            }
        }
        ranges.push(MergedRange {
            start: item.addr,
            end: item_end,
            members: vec![item],
        });
    }
    ranges
}

/// Extract a single metric's value from a register buffer read starting at `range_start`.
fn decode_metric(metric: &config::Metric, regs: &[u16], range_start: u16) -> Result<f64> {
    let addr = metric.address.unwrap();
    let count = metric.data_type.register_count();
    let offset_in_buf = (addr - range_start) as usize;
    let slice = &regs[offset_in_buf..offset_in_buf + count as usize];
    let data_type = bus::map_data_type(metric.data_type);
    let byte_order = bus::map_byte_order(metric.byte_order);
    decoder::decode(slice, data_type, byte_order, metric.scale, metric.offset)
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Read a single metric individually (fallback path).
async fn read_single(reader: &mut dyn ModbusReader, metric: &config::Metric) -> Result<f64> {
    let addr = metric.address.unwrap();
    let count = metric.data_type.register_count();
    let register_type = metric.register_type.unwrap_or(RegisterType::Holding);
    let data_type = bus::map_data_type(metric.data_type);
    let byte_order = bus::map_byte_order(metric.byte_order);

    match register_type {
        RegisterType::Holding => {
            let regs = reader.read_holding_registers(addr, count).await?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Input => {
            let regs = reader.read_input_registers(addr, count).await?;
            decoder::decode(&regs, data_type, byte_order, metric.scale, metric.offset)
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
        RegisterType::Coil => {
            let bits = reader.read_coils(addr, 1).await?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty coil response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
        RegisterType::Discrete => {
            let bits = reader.read_discrete_inputs(addr, 1).await?;
            let val = bits
                .first()
                .ok_or_else(|| anyhow::anyhow!("empty discrete input response"))?;
            let raw = if *val { 1.0 } else { 0.0 };
            Ok(raw * metric.scale + metric.offset)
        }
    }
}

/// Perform a batch read with register coalescing.
///
/// Metrics are grouped by register type, coalesced, read in bulk, then
/// split back. Coils/discrete inputs are read individually (not coalesced).
/// On any batch failure, falls back to individual reads for that range.
pub async fn batch_read_coalesced<'a>(
    reader: &mut dyn ModbusReader,
    metrics: &'a [config::Metric],
) -> Vec<(&'a config::Metric, Result<f64>)> {
    let mut results: Vec<Option<Result<f64>>> = (0..metrics.len()).map(|_| None).collect();

    // Separate register-based metrics (holding/input) from bit-based (coil/discrete).
    let mut holding: Vec<IndexedMetric<'_>> = Vec::new();
    let mut input: Vec<IndexedMetric<'_>> = Vec::new();
    let mut individual: Vec<(usize, &config::Metric)> = Vec::new();

    for (idx, m) in metrics.iter().enumerate() {
        let Some(addr) = m.address else {
            results[idx] = Some(Err(anyhow::anyhow!("metric '{}' has no address", m.name)));
            continue;
        };
        let count = m.data_type.register_count();
        let rt = m.register_type.unwrap_or(RegisterType::Holding);
        match rt {
            RegisterType::Holding => holding.push(IndexedMetric {
                idx,
                metric: m,
                addr,
                count,
            }),
            RegisterType::Input => input.push(IndexedMetric {
                idx,
                metric: m,
                addr,
                count,
            }),
            RegisterType::Coil | RegisterType::Discrete => {
                individual.push((idx, m));
            }
        }
    }

    // Coalesce and read holding registers
    holding.sort_by_key(|im| im.addr);
    let holding_ranges = coalesce(holding);
    for range in holding_ranges {
        match reader
            .read_holding_registers(range.start, range.count())
            .await
        {
            Ok(regs) => {
                for member in &range.members {
                    results[member.idx] = Some(decode_metric(member.metric, &regs, range.start));
                }
            }
            Err(e) => {
                warn!(
                    start = range.start,
                    count = range.count(),
                    error = %e,
                    "batch holding read failed, falling back to individual reads"
                );
                for member in &range.members {
                    results[member.idx] = Some(read_single(reader, member.metric).await);
                }
            }
        }
    }

    // Coalesce and read input registers
    input.sort_by_key(|im| im.addr);
    let input_ranges = coalesce(input);
    for range in input_ranges {
        match reader
            .read_input_registers(range.start, range.count())
            .await
        {
            Ok(regs) => {
                for member in &range.members {
                    results[member.idx] = Some(decode_metric(member.metric, &regs, range.start));
                }
            }
            Err(e) => {
                warn!(
                    start = range.start,
                    count = range.count(),
                    error = %e,
                    "batch input read failed, falling back to individual reads"
                );
                for member in &range.members {
                    results[member.idx] = Some(read_single(reader, member.metric).await);
                }
            }
        }
    }

    // Read coils/discrete individually
    for (idx, m) in individual {
        results[idx] = Some(read_single(reader, m).await);
    }

    // Build output
    metrics
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            let r = results[idx]
                .take()
                .unwrap_or_else(|| Err(anyhow::anyhow!("metric '{}' not processed", m.name)));
            (m, r)
        })
        .collect()
}

#[cfg(test)]
pub use self::batch_tests::*;
