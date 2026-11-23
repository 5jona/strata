"""Pure-Python Perfetto trace analyzer with optional trace_processor_shell support."""

from __future__ import annotations

import io
import json
import os
import posixpath
import shutil
import subprocess
from typing import Any, BinaryIO

from host_runner import HOST_EXEC, run_host_bytes

PACKET_FIELD_NAMES: dict[int, str] = {
    1: "ftrace_events", 2: "process_tree", 3: "sys_stats", 6: "clock_snapshot",
    11: "track_event", 35: "trace_stats", 36: "trace_config", 39: "android_log",
    53: "surfaceflinger_layers", 54: "surfaceflinger_transactions",
    60: "track_descriptor", 74: "frame_timeline_event",
}

FTRACE_EVENT_NAMES: dict[int, str] = {
    3: "print_atrace", 4: "sched_switch", 16: "print", 18: "sched_wakeup",
    67: "binder_transaction", 68: "binder_transaction_received",
    69: "binder_lock", 70: "binder_locked", 71: "binder_unlock",
    72: "cpu_frequency", 73: "cpu_idle", 170: "sched_waking", 235: "sched_process_free",
    338: "rss_stat",
}


def _read_varint_bytes(buf: bytes, pos: int, end: int) -> tuple[int, int]:
    val, shift = 0, 0
    while pos < end and shift < 70:
        b = buf[pos]
        pos += 1
        val |= (b & 0x7F) << shift
        if not (b & 0x80):
            return val, pos
        shift += 7
    raise ValueError("truncated varint")


def _read_varint_stream(stream: BinaryIO) -> int | None:
    val, shift = 0, 0
    while shift < 70:
        chunk = stream.read(1)
        if not chunk:
            if shift == 0:
                return None
            raise ValueError("truncated stream varint")
        b = chunk[0]
        val |= (b & 0x7F) << shift
        if not (b & 0x80):
            return val
        shift += 7
    raise ValueError("overlong stream varint")


def _skip_wire_bytes(buf: bytes, pos: int, end: int, wire_type: int) -> int:
    if wire_type == 0:
        return _read_varint_bytes(buf, pos, end)[1]
    if wire_type == 1:
        return min(end, pos + 8)
    if wire_type == 2:
        length, pos = _read_varint_bytes(buf, pos, end)
        return min(end, pos + length)
    return min(end, pos + 4) if wire_type == 5 else end


def _skip_wire_stream(stream: BinaryIO, wire_type: int) -> bool:
    if wire_type == 0:
        return _read_varint_stream(stream) is not None
    if wire_type == 1:
        return len(stream.read(8)) == 8
    if wire_type == 2:
        length = _read_varint_stream(stream)
        if length is None:
            return False
        stream.seek(length, io.SEEK_CUR) if stream.seekable() else stream.read(length)
        return True
    return len(stream.read(4)) == 4 if wire_type == 5 else False


def _record_ts(ts: int, ts_bounds: list[int | None]) -> None:
    if ts <= 0:
        return
    if ts_bounds[0] is None or ts < ts_bounds[0]:
        ts_bounds[0] = ts
    if ts_bounds[1] is None or ts > ts_bounds[1]:
        ts_bounds[1] = ts


def _parse_ftrace_event(
    buf: bytes, pos: int, end: int, ftrace_counts: dict[str, int], ts_bounds: list[int | None]
) -> None:
    while pos < end:
        try:
            tag, pos = _read_varint_bytes(buf, pos, end)
            field_no, wire_type = tag >> 3, tag & 0x07
            if field_no == 1 and wire_type == 0:
                ts, pos = _read_varint_bytes(buf, pos, end)
                _record_ts(ts, ts_bounds)
            elif field_no >= 3 and wire_type == 2:
                name = FTRACE_EVENT_NAMES.get(field_no, f"ftrace_tag_{field_no}")
                ftrace_counts[name] = ftrace_counts.get(name, 0) + 1
                pos = _skip_wire_bytes(buf, pos, end, wire_type)
            else:
                pos = _skip_wire_bytes(buf, pos, end, wire_type)
        except ValueError:
            break


def _parse_ftrace_bundle(
    buf: bytes, ftrace_counts: dict[str, int], ts_bounds: list[int | None]
) -> None:
    pos, end = 0, len(buf)
    while pos < end:
        try:
            tag, pos = _read_varint_bytes(buf, pos, end)
            field_no, wire_type = tag >> 3, tag & 0x07
            if field_no == 2 and wire_type == 2:
                ev_len, pos = _read_varint_bytes(buf, pos, end)
                ev_end = min(end, pos + ev_len)
                _parse_ftrace_event(buf, pos, ev_end, ftrace_counts, ts_bounds)
                pos = ev_end
            else:
                pos = _skip_wire_bytes(buf, pos, end, wire_type)
        except ValueError:
            break


def _parse_trace_packet(
    buf: bytes,
    packet_types: dict[str, int],
    ftrace_counts: dict[str, int],
    seq_ids: set[int],
    ts_bounds: list[int | None],
) -> None:
    pos, end = 0, len(buf)
    while pos < end:
        try:
            tag, pos = _read_varint_bytes(buf, pos, end)
            field_no, wire_type = tag >> 3, tag & 0x07
            if field_no == 8 and wire_type == 0:
                ts, pos = _read_varint_bytes(buf, pos, end)
                _record_ts(ts, ts_bounds)
            elif field_no == 10 and wire_type == 0:
                seq_id, pos = _read_varint_bytes(buf, pos, end)
                seq_ids.add(seq_id)
            elif wire_type == 2 and field_no in PACKET_FIELD_NAMES:
                length, pos = _read_varint_bytes(buf, pos, end)
                sub_end = min(end, pos + length)
                name = PACKET_FIELD_NAMES[field_no]
                packet_types[name] = packet_types.get(name, 0) + 1
                if field_no == 1:
                    _parse_ftrace_bundle(buf[pos:sub_end], ftrace_counts, ts_bounds)
                pos = sub_end
            else:
                pos = _skip_wire_bytes(buf, pos, end, wire_type)
        except ValueError:
            break


