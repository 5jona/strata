"""Optional SideVeil plugin bridge, HUD, and WebView live checks (steps 11-12 + UI)."""

import json
import subprocess
import time

from verify_live.runner import DeviceRunner

SV_CTL = "/data/adb/modules/sideveil/bin/sideveil-ctl"


def verify_sideveil_backend(dev: DeviceRunner) -> None:
    has_sideveil = (
        dev.su(
            f"test -x {SV_CTL} && test -S /data/system/sideveil/run/hub.sock && echo yes || echo no"
        )
        .stdout.strip()
        == "yes"
    )
    if not has_sideveil:
        print("[11-12/12] SideVeil not running on device; confirmed 100% standalone Strata operation.")
        return

    print("[11/12] Verifying SideVeil plugin sync, online backendState, and WebView bundle...")
    dev.su(
        "test -f /data/system/sideveil/plugins/strata.manager/manifest.json && "
        "test -f /data/system/sideveil/plugins/strata.manager/web/index.html && "
        "test -f /data/system/sideveil/plugins/strata.manager/web/icon.svg"
    )
    sv_plugins = json.loads(dev.su(f"{SV_CTL} plugins list").stdout)
    strata_entry = next(
        (p for p in sv_plugins["result"]["plugins"] if p["id"] == "strata.manager"),
        None,
    )
    assert strata_entry is not None, f"strata.manager missing from SideVeil registry: {sv_plugins}"
    assert strata_entry["backendState"] == "online", f"expected online backend: {strata_entry}"
    dstatus_after = dev.strata_json("daemon status")
    assert dstatus_after["sideveilConnected"] is True, (
        f"expected sideveilConnected=true: {dstatus_after}"
    )

    sv_ping = json.loads(
        dev.su(f"{SV_CTL} plugins invoke strata.manager daemon.ping '{{}}' --timeout-ms 10000").stdout
    )
    assert sv_ping.get("result", {}).get("pong") is True, (
        f"sideveil-ctl plugins invoke strata.manager daemon.ping failed: {sv_ping}"
    )
    sv_list = json.loads(
        dev.su(
            f"{SV_CTL} plugins invoke strata.manager container.list '{{}}' --timeout-ms 10000"
        ).stdout
    )
    assert any(
        item.get("id") == "main" and item.get("phase") == "running"
        for item in sv_list.get("result", [])
    ), f"sideveil-ctl plugins invoke container.list unexpected: {sv_list}"

    print("[12/12] Verifying task.logs replay, Native HUD toggle, and SideVeil asset serving...")
    tasks_json = dev.strata_json("task list")
    assert len(tasks_json) >= 1, f"expected recorded task history: {tasks_json}"
    latest_tid = tasks_json[-1]["taskId"]
    tlogs = dev.strata_json(f"task logs {latest_tid}")
    assert tlogs["summary"]["exitCode"] == 0, f"unexpected task summary: {tlogs}"
    assert any("strata-stream-ok" in e["chunk"] for e in tlogs["entries"]), (
        f"expected buffered task output in task.logs: {tlogs}"
    )

    hud_on = dev.strata_json(
        "rpc hud.set '{\"enabled\":true,\"clickThrough\":false,\"isDark\":true}'"
    )
    assert hud_on["enabled"] is True and hud_on["sideveilConnected"] is True, (
        f"unexpected hud_on response: {hud_on}"
    )
    hud_off = dev.strata_json("rpc hud.set '{\"enabled\":false}'")
    assert hud_off["enabled"] is False, f"unexpected hud_off response: {hud_off}"

    dev.su(
        "grep -q 'pochita:color-scheme' /data/system/sideveil/plugins/strata.manager/web/index.html"
    )
    dev.su(f"{SV_CTL} plugins reload")
    sv_after = json.loads(dev.su(f"{SV_CTL} plugins list").stdout)
    strata_after = next(
        (p for p in sv_after["result"]["plugins"] if p["id"] == "strata.manager"),
        None,
    )
    assert strata_after is not None and strata_after["backendState"] == "online", (
        f"strata.manager did not remain online after reload: {strata_after}"
    )


def _wait_panel_open(dev: DeviceRunner, timeout_s: float = 4.0) -> dict:
    deadline = time.monotonic() + timeout_s
    last: dict = {}
    while time.monotonic() < deadline:
        last = json.loads(dev.su(f"{SV_CTL} status").stdout)
        if last.get("panelState") == "open" and last.get("activePluginId") == "strata.manager":
            return last
        time.sleep(0.15)
    return last


def verify_sideveil_ui(dev: DeviceRunner, screenshot_out: str | None) -> None:
    print("[UI] Selecting strata.manager and opening SideVeil overlay panel...")
    sel = json.loads(
        dev.su(f"{SV_CTL} rpc plugins.select '{{\"pluginId\":\"strata.manager\"}}'").stdout
    )
    assert sel.get("result", {}).get("pluginId") == "strata.manager", f"select failed: {sel}"
    dev.su(f"{SV_CTL} open")
    try:
        st = _wait_panel_open(dev)
        assert st.get("activePluginId") == "strata.manager", f"unexpected active plugin: {st}"
        assert st.get("panelState") == "open", f"panel did not open: {st}"

        inspect = dev.su(f"{SV_CTL} debug inspect-ui", check=False)
        if inspect.returncode == 0 and inspect.stdout.strip():
            print(f"[UI] inspect-ui: {inspect.stdout.strip()[:240]}")

        probe = dev.container_cmd("strata-debug sys-probe --package pochita.sideveil.host", check=False)
        if probe.returncode == 0 and probe.stdout.strip():
            parsed = json.loads(probe.stdout)
            print(f"[UI] sys-probe windows={len(parsed.get('packageWindows', []))}")

        if screenshot_out:
            remote_img = "/data/local/tmp/strata_sideveil_ui.webp"
            vis = dev.container_cmd(f"strata-debug visual --out {remote_img}", check=False)
            if vis.returncode == 0:
                dev.pull_host_file(remote_img, screenshot_out)
                dev.su(f"/system/bin/rm -f {remote_img}", check=False)
            else:
                remote_png = "/data/local/tmp/strata_sideveil_ui.png"
                dev.su(f"/system/bin/screencap -p {remote_png}")
                dev.pull_host_file(remote_png, screenshot_out)
                dev.su(f"/system/bin/rm -f {remote_png}", check=False)

        logs = json.loads(dev.su(f"{SV_CTL} logs").stdout)
        lines = logs.get("result", {}).get("lines", [])
        errors = [
            ln
            for ln in lines[-20:]
            if "strata.manager" in ln and ("[E]" in ln or "[W]" in ln or "error" in ln.lower())
        ]
        assert not errors, f"SideVeil logged errors for strata.manager: {errors}"
        print("[UI] SideVeil WebView mounted strata.manager cleanly with zero errors.")
    finally:
        dev.su(f"{SV_CTL} close", check=False)
