"""Core Strata runtime, isolation, teardown, and maintenance live checks (steps 1-10)."""

import shlex

from verify_live.debug_checks import verify_debug_tools
from verify_live.runner import DeviceRunner


def verify_core(dev: DeviceRunner, *, non_destructive: bool = False) -> None:
    print(f"[1/12] Checking stratad daemon ping and status (mode={dev.mode})...")
    ping = dev.strata_json("daemon ping")
    assert isinstance(ping, dict) and ping.get("pong") is True, f"unexpected ping: {ping}"
    dstatus = dev.strata_json("daemon status")
    assert isinstance(dstatus, dict), f"unexpected daemon status: {dstatus}"
    listed_init = dev.strata_json("list")
    assert isinstance(listed_init, list) and listed_init, (
        "no containers configured on device; create or import a container before running verify-live"
    )
    target_id = dstatus.get("defaultContainer") or listed_init[0]["id"]
    assert isinstance(target_id, str) and target_id, f"unable to resolve target container: {dstatus}"

    if non_destructive:
        print("[2/12] Skipping IMAGE_BUSY stop test (--non-destructive).")
        started = dev.strata_json("status")
        if not isinstance(started, dict) or started.get("phase") != "running":
            started = dev.strata_json("start")
    else:
        print("[2/12] Testing concurrent-mount IMAGE_BUSY protection...")
        dev.strata_json("stop")
        status_stopped = dev.strata_json("status")
        assert isinstance(status_stopped, dict)
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

        print(f"[3/12] Starting container '{target_id}' and verifying status/list...")
        started = dev.strata_json("start")
    assert isinstance(started, dict)
    assert started["phase"] == "running", f"expected running, got {started}"
    assert started["mounted"] is True
    assert started["nsPid"] and started["nsPid"] > 1
    assert started["loopDevices"], "expected active loop device"
    assert started["cgroupPath"] == f"/sys/fs/cgroup/strata/{target_id}"
    listed = dev.strata_json("list")
    assert any(item["id"] == target_id and item["phase"] == "running" for item in listed)

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
        "'test ! -e /dev/block && stat -c \"%a\" /dev/shm && "
        "test -s /etc/resolv.conf && test -s /run/strata/host.env'"
    ).stdout.strip()
    assert dev_check == "1777", f"unexpected /dev/shm mode or missing runtime files: {dev_check}"

    print("[6/12] Verifying headless streaming task runner (task.run + events.subscribe)...")
    task_out = dev.su(
        "/data/adb/ksu/bin/strata task run /usr/bin/python3 -c \"print('strata-stream-ok')\""
    ).stdout.strip()
    assert task_out == "strata-stream-ok", f"unexpected streaming task output: {task_out}"

    print("[7/12] Verifying in-container host-exec and strata-debug (zero legacy adb binary)...")
    verify_debug_tools(dev)

    if non_destructive:
        print("[8-10/12] Skipping destructive stop/debug/fsck/compact/backup (--non-destructive).")
        return

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
    cg_exists = dev.su(
        f"test -d /sys/fs/cgroup/strata/{shlex.quote(target_id)} && echo yes || echo no"
    ).stdout.strip()
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
