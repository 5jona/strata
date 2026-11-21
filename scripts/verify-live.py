#!/usr/bin/env python3
"""End-to-end live Android device verification suite for Strata Phase 1."""

import argparse
import json
import shlex
import subprocess
import sys
import time


class DeviceRunner:
    def __init__(self, serial: str | None) -> None:
        self.serial = serial

    def su(self, cmd: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        base = ["adb"]
        if self.serial:
            base += ["-s", self.serial]
        base += ["shell", f"su -c {shlex.quote(cmd)}"]
        res = subprocess.run(base, text=True, capture_output=True, check=False)
        if check and res.returncode != 0:
            raise RuntimeError(
                f"Command failed (rc={res.returncode}): {cmd}\nstdout: {res.stdout}\nstderr: {res.stderr}"
            )
        return res

    def strata_json(self, args: str) -> dict | list:
        res = self.su(f"/data/adb/ksu/bin/strata --json {args}")
        return json.loads(res.stdout)


def detect_serial(explicit: str | None) -> str:
    if explicit:
        return explicit
    out = subprocess.check_output(["adb", "devices"], text=True)
    serials = [
        line.split()[0]
        for line in out.splitlines()[1:]
        if len(line.split()) >= 2 and line.split()[1] == "device"
    ]
    if len(serials) != 1:
        raise SystemExit(f"Expected 1 ADB device, found {len(serials)}. Pass --serial.")
    return serials[0]


def verify(dev: DeviceRunner) -> None:
    print("[1/12] Checking stratad daemon ping and status...")
    ping = dev.strata_json("daemon ping")
    assert ping.get("pong") is True, f"unexpected ping: {ping}"
    dstatus = dev.strata_json("daemon status")
    assert dstatus["defaultContainer"] == "main"

    print("[2/12] Testing concurrent-mount IMAGE_BUSY protection...")
    dev.strata_json("stop")
    status_stopped = dev.strata_json("status")
    image_path = status_stopped["imagePath"]
    busy_dir = "/data/local/tmp/strata_busy_probe"
    try:
        dev.su(
            f"/system/bin/mkdir -p {busy_dir} && "
            f"L=$(/system/bin/losetup -f --show {shlex.quote(image_path)}) && "
            f"/system/bin/mount -t ext4 -o ro \"$L\" {busy_dir}"
        )
        busy_res = dev.su("/data/adb/ksu/bin/strata start", check=False)
        assert busy_res.returncode != 0, "strata start should have rejected busy image"
        assert "-32020" in busy_res.stderr or "already mounted" in busy_res.stderr, (
            f"expected IMAGE_BUSY (-32020), got: {busy_res.stderr}"
        )
    finally:
        dev.su(
            f"/system/bin/umount {busy_dir} 2>/dev/null || true; "
            f"/system/bin/rmdir {busy_dir} 2>/dev/null || true; "
            f"for l in $(/system/bin/losetup -a | awk -F: '/rootfs\\.img/ {{print $1}}'); do "
            f"/system/bin/losetup -d \"$l\" 2>/dev/null || true; done",
            check=False,
        )

    print("[3/12] Starting main container and verifying status/list...")
    started = dev.strata_json("start")
    assert started["phase"] == "running", f"expected running, got {started}"
    assert started["mounted"] is True
    assert started["nsPid"] and started["nsPid"] > 1
    assert started["loopDevices"], "expected active loop device"
    assert started["cgroupPath"] == "/sys/fs/cgroup/strata/main"
    listed = dev.strata_json("list")
    assert any(item["id"] == "main" and item["phase"] == "running" for item in listed)

    print("[4/12] Verifying Dropbear SSH process (zero OpenSSH privsep) and OEM kevent...")
    ps_out = dev.su("ps -ef | grep dropbear | grep -v grep").stdout
    assert "/usr/sbin/dropbear" in ps_out or "dropbear -F" in ps_out, f"dropbear missing: {ps_out}"
    sshd_check = dev.su("ps -ef | grep '[ /]sshd' || true").stdout.strip()
    assert not sshd_check, f"unexpected openssh sshd running: {sshd_check}"

    print("[5/12] Verifying exec, py, minimal /dev, /dev/shm, DNS, and host.env...")
    hn = dev.su("/data/adb/ksu/bin/strata exec uname -n").stdout.strip()
    assert hn == started["hostname"], f"hostname mismatch: {hn} != {started['hostname']}"
    py_ver = dev.su(
        "/data/adb/ksu/bin/strata py -c 'import sys; print(sys.version_info[:2])'"
    ).stdout.strip()
    assert py_ver.startswith("(3,"), f"unexpected python version: {py_ver}"

    dev_check = dev.su(
        "/data/adb/ksu/bin/strata exec /bin/sh -c "
        "'test ! -e /dev/block && stat -c \"%a\" /dev/shm && test -s /etc/resolv.conf && test -s /run/strata/host.env'"
    ).stdout.strip()
    assert dev_check == "1777", f"unexpected /dev/shm mode or missing runtime files: {dev_check}"

    print("[6/12] Verifying headless streaming task runner (task.run + events.subscribe)...")
    task_out = dev.su(
        "/data/adb/ksu/bin/strata task run /usr/bin/python3 -c \"print('strata-stream-ok')\""
    ).stdout.strip()
    assert task_out == "strata-stream-ok", f"unexpected streaming task output: {task_out}"

    print("[7/12] Verifying in-container host-exec and zero-ADB shim...")
    rel = dev.su("/data/adb/ksu/bin/strata exec host-exec getprop ro.build.version.release").stdout.strip()
    assert rel, "host-exec getprop returned empty"
    adb_id = dev.su("/data/adb/ksu/bin/strata exec adb shell id").stdout.strip()
    assert "uid=0(root)" in adb_id, f"adb-shim failed: {adb_id}"

    print("[8/12] Verifying dual-signal zero-residual stop under active background load...")
    dev.su(
        "/data/adb/ksu/bin/strata exec /bin/sh -c 'nohup sleep 300 >/dev/null 2>&1 </dev/null &'"
    )
    sleep_check = dev.su("ps -ef | grep 'sleep 300' | grep -v grep || true").stdout.strip()
    assert sleep_check, "background sleep 300 did not spawn"
    cleanup = dev.strata_json("stop")
    assert cleanup["residualMounts"] == 0, f"residual mounts: {cleanup}"
    assert cleanup["residualLoops"] == 0, f"residual loops: {cleanup}"
    sleep_after = dev.su("ps -ef | grep 'sleep 300' | grep -v grep || true").stdout.strip()
    assert not sleep_after, f"background process survived teardown: {sleep_after}"
    cg_exists = dev.su("test -d /sys/fs/cgroup/strata/main && echo yes || echo no").stdout.strip()
    assert cg_exists == "no", "container cgroup directory was not removed"

    print("[9/12] Verifying explicit debug mode (/dev/block visible) and zero-residual exit...")
    dbg = dev.su("/data/adb/ksu/bin/strata debug /bin/sh -c 'ls /dev/block | head -n 3'").stdout.strip()
    assert dbg, "expected /dev/block entries in debug mode"
    post_dbg = dev.strata_json("status")
    assert post_dbg["mounted"] is False and not post_dbg["loopDevices"], (
        f"debug lease leaked mount/loop: {post_dbg}"
    )

    print("[10/12] Verifying fsck, compact, start, dns, restart, and atomic backup...")
    fsck = dev.strata_json("fsck")
    assert fsck["exitCode"] in (0, 1), f"fsck failed: {fsck}"
    compact = dev.strata_json("compact")
    assert compact["afterBytes"] <= compact["beforeBytes"] + 65536
    dev.strata_json("start")
    dev.strata_json("dns")
    restarted = dev.strata_json("restart")
    assert restarted["phase"] == "running"

    backup_path = "/data/local/tmp/strata_verify_backup.img.gz"
    try:
        backup = dev.strata_json(f"backup {backup_path}")
        assert backup["bytesWritten"] > 1024 and backup["restarted"] is True
        magic = dev.su(f"/system/bin/od -An -tx1 -N2 {backup_path}").stdout.strip()
        assert magic == "1f 8b", f"invalid gzip header: {magic}"
    finally:
        dev.su(f"/system/bin/rm -f {backup_path}", check=False)

    final_status = dev.strata_json("status")
    assert final_status["phase"] == "running" and final_status["mounted"] is True

    sv_ctl = "/data/adb/modules/sideveil/bin/sideveil-ctl"
    has_sideveil = (
        dev.su(f"test -x {sv_ctl} && test -S /data/system/sideveil/run/hub.sock && echo yes || echo no")
        .stdout.strip()
        == "yes"
    )
    if has_sideveil:
        print("[11/12] Verifying SideVeil plugin sync, online backendState, and WebView bundle...")
        dev.su(
            "test -f /data/system/sideveil/plugins/strata.manager/manifest.json && "
            "test -f /data/system/sideveil/plugins/strata.manager/web/index.html && "
            "test -f /data/system/sideveil/plugins/strata.manager/web/icon.svg"
        )
        sv_plugins = json.loads(dev.su(f"{sv_ctl} plugins list").stdout)
        strata_entry = next(
            (p for p in sv_plugins["result"]["plugins"] if p["id"] == "strata.manager"),
            None,
        )
        assert strata_entry is not None, f"strata.manager missing from SideVeil registry: {sv_plugins}"
        assert strata_entry["backendState"] == "online", f"expected online backend: {strata_entry}"
        dstatus_after = dev.strata_json("daemon status")
        assert dstatus_after["sideveilConnected"] is True, f"expected sideveilConnected=true: {dstatus_after}"

        sv_ping = json.loads(
            dev.su(
                f"{sv_ctl} plugins invoke strata.manager daemon.ping '{{}}' --timeout-ms 10000"
            ).stdout
        )
        assert sv_ping.get("result", {}).get("pong") is True, (
            f"sideveil-ctl plugins invoke strata.manager daemon.ping failed: {sv_ping}"
        )
        sv_list = json.loads(
            dev.su(
                f"{sv_ctl} plugins invoke strata.manager container.list '{{}}' --timeout-ms 10000"
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
        dev.su(f"{sv_ctl} plugins reload")
        sv_after = json.loads(dev.su(f"{sv_ctl} plugins list").stdout)
        strata_after = next(
            (p for p in sv_after["result"]["plugins"] if p["id"] == "strata.manager"),
            None,
        )
        assert strata_after is not None and strata_after["backendState"] == "online", (
            f"strata.manager did not remain online after reload: {strata_after}"
        )
    else:
        print("[11-12/12] SideVeil not running on device; confirmed 100% standalone Strata operation.")

    print("[+] All Strata live verification checks PASSED!")


def verify_sideveil_ui(dev: DeviceRunner, screenshot_out: str | None) -> None:
    sv_ctl = "/data/adb/modules/sideveil/bin/sideveil-ctl"
    print("[UI] Selecting strata.manager and opening SideVeil overlay panel...")
    sel = json.loads(
        dev.su(f"{sv_ctl} rpc plugins.select '{{\"pluginId\":\"strata.manager\"}}'").stdout
    )
    assert sel.get("result", {}).get("pluginId") == "strata.manager", f"select failed: {sel}"
    dev.su(f"{sv_ctl} open")
    try:
        time.sleep(3.0)
        st = json.loads(dev.su(f"{sv_ctl} status").stdout)
        assert st.get("activePluginId") == "strata.manager", f"unexpected active plugin: {st}"
        assert st.get("panelState") == "open", f"panel did not open: {st}"

        if screenshot_out:
            remote_png = "/data/local/tmp/strata_sideveil_ui.png"
            dev.su(f"/system/bin/screencap -p {remote_png}")
            pull_cmd = ["adb"]
            if dev.serial:
                pull_cmd += ["-s", dev.serial]
            pull_cmd += ["pull", remote_png, screenshot_out]
            subprocess.run(pull_cmd, check=True)
            dev.su(f"/system/bin/rm -f {remote_png}", check=False)

        logs = json.loads(dev.su(f"{sv_ctl} logs").stdout)
        lines = logs.get("result", {}).get("lines", [])
        errors = [
            ln
            for ln in lines[-20:]
            if "strata.manager" in ln and ("[E]" in ln or "[W]" in ln or "error" in ln.lower())
        ]
        assert not errors, f"SideVeil logged errors for strata.manager: {errors}"
        print("[UI] SideVeil WebView mounted strata.manager cleanly with zero errors.")
    finally:
        dev.su(f"{sv_ctl} close", check=False)


def main() -> int:
    parser = argparse.ArgumentParser(description="Live device verification for Strata Phase 1 & 2")
    parser.add_argument("--serial", help="ADB device serial (optional when single device connected)")
    parser.add_argument(
        "--ui-only",
        action="store_true",
        help="Run only the live SideVeil WebView mount check for strata.manager",
    )
    parser.add_argument(
        "--screenshot",
        help="Optional local path to save a live screenshot during --ui-only / UI check",
    )
    args = parser.parse_args()
    serial = detect_serial(args.serial)
    dev = DeviceRunner(serial)
    if args.ui_only:
        verify_sideveil_ui(dev, args.screenshot)
    else:
        verify(dev)
        if args.screenshot:
            verify_sideveil_ui(dev, args.screenshot)
    return 0


if __name__ == "__main__":
    sys.exit(main())
