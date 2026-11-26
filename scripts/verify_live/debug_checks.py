"""Live verification checks for in-container host-exec, strata-debug, and char devices."""

from __future__ import annotations

import json

from verify_live.runner import DeviceRunner


def _run_debug_json(dev: DeviceRunner, cmd: str) -> dict:
    res = dev.container_cmd(cmd, check=False)
    assert res.returncode == 0, f"'{cmd}' failed (rc={res.returncode}): {res.stderr or res.stdout}"
    try:
        payload = json.loads(res.stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(f"'{cmd}' returned invalid JSON: {exc}\noutput: {res.stdout}") from exc
    assert isinstance(payload, dict), f"'{cmd}' expected JSON object, got: {type(payload).__name__}"
    assert payload.get("ok") is True, f"'{cmd}' reported ok!=true: {payload}"
    return payload


def verify_debug_tools(dev: DeviceRunner) -> None:
    rel = dev.su(
        "/data/adb/ksu/bin/strata exec host-exec getprop ro.build.version.release"
    ).stdout.strip()
    assert rel, "host-exec getprop returned empty"

    ver_out = dev.container_cmd("strata-debug --version", check=False)
    assert ver_out.returncode == 0 and "strata-debug" in ver_out.stdout, (
        f"strata-debug --version failed: {ver_out.stderr or ver_out.stdout}"
    )

    probe = _run_debug_json(dev, "strata-debug health --no-sf --no-sideveil")
    assert "package" in probe, f"strata-debug health missing package field: {probe}"

    legacy_adb = dev.container_cmd("test ! -e /usr/local/bin/adb && echo clean", check=False)
    assert legacy_adb.stdout.strip() == "clean", "legacy /usr/local/bin/adb still exists in container"

    _run_debug_json(dev, "strata-debug cdp list-targets")

    cursor_path = "/tmp/strata-verify-live.cursor"
    try:
        _run_debug_json(dev, f"strata-debug logcat --mark --cursor {cursor_path}")
        _run_debug_json(dev, f"strata-debug logcat --cursor {cursor_path}")
    finally:
        dev.container_cmd(f"rm -f {cursor_path}", check=False)

    _run_debug_json(dev, "strata-debug system pstore")

    visual_path = "/tmp/strata-verify-visual.png"
    try:
        visual = _run_debug_json(dev, f"strata-debug visual --format png --out {visual_path}")
        health = visual.get("health") if isinstance(visual.get("health"), dict) else {}
        width = visual.get("width") if isinstance(visual.get("width"), int) else health.get("width")
        assert isinstance(width, int) and width > 0, f"expected positive visual width: {visual}"
    finally:
        dev.container_cmd(f"rm -f {visual_path}", check=False)

    _run_debug_json(dev, "strata-debug observe --settle-ms 100 --no-cdp")

    cfg_res = dev.strata_json("config show")
    if isinstance(cfg_res, dict):
        extra_devs = cfg_res.get("config", {}).get("extraCharDevs", [])
        if isinstance(extra_devs, list) and "uinput" in extra_devs:
            uinput_check = dev.container_cmd("test -c /dev/uinput && echo ok", check=False)
            assert uinput_check.stdout.strip() == "ok", (
                "uinput is configured in extraCharDevs but /dev/uinput is missing inside container"
            )
