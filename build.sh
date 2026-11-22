#!/usr/bin/env bash
#
# Strata KernelSU Module Builder
# Validates Rust workspace, cross-compiles stratad + strata CLI for aarch64-linux-android,
# and packages out/strata.zip with exact POSIX permissions.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET_DIR="${SCRIPT_DIR}/target/aarch64-linux-android/release"
BIN_DIR="${SCRIPT_DIR}/bin"
OUTPUT_DIR="${SCRIPT_DIR}/out"
PACKAGE_NAME="strata.zip"

run_in_env() {
    if command -v mise >/dev/null 2>&1; then
        mise exec -- "$@"
    else
        "$@"
    fi
}

echo "[*] Running Rust quality gates (fmt, clippy -D warnings, host unit tests)..."
(
    cd "${SCRIPT_DIR}"
    run_in_env cargo fmt --check
    run_in_env cargo clippy --workspace --all-targets -- -D warnings
    run_in_env cargo test --workspace
)

echo "[*] Running Python syntax and unit test checks..."
(
    cd "${SCRIPT_DIR}"
    python3 -m py_compile container/strata-debug/*.py scripts/*.py scripts/strata_mcp/*.py scripts/verify_live/*.py
    python3 -m unittest discover -s tests
)

echo "[*] Running SideVeil plugin WebUI quality gate and Vite build..."
(
    cd "${SCRIPT_DIR}/webui"
    run_in_env pnpm check
)

echo "[*] Cross-compiling stratad and strata CLI for aarch64-linux-android..."
(
    cd "${SCRIPT_DIR}"
    run_in_env cargo build --release --target aarch64-linux-android -p stratad -p strata-ctl
)

mkdir -p "${BIN_DIR}" "${OUTPUT_DIR}"
cp "${TARGET_DIR}/stratad" "${BIN_DIR}/stratad"
cp "${TARGET_DIR}/strata" "${BIN_DIR}/strata"
chmod 0755 "${BIN_DIR}/stratad" "${BIN_DIR}/strata"

echo "[*] Packaging KernelSU archive -> ${OUTPUT_DIR}/${PACKAGE_NAME}..."
rm -f "${OUTPUT_DIR}/${PACKAGE_NAME}"

python3 - <<EOF
import os
import stat
import zipfile

script_dir = "${SCRIPT_DIR}"
output_zip = os.path.join("${OUTPUT_DIR}", "${PACKAGE_NAME}")

explicit_files = [
    ("module.prop", 0o644),
    ("config.json", 0o644),
    ("service.sh", 0o755),
    ("post-fs-data.sh", 0o755),
    ("customize.sh", 0o755),
    ("uninstall.sh", 0o755),
    ("bin/stratad", 0o755),
    ("bin/strata", 0o755),
    ("container/host-exec", 0o755),
    ("container/setup-container.sh", 0o755),
    ("scripts/sync-sideveil-plugin.sh", 0o755),
]

debug_pkg_root = os.path.join(script_dir, "container", "strata-debug")
if os.path.isdir(debug_pkg_root):
    for dirpath, dirnames, filenames in os.walk(debug_pkg_root):
        dirnames[:] = [d for d in sorted(dirnames) if d != "__pycache__"]
        for name in sorted(filenames):
            if not name.endswith(".py"):
                continue
            full = os.path.join(dirpath, name)
            rel = os.path.relpath(full, script_dir).replace(os.sep, "/")
            mode = 0o755 if name in {"cli.py", "forward_bridge.py"} else 0o644
            explicit_files.append((rel, mode))

plugin_root = os.path.join(script_dir, "plugins", "strata.manager")
if not os.path.isfile(os.path.join(plugin_root, "manifest.json")):
    raise FileNotFoundError(f"Missing SideVeil plugin manifest under {plugin_root}")

for dirpath, _dirnames, filenames in os.walk(plugin_root):
    for name in sorted(filenames):
        full = os.path.join(dirpath, name)
        rel = os.path.relpath(full, script_dir).replace(os.sep, "/")
        explicit_files.append((rel, 0o644))

with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
    for rel_path, mode in explicit_files:
        full_path = os.path.join(script_dir, rel_path)
        if not os.path.isfile(full_path):
            raise FileNotFoundError(f"Required module file missing: {full_path}")
        with open(full_path, "rb") as f:
            data = f.read()
        info = zipfile.ZipInfo(rel_path)
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = (stat.S_IFREG | mode) << 16
        zf.writestr(info, data)

print("[+] Packaged module archive:", output_zip)
EOF
