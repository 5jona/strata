//! Fast zero-dependency Perfetto protobuf wire scanner for lightweight on-device trace summaries.

use std::collections::BTreeMap;

use serde_json::{Value, json};

/// Scans Perfetto `Trace` protobuf bytes and summarizes packet counts, types, and duration.
pub(crate) fn summarize_trace_bytes(data: &[u8], max_packets: usize) -> Value {
    let mut pos = 0usize;
    let mut pkt_count = 0usize;
    let mut pkt_types: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut min_ts: Option<u64> = None;
    let mut max_ts: Option<u64> = None;

    while pos < data.len() && pkt_count < max_packets {
        let Some((tag, next)) = read_varint(data, pos) else {
            break;
        };
        pos = next;
        let (field, wire) = (tag >> 3, tag & 0x07);
        if field == 1 && wire == 2 {
            let Some((len, p2)) = read_varint(data, pos) else {
                break;
            };
            let end = (p2 + (len as usize)).min(data.len());
            parse_packet(&data[p2..end], &mut pkt_types, &mut min_ts, &mut max_ts);
            pkt_count += 1;
            pos = end;
        } else {
            let Some(p2) = skip_wire(data, pos, wire) else {
                break;
            };
            pos = p2;
        }
    }
    let dur_ms = match (min_ts, max_ts) {
        (Some(a), Some(b)) if b >= a => ((b - a) as f64 / 1_000_000.0 * 100.0).round() / 100.0,
        _ => 0.0,
    };
    json!({
        "ok": true,
        "packetCount": pkt_count,
        "traceDurationMs": dur_ms,
        "packetTypes": pkt_types,
    })
}

fn parse_packet(
    buf: &[u8],
    types: &mut BTreeMap<&'static str, usize>,
    min_ts: &mut Option<u64>,
    max_ts: &mut Option<u64>,
) {
    let mut pos = 0usize;
    while pos < buf.len() {
        let Some((tag, p1)) = read_varint(buf, pos) else {
            break;
        };
        pos = p1;
        let (field, wire) = (tag >> 3, tag & 0x07);
        if field == 8 && wire == 0 {
            if let Some((ts, p2)) = read_varint(buf, pos) {
                if ts > 0 {
                    *min_ts = Some(min_ts.map_or(ts, |m| m.min(ts)));
                    *max_ts = Some(max_ts.map_or(ts, |m| m.max(ts)));
                }
                pos = p2;
            } else {
                break;
            }
        } else {
            let label = match field {
                1 => Some("ftrace_events"),
                2 => Some("process_tree"),
                3 => Some("sys_stats"),
                6 => Some("clock_snapshot"),
                11 => Some("track_event"),
                39 => Some("android_log"),
                74 => Some("frame_timeline_event"),
                _ => None,
            };
            if let Some(name) = label {
                *types.entry(name).or_insert(0) += 1;
            }
            let Some(p2) = skip_wire(buf, pos, wire) else {
                break;
            };
            pos = p2;
        }
    }
}

fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let (mut val, mut shift) = (0u64, 0u32);
    while pos < buf.len() && shift < 70 {
        let b = buf[pos];
        pos += 1;
        val |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some((val, pos));
        }
        shift += 7;
    }
    None
}

fn skip_wire(buf: &[u8], pos: usize, wire: u64) -> Option<usize> {
    match wire {
        0 => read_varint(buf, pos).map(|(_, p)| p),
        1 => Some((pos + 8).min(buf.len())),
        2 => read_varint(buf, pos).map(|(l, p)| (p + l as usize).min(buf.len())),
        5 => Some((pos + 4).min(buf.len())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_synthetic_trace_packets() {
        let pkt: Vec<u8> = vec![0x40, 0xC0, 0x84, 0x3D, 0x0A, 0x00];
        let mut trace = vec![0x0A, pkt.len() as u8];
        trace.extend_from_slice(&pkt);
        let summary = summarize_trace_bytes(&trace, 100);
        assert_eq!(summary["packetCount"], json!(1));
        assert_eq!(summary["packetTypes"]["ftrace_events"], json!(1));
    }
}
