#!/usr/bin/env python3
"""Local zero-ADB compatibility shim for Android Python test scripts inside Strata.

Translates `adb [-s <serial>] shell|exec-out|logcat|push|pull|devices|root|wait-for-device`
directly into local `host-exec` calls in Android's PID 1 namespace so existing PC-oriented
ADB test harnesses run unmodified on-device without enabling ADB.

Set STRATA_ADB_SHIM=0 (or STRATA_ADB_REAL=/path/to/adb) to delegate to a real adb binary.
"""

import os
import shlex
import shutil
import stat
import subprocess
import sys
import tarfile

HOST_EXEC = "/usr/local/bin/host-exec"
REAL_ADB_CANDIDATE = "/usr/bin/adb"
HOST_BUSYBOX = os.environ.get("STRATA_HOST_BUSYBOX", "/data/adb/ksu/bin/busybox")


def maybe_delegate_real_adb(argv: list[str]) -> None:
    real_bin = os.environ.get("STRATA_ADB_REAL", "").strip()
    if os.environ.get("STRATA_ADB_SHIM", "1") == "0" and not real_bin:
        real_bin = REAL_ADB_CANDIDATE
    if real_bin and os.path.isfile(real_bin) and os.access(real_bin, os.X_OK):
        os.execv(real_bin, [real_bin, *argv])


def parse_global_options(argv: list[str]) -> tuple[str | None, list[str]]:
    serial: str | None = None
    idx = 0
    value_flags = {"-s", "-t", "-H", "-P", "-L"}
    bool_flags = {"-a", "-d", "-e"}

    while idx < len(argv):
        tok = argv[idx]
        if tok in value_flags:
            if idx + 1 >= len(argv):
                raise SystemExit(f"adb-shim: option {tok} requires an argument")
            if tok == "-s":
                serial = argv[idx + 1]
            idx += 2
            continue
        if tok in bool_flags:
            idx += 1
            continue
        break
    return serial, argv[idx:]


