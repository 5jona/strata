#!/usr/bin/env python3
"""End-to-end live Android device verification CLI for Strata Phase 1 & 2."""

import argparse
import sys

from verify_live.core_checks import verify_core
from verify_live.runner import DeviceRunner
from verify_live.sideveil_checks import verify_sideveil_backend, verify_sideveil_ui


def verify(dev: DeviceRunner, *, non_destructive: bool = False) -> None:
    verify_core(dev, non_destructive=non_destructive)
    verify_sideveil_backend(dev)
    print("[+] All Strata live verification checks PASSED!")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Live device verification for Strata")
    parser.add_argument(
        "--mode",
        choices=["auto", "ssh", "adb", "host-exec", "local"],
        default="auto",
        help="Transport mode (default: auto; checks --adb / STRATA_SSH_HOST / STRATA_ADB_SERIAL, host-exec, local)",
    )
    parser.add_argument(
        "--adb",
        help="Target ADB device serial; creates an isolated temporary local SSH forward (default: $STRATA_ADB_SERIAL)",
    )
    parser.add_argument(
        "--ssh",
        help="Target host or user@host for container SSH mode (default: $STRATA_SSH_HOST / $SSH_TARGET)",
    )
    parser.add_argument(
        "--ssh-port",
        type=int,
        default=None,
        help="SSH port (default: $STRATA_SSH_PORT / $SSH_PORT / 2222)",
    )
    parser.add_argument(
        "--non-destructive",
        action="store_true",
        help="Skip container-stopping tests (auto-enabled in ssh/host-exec modes)",
    )
    parser.add_argument(
        "--allow-destructive",
        action="store_true",
        help="Force container-stopping tests even in ssh/host-exec modes",
    )
    parser.add_argument(
        "--ui-only",
        action="store_true",
        help="Run only the live SideVeil WebView mount check for strata.manager",
    )
    parser.add_argument(
        "--screenshot",
        help="Optional local path to save a compact visual capture during UI check",
    )
    args = parser.parse_args(argv)
    with DeviceRunner(
        mode=args.mode,
        ssh_target=args.ssh,
        ssh_port=args.ssh_port,
        adb_serial=args.adb,
    ) as dev:
        non_destructive = args.non_destructive or (
            dev.mode in {"ssh", "host-exec"} and not args.allow_destructive
        )
        if args.ui_only:
            verify_sideveil_ui(dev, args.screenshot)
        else:
            verify(dev, non_destructive=non_destructive)
            if args.screenshot:
                verify_sideveil_ui(dev, args.screenshot)
    return 0


if __name__ == "__main__":
    sys.exit(main())
