"""UI Automator XML hierarchy dumper, parser, and node tapper."""

from __future__ import annotations

import os
import re
import subprocess
import time
import xml.etree.ElementTree as ET
from typing import Any

import gesture_replay

HOST_EXEC = "/usr/local/bin/host-exec"
BOUNDS_RE = re.compile(r"\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]")


def _run_host(args: list[str], timeout: float = 12.0) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [HOST_EXEC, *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=timeout,
        check=False,
    )


def _parse_bounds(raw: str) -> tuple[list[int], list[int]] | tuple[None, None]:
    m = BOUNDS_RE.search(raw)
    if not m:
        return None, None
    x0, y0, x1, y1 = (int(g) for g in m.groups())
    return [x0, y0, x1, y1], [(x0 + x1) // 2, (y0 + y1) // 2]


def parse_uiautomator_xml(
    xml_text: str,
    *,
    query: str | None = None,
    resource_id: str | None = None,
    content_desc: str | None = None,
    clickable_only: bool = False,
    limit: int = 80,
) -> dict[str, Any]:
    """Parses uiautomator XML into structured nodes with bounds, centers, and filters."""
    try:
        root = ET.fromstring(xml_text)
    except ET.ParseError as exc:
        return {"ok": False, "error": f"failed to parse uiautomator XML: {exc}"}

    q_low = query.lower() if query else None
    rid_low = resource_id.lower() if resource_id else None
    desc_low = content_desc.lower() if content_desc else None

    total_nodes = 0
    matched_count = 0
    nodes: list[dict[str, Any]] = []

    for elem in root.iter("node"):
        total_nodes += 1
        attrs = elem.attrib
        text = attrs.get("text", "")
        rid = attrs.get("resource-id", "")
        desc = attrs.get("content-desc", "")
        clickable = attrs.get("clickable", "false") == "true"

        if clickable_only and not clickable:
            continue
        if q_low is not None and q_low not in text.lower() and q_low not in desc.lower():
            continue
        if rid_low is not None and rid_low not in rid.lower():
            continue
        if desc_low is not None and desc_low not in desc.lower():
            continue

        matched_count += 1
        if limit > 0 and len(nodes) >= limit:
            continue

        bounds, center = _parse_bounds(attrs.get("bounds", ""))
        nodes.append(
            {
                "text": text,
                "resourceId": rid,
                "contentDesc": desc,
                "class": attrs.get("class", ""),
                "package": attrs.get("package", ""),
                "clickable": clickable,
                "enabled": attrs.get("enabled", "true") == "true",
                "focused": attrs.get("focused", "false") == "true",
                "selected": attrs.get("selected", "false") == "true",
                "checked": attrs.get("checked", "false") == "true",
                "bounds": bounds,
                "center": center,
            }
        )

    return {
        "ok": True,
        "nodeCount": total_nodes,
        "matchedCount": matched_count,
        "truncated": matched_count > len(nodes),
        "nodes": nodes,
    }


def dump_ui_tree(
    *,
    compressed: bool = False,
    query: str | None = None,
    resource_id: str | None = None,
    content_desc: str | None = None,
    clickable_only: bool = False,
    limit: int = 80,
) -> dict[str, Any]:
    """Dumps the current Android UI hierarchy via uiautomator and returns parsed nodes."""
    tmp_xml = f"/data/local/tmp/strata_ui_{os.getpid()}.xml"
    dump_cmd = ["/system/bin/uiautomator", "dump"]
    if compressed:
        dump_cmd.append("--compressed")
    dump_cmd.append(tmp_xml)

    try:
        last_err = ""
        dumped = False
        for attempt in range(2):
            res = _run_host(dump_cmd, timeout=15.0)
            combined = f"{res.stdout}\n{res.stderr}".strip()
            if res.returncode == 0 and "ERROR:" not in combined:
                dumped = True
                break
            last_err = combined or f"uiautomator exited with rc={res.returncode}"
            if attempt == 0:
                time.sleep(0.25)

        if not dumped:
            return {
                "ok": False,
                "error": last_err[:300],
                "hint": "Ensure the screen is unlocked and idle; transient animations can block uiautomator.",
            }

        read_res = _run_host(["/system/bin/cat", tmp_xml], timeout=10.0)
        if read_res.returncode != 0 or not read_res.stdout.strip():
            return {
                "ok": False,
                "error": read_res.stderr.strip()[:300] or "empty uiautomator XML output",
                "hint": "Verify /data/local/tmp is writable on the Android host.",
            }
        return parse_uiautomator_xml(
            read_res.stdout,
            query=query,
            resource_id=resource_id,
            content_desc=content_desc,
            clickable_only=clickable_only,
            limit=limit,
        )
    finally:
        _run_host(["/system/bin/rm", "-f", tmp_xml], timeout=5.0)


def tap_ui_node(
    *,
    text: str | None = None,
    resource_id: str | None = None,
    content_desc: str | None = None,
    index: int = 0,
    method: str = "input",
) -> dict[str, Any]:
    """Locates a UI node by selector and taps its center using input or uinput."""
    if not any((text, resource_id, content_desc)):
        return {
            "ok": False,
            "error": "at least one selector (text, resource_id, or content_desc) is required",
        }
    if method not in ("input", "uinput"):
        return {"ok": False, "error": f"unsupported tap method: {method}"}
    if index < 0:
        return {"ok": False, "error": "index must be non-negative"}

    tree = dump_ui_tree(
        query=text,
        resource_id=resource_id,
        content_desc=content_desc,
        limit=max(80, index + 1),
    )
    if not tree.get("ok"):
        return tree

    nodes = tree.get("nodes", [])
    if index >= len(nodes):
        return {
            "ok": False,
            "error": f"node index {index} out of range (matched {tree.get('matchedCount', 0)})",
            "matchedCount": tree.get("matchedCount", 0),
        }

    node = nodes[index]
    center = node.get("center")
    if not center or len(center) != 2:
        return {"ok": False, "error": "selected node has no valid bounds", "node": node}

    cx, cy = int(center[0]), int(center[1])
    if method == "input":
        tap_res = _run_host(["/system/bin/input", "tap", str(cx), str(cy)], timeout=8.0)
        if tap_res.returncode != 0:
            return {
                "ok": False,
                "method": method,
                "center": [cx, cy],
                "error": tap_res.stderr.strip()[:240] or f"input tap exited rc={tap_res.returncode}",
            }
    else:
        geom = gesture_replay.get_display_geometry()
        with gesture_replay.UInputTouchDevice(
            geom["phys_w"],
            geom["phys_h"],
            logical_width=geom["logical_w"],
            logical_height=geom["logical_h"],
            rotation=geom["rotation"],
        ) as dev:
            dev.tap(cx, cy)

    return {"ok": True, "method": method, "center": [cx, cy], "node": node}
