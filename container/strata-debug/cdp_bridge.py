"""L1 WebView CDP discovery, proxy, evaluation, console capture, and DOM auditor."""

from __future__ import annotations

import re
from typing import Any
from urllib.parse import urlparse

from cdp_audit_js import DOM_AUDIT_JS
from cdp_http import http_get_unix, serve_cdp_proxy
from cdp_ws import CdpWsClient

DEVTOOLS_RE = re.compile(r"@((?:webview|chrome)_devtools_remote(?:_(\d+))?)")

__all__ = [
    "CdpWsClient",
    "audit_webview",
    "collect_console",
    "eval_js",
    "http_get_unix",
    "list_targets",
    "match_devtools_socket",
    "scan_devtools_sockets",
    "serve_cdp_proxy",
]


def _read_cmdline(pid: int) -> str:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\x00", b" ").decode("utf-8", errors="replace").strip()
    except OSError:
        return ""


def scan_devtools_sockets(proc_unix_path: str = "/proc/net/unix") -> list[dict[str, Any]]:
    seen: dict[str, dict[str, Any]] = {}
    try:
        with open(proc_unix_path, encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
    except OSError:
        return []
    for line in lines[1:]:
        m = DEVTOOLS_RE.search(line)
        if not m:
            continue
        sock_name = m.group(1)
        pid = int(m.group(2)) if m.group(2) else None
        if sock_name not in seen:
            seen[sock_name] = {
                "socket": sock_name,
                "pid": pid,
                "process": _read_cmdline(pid) if pid else "",
            }
    return list(seen.values())


def match_devtools_socket(
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    proc_unix_path: str = "/proc/net/unix",
) -> dict[str, Any] | None:
    sockets = scan_devtools_sockets(proc_unix_path)
    if pid is not None:
        for item in sockets:
            if item["pid"] == pid:
                return item
        return {"socket": f"webview_devtools_remote_{pid}", "pid": pid, "process": _read_cmdline(pid)}
    if target:
        for item in sockets:
            if target in item["process"]:
                return item
    return sockets[0] if sockets else None


def list_targets(
    *,
    target: str | None = None,
    pid: int | None = None,
    proc_unix_path: str = "/proc/net/unix",
    timeout: float = 2.5,
) -> dict[str, Any]:
    """Lists discovered DevTools abstract sockets and queries /json/list targets from each."""
    sockets = scan_devtools_sockets(proc_unix_path)
    filtered = sockets
    if pid is not None:
        filtered = [s for s in sockets if s.get("pid") == pid]
    elif target:
        filtered = [s for s in sockets if target in s.get("process", "")]

    targets: list[dict[str, Any]] = []
    for sock_info in filtered:
        sock_name = sock_info["socket"]
        try:
            items = http_get_unix(sock_name, "/json/list", timeout=timeout)
            if isinstance(items, list):
                for item in items:
                    if isinstance(item, dict):
                        entry = dict(item)
                        entry.setdefault("socket", sock_name)
                        entry.setdefault("pid", sock_info.get("pid"))
                        entry.setdefault("process", sock_info.get("process", ""))
                        targets.append(entry)
        except Exception as exc:
            sock_info["probeError"] = str(exc)

    return {"ok": True, "sockets": sockets, "targets": targets}


def _select_page(
    *,
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    url_match: str | None = None,
    timeout: float = 4.0,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None, dict[str, Any] | None]:
    """Selects (matched_socket, page_target, error_payload)."""
    matched = match_devtools_socket(target=target, pid=pid)
    if not matched:
        return None, None, {"ok": False, "error": "no webview_devtools_remote socket found"}
    try:
        raw_targets = http_get_unix(matched["socket"], "/json/list", timeout=timeout)
    except Exception as exc:
        return matched, None, {"ok": False, "socket": matched["socket"], "error": f"CDP HTTP query failed: {exc}"}
    pages = [t for t in raw_targets if isinstance(t, dict) and t.get("webSocketDebuggerUrl")]
    if url_match:
        matched_pages = [t for t in pages if url_match in t.get("url", "")]
        if not matched_pages:
            return matched, None, {
                "ok": False,
                "socket": matched["socket"],
                "error": f"no WebView target matching url '{url_match}'",
            }
        pages = matched_pages
    if not pages:
        return matched, None, {"ok": False, "socket": matched["socket"], "error": "no inspectable WebView pages"}
    return matched, pages[0], None


def _ws_path_from_page(page: dict[str, Any]) -> str:
    parsed = urlparse(page["webSocketDebuggerUrl"])
    return f"{parsed.path}?{parsed.query}" if parsed.query else parsed.path


def eval_js(
    expression: str,
    *,
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    url_match: str | None = None,
    timeout: float = 5.0,
) -> dict[str, Any]:
    """Evaluates a JavaScript expression in the selected WebView target via CDP Runtime.evaluate."""
    matched, page, err = _select_page(target=target, pid=pid, url_match=url_match, timeout=timeout)
    if err or not matched or not page:
        return err or {"ok": False, "error": "target selection failed"}
    try:
        client = CdpWsClient.connect_unix(matched["socket"], _ws_path_from_page(page), timeout=timeout)
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "error": f"CDP WebSocket connect failed: {exc}"}
    try:
        client.call("Runtime.enable", timeout=timeout)
        res = client.call(
            "Runtime.evaluate",
            {"expression": expression, "returnByValue": True, "awaitPromise": True},
            timeout=timeout,
        )
        result_obj = res.get("result", {})
        exc_details = result_obj.get("exceptionDetails")
        return {
            "ok": exc_details is None,
            "socket": matched["socket"],
            "pid": matched["pid"],
            "pageUrl": page.get("url"),
            "value": result_obj.get("result", {}).get("value"),
            "exception": exc_details,
        }
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "pageUrl": page.get("url"), "error": str(exc)}
    finally:
        client.close()