def host_run(
    cmd: list[str],
    *,
    stdin=None,
    stdout=None,
    stderr=None,
    input_bytes: bytes | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess:
    return subprocess.run(
        [HOST_EXEC, *cmd],
        stdin=stdin,
        stdout=stdout,
        stderr=stderr,
        input=input_bytes,
        capture_output=capture_output,
        check=False,
    )


def normalize_shell_command(args: list[str]) -> list[str]:
    idx = 0
    while idx < len(args) and args[idx] in {"-T", "-t", "-n", "-x"}:
        idx += 1
    cmd_args = args[idx:]

    if not cmd_args:
        return ["/system/bin/sh"]

    if len(cmd_args) == 1 and cmd_args[0].startswith("su"):
        try:
            tokens = shlex.split(cmd_args[0])
        except ValueError:
            tokens = []
        if tokens in (["su"], ["su", "0"]):
            return ["/system/bin/sh"]
        if len(tokens) == 3 and tokens[0] == "su" and tokens[1] == "-c":
            return ["/system/bin/sh", "-c", tokens[2]]
        if len(tokens) == 4 and tokens[:3] == ["su", "0", "-c"]:
            return ["/system/bin/sh", "-c", tokens[3]]

    if cmd_args in (["su"], ["su", "0"]):
        return ["/system/bin/sh"]
    if len(cmd_args) >= 3 and cmd_args[0] == "su" and cmd_args[1] == "-c":
        return ["/system/bin/sh", "-c", " ".join(cmd_args[2:])]
    if len(cmd_args) >= 4 and cmd_args[:3] == ["su", "0", "-c"]:
        return ["/system/bin/sh", "-c", " ".join(cmd_args[3:])]
    if cmd_args[0] == "su" and len(cmd_args) >= 2:
        return ["/system/bin/sh", "-c", " ".join(cmd_args[1:])]

    return ["/system/bin/sh", "-c", " ".join(cmd_args)]


def handle_shell(args: list[str]) -> int:
    return host_run(normalize_shell_command(args)).returncode


def handle_exec_out(args: list[str]) -> int:
    if not args:
        print("adb-shim: exec-out requires a command", file=sys.stderr)
        return 1
    return host_run(["/system/bin/sh", "-c", " ".join(args)]).returncode


def handle_logcat(args: list[str]) -> int:
    return host_run(["/system/bin/logcat", *args]).returncode


def host_path_is_dir(path: str) -> bool:
    res = host_run(["/system/bin/sh", "-c", f"[ -d {shlex.quote(path)} ]"])
    return res.returncode == 0


def stream_single_file_to_host(src: str, dst: str) -> int:
    mode = stat.S_IMODE(os.stat(src).st_mode)
    parent = os.path.dirname(dst) or "/"
    remote_cmd = (
        f"mkdir -p {shlex.quote(parent)} && "
        f"cat > {shlex.quote(dst)} && "
        f"chmod {mode:04o} {shlex.quote(dst)}"
    )
    with open(src, "rb") as f:
        res = host_run(["/system/bin/sh", "-c", remote_cmd], stdin=f)
    return res.returncode


def stream_entries_to_host_dir(sources: list[str], dst_dir: str) -> int:
    clean_dst = dst_dir.rstrip("/") or "/"
    mk = host_run(["/system/bin/sh", "-c", f"mkdir -p {shlex.quote(clean_dst)}"])
    if mk.returncode != 0:
        return mk.returncode

    proc = subprocess.Popen(
        [HOST_EXEC, HOST_BUSYBOX, "tar", "-xf", "-", "-C", clean_dst],
        stdin=subprocess.PIPE,
    )
    assert proc.stdin is not None
    try:
        with tarfile.open(fileobj=proc.stdin, mode="w|", format=tarfile.GNU_FORMAT) as tf:
            for src in sources:
                norm = os.path.normpath(src)
                tf.add(norm, arcname=os.path.basename(norm))
    finally:
        proc.stdin.close()
    return proc.wait()


def handle_push(args: list[str]) -> int:
    filtered: list[str] = []
    idx = 0
    while idx < len(args):
        tok = args[idx]
        if tok in {"--sync", "-n"}:
            idx += 1
            continue
        if tok == "-z" and idx + 1 < len(args):
            idx += 2
            continue
        filtered.append(tok)
        idx += 1

    if len(filtered) < 2:
        print("adb-shim: push requires <local...> <remote>", file=sys.stderr)
        return 1

    sources = filtered[:-1]
    dst = filtered[-1]
    for src in sources:
        if not os.path.exists(src):
            print(f"adb-shim: cannot stat '{src}': No such file or directory", file=sys.stderr)
            return 1

    dst_is_dir = dst.endswith("/") or len(sources) > 1 or host_path_is_dir(dst)
    if len(sources) == 1 and os.path.isfile(sources[0]) and not dst_is_dir:
        return stream_single_file_to_host(sources[0], dst)

    if len(sources) == 1 and os.path.isdir(sources[0]) and not dst_is_dir:
        mk = host_run(["/system/bin/sh", "-c", f"mkdir -p {shlex.quote(dst)}"])
        if mk.returncode != 0:
            return mk.returncode
        children = [os.path.join(sources[0], name) for name in os.listdir(sources[0])]
        if not children:
            return 0
        return stream_entries_to_host_dir(children, dst)

    return stream_entries_to_host_dir(sources, dst)


def handle_pull(args: list[str]) -> int:
    filtered = [tok for tok in args if tok not in {"-a", "-z"}]
    if not filtered:
        print("adb-shim: pull requires <remote> [local]", file=sys.stderr)
        return 1

    src = filtered[0]
    dst = filtered[1] if len(filtered) > 1 else "."
    src_norm = src.rstrip("/") or "/"
    parent = os.path.dirname(src_norm) or "/"
    base = os.path.basename(src_norm)

    if host_path_is_dir(src_norm):
        os.makedirs(dst, exist_ok=True)
        proc = subprocess.Popen(
            [HOST_EXEC, HOST_BUSYBOX, "tar", "-cf", "-", "-C", parent, base],
            stdout=subprocess.PIPE,
        )
        assert proc.stdout is not None
        try:
            with tarfile.open(fileobj=proc.stdout, mode="r|*") as tf:
                tf.extractall(path=dst, filter="data")
        except tarfile.ReadError:
            pass
        finally:
            proc.stdout.close()
        return proc.wait()

    target_file = os.path.join(dst, base) if os.path.isdir(dst) or dst.endswith("/") else dst
    os.makedirs(os.path.dirname(os.path.abspath(target_file)), exist_ok=True)
    proc = subprocess.Popen(
        [HOST_EXEC, "/system/bin/cat", src_norm],
        stdout=subprocess.PIPE,
    )
    assert proc.stdout is not None
    try:
        with open(target_file, "wb") as f:
            shutil.copyfileobj(proc.stdout, f)
    finally:
        proc.stdout.close()
    rc = proc.wait()
    if rc != 0 and os.path.isfile(target_file):
        os.remove(target_file)
    return rc


def main(argv: list[str]) -> int:
    maybe_delegate_real_adb(argv)
    serial, rest = parse_global_options(argv)
    if not rest:
        print("adb-shim: local Strata host-exec bridge active", file=sys.stderr)
        return 0

    cmd, cmd_args = rest[0], rest[1:]
    if cmd == "shell":
        return handle_shell(cmd_args)
    if cmd == "exec-out":
        return handle_exec_out(cmd_args)
    if cmd == "logcat":
        return handle_logcat(cmd_args)
    if cmd == "push":
        return handle_push(cmd_args)
    if cmd == "pull":
        return handle_pull(cmd_args)
    if cmd == "devices":
        device_id = serial or "local"
        print("List of devices attached")
        if "-l" in cmd_args:
            print(f"{device_id}\tdevice transport_id:1")
        else:
            print(f"{device_id}\tdevice")
        return 0
    if cmd == "get-state":
        print("device")
        return 0
    if cmd == "get-serialno":
        print(serial or "local")
        return 0
    if cmd in {"root", "wait-for-device", "start-server", "kill-server"}:
        return 0
    if cmd == "version":
        print("Strata local ADB shim (host-exec bridge)")
        return 0

    print(f"adb-shim: unsupported subcommand '{cmd}'", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