def analyze_trace_stream(stream: BinaryIO, *, max_packets: int = 50000) -> dict[str, Any]:
    """Streaming pure-Python protobuf wire-format analyzer for Perfetto traces."""
    packet_count, truncated = 0, False
    packet_types: dict[str, int] = {}
    ftrace_counts: dict[str, int] = {}
    seq_ids: set[int] = set()
    ts_bounds: list[int | None] = [None, None]

    while True:
        try:
            tag = _read_varint_stream(stream)
            if tag is None:
                break
            field_no, wire_type = tag >> 3, tag & 0x07
            if field_no == 1 and wire_type == 2:
                pkt_len = _read_varint_stream(stream)
                if pkt_len is None:
                    break
                if packet_count >= max_packets:
                    truncated = True
                    break
                pkt_bytes = stream.read(pkt_len)
                if not pkt_bytes:
                    break
                packet_count += 1
                _parse_trace_packet(pkt_bytes, packet_types, ftrace_counts, seq_ids, ts_bounds)
            elif not _skip_wire_stream(stream, wire_type):
                break
        except ValueError:
            break

    dur_ms = 0.0
    if ts_bounds[0] is not None and ts_bounds[1] is not None and ts_bounds[1] >= ts_bounds[0]:
        dur_ms = round((ts_bounds[1] - ts_bounds[0]) / 1_000_000.0, 2)

    highlights: list[str] = []
    if dur_ms > 0:
        rate = round(packet_count / (dur_ms / 1000.0), 1)
        highlights.append(f"{packet_count} packets over {dur_ms} ms ({rate} pkt/s, {len(seq_ids)} sequences)")
    else:
        highlights.append(f"{packet_count} packets parsed ({len(seq_ids)} sequences)")
    if packet_types:
        top_pkts = sorted(packet_types.items(), key=lambda kv: kv[1], reverse=True)[:4]
        highlights.append("top packets: " + ", ".join(f"{k}={v}" for k, v in top_pkts))
    if ftrace_counts:
        top_ft = sorted(ftrace_counts.items(), key=lambda kv: kv[1], reverse=True)[:5]
        highlights.append("top ftrace: " + ", ".join(f"{k}={v}" for k, v in top_ft))
    if truncated:
        highlights.append(f"truncated at max_packets={max_packets}")

    return {
        "packetCount": packet_count,
        "traceDurationMs": dur_ms,
        "packetTypes": packet_types,
        "ftraceEventCounts": ftrace_counts,
        "highlights": highlights,
    }


def _try_trace_processor(trace_path: str) -> dict[str, Any] | None:
    tp_bin = shutil.which("trace_processor_shell")
    if not tp_bin or not os.path.isfile(trace_path):
        return None
    sql = (
        "SELECT (SELECT (end_ts - start_ts)/1e6 FROM trace_bounds) AS dur_ms, "
        "(SELECT COUNT(*) FROM slice) AS slice_cnt, (SELECT COUNT(*) FROM sched) AS sched_cnt;"
    )
    try:
        res = subprocess.run(
            [tp_bin, "-Q", sql, "--output-format=json", trace_path],
            capture_output=True, text=True, timeout=15.0, check=False,
        )
        if res.returncode == 0 and res.stdout.strip():
            return {"sqlSummary": json.loads(res.stdout)}
    except (OSError, subprocess.TimeoutExpired, ValueError):
        pass
    return None


def analyze_trace_file(trace_path: str, max_packets: int = 50000) -> dict[str, Any]:
    """Analyzes a `.perfetto-trace` file and returns a structured summary."""
    if not isinstance(trace_path, str) or not trace_path or ".." in trace_path:
        return {"ok": False, "error": "invalid trace_path"}
    norm = posixpath.normpath(trace_path)
    max_pkts = max(1, min(500000, int(max_packets)))

    if os.path.isfile(norm) and os.access(norm, os.R_OK):
        size_bytes = os.path.getsize(norm)
        with open(norm, "rb") as f:
            stats = analyze_trace_stream(f, max_packets=max_pkts)
        tp_extra = _try_trace_processor(norm)
        analyzer = "trace_processor_shell+protobuf" if tp_extra else "pure-python-protobuf"
        out: dict[str, Any] = {"ok": True, "path": norm, "sizeBytes": size_bytes, "analyzer": analyzer, **stats}
        if tp_extra:
            out.update(tp_extra)
        return out

    if norm.startswith("/data/local/tmp/") and os.path.exists(HOST_EXEC):
        try:
            proc = run_host_bytes(["/system/bin/cat", norm], timeout=20.0)
            if proc.returncode == 0 and proc.stdout:
                stats = analyze_trace_stream(io.BytesIO(proc.stdout), max_packets=max_pkts)
                return {
                    "ok": True, "path": norm, "sizeBytes": len(proc.stdout),
                    "analyzer": "pure-python-protobuf", **stats,
                }
        except (OSError, subprocess.TimeoutExpired) as exc:
            return {"ok": False, "path": norm, "error": str(exc)}

    return {"ok": False, "path": norm, "error": f"trace file not found or unreadable: {norm}"}