def collect_console(
    *,
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    url_match: str | None = None,
    duration_s: float = 2.0,
    timeout: float = 5.0,
) -> dict[str, Any]:
    """Captures console events and uncaught exceptions from a WebView target for duration_s seconds."""
    matched, page, err = _select_page(target=target, pid=pid, url_match=url_match, timeout=timeout)
    if err or not matched or not page:
        return err or {"ok": False, "error": "target selection failed"}
    try:
        client = CdpWsClient.connect_unix(matched["socket"], _ws_path_from_page(page), timeout=timeout)
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "error": f"CDP WebSocket connect failed: {exc}"}
    try:
        client.call("Runtime.enable", timeout=timeout)
        client.drain_events(duration_s=duration_s)
        return {
            "ok": True,
            "socket": matched["socket"],
            "pid": matched["pid"],
            "pageUrl": page.get("url"),
            "events": client.console_events[:50],
            "exceptions": client.exceptions[:20],
        }
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "pageUrl": page.get("url"), "error": str(exc)}
    finally:
        client.close()


def audit_webview(
    target: str | None = "pochita.sideveil.host",
    pid: int | None = None,
    url_contains: str | None = None,
    eval_js_expr: str | None = None,
    *,
    eval_js: str | None = None,
    timeout: float = 5.0,
) -> dict[str, Any]:
    expr = eval_js if eval_js is not None else eval_js_expr
    matched, page, err = _select_page(target=target, pid=pid, url_match=url_contains, timeout=timeout)
    if err or not matched or not page:
        return err or {"ok": False, "error": "target selection failed"}
    try:
        client = CdpWsClient.connect_unix(matched["socket"], _ws_path_from_page(page), timeout=timeout)
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "error": f"CDP WebSocket connect failed: {exc}"}
    try:
        client.call("Runtime.enable", timeout=timeout)
        res = client.call(
            "Runtime.evaluate",
            {"expression": DOM_AUDIT_JS, "returnByValue": True, "awaitPromise": True},
            timeout=timeout,
        )
        audit = res.get("result", {}).get("result", {}).get("value", {})
        eval_out = None
        if expr:
            ev = client.call(
                "Runtime.evaluate",
                {"expression": expr, "returnByValue": True, "awaitPromise": True},
                timeout=timeout,
            )
            eval_out = ev.get("result", {}).get("result", {}).get("value")
        return {
            "ok": True,
            "socket": matched["socket"],
            "pid": matched["pid"],
            "pageUrl": page.get("url"),
            "audit": audit,
            "evalResult": eval_out,
            "consoleErrors": [e for e in client.console_events if e.get("type") in {"error", "warning"}][:10],
            "exceptions": client.exceptions[:10],
        }
    except Exception as exc:
        return {"ok": False, "socket": matched["socket"], "pageUrl": page.get("url"), "error": str(exc)}
    finally:
        client.close()
