#!/usr/bin/env bash
#
# Strata KernelSU Module Builder
# Validates versions and Rust/WebUI quality gates, cross-compiles stratad + strata CLI
# (aarch64-linux-android) and static musl strata-debug (aarch64-unknown-linux-musl),
# builds SideVeil plugin + standalone KernelSU WebUI into out/stage/, and packages out/strata.zip.
#

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAGE_DIR="${ROOT_DIR}/out/stage"
STAGE_BIN_DIR="${STAGE_DIR}/bin"
OUTPUT_DIR="${ROOT_DIR}/out"
PACKAGE_NAME="strata.zip"

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER:-${ROOT_DIR}/scripts/aarch64-android-linker.sh}"
export CC_aarch64_linux_android="${CC_aarch64_linux_android:-${ROOT_DIR}/scripts/aarch64-android-linker.sh}"

run_in_env() {
    if command -v mise >/dev/null 2>&1; then
        mise exec -- "$@"
    else
        "$@"
    fi
}

verify_static_elf() {
    local bin_path="$1"
    if command -v readelf >/dev/null 2>&1; then
        if readelf -l "$bin_path" | grep -q 'INTERP'; then
            echo "[!] Error: ${bin_path} contains PT_INTERP (expected fully static musl ELF)." >&2
            exit 1
        fi
    elif command -v llvm-readelf >/dev/null 2>&1; then
        if llvm-readelf -l "$bin_path" | grep -q 'INTERP'; then
            echo "[!] Error: ${bin_path} contains PT_INTERP (expected fully static musl ELF)." >&2
            exit 1
        fi
    else
        python3 - "$bin_path" <<'PYEOF'
import struct, sys
data = open(sys.argv[1], "rb").read()
assert data[:4] == b"\x7fELF" and data[4] == 2, "not a 64-bit ELF"
e_phoff = struct.unpack_from("<Q", data, 32)[0]
e_phentsize, e_phnum = struct.unpack_from("<HH", data, 54)
for i in range(e_phnum):
    p_type = struct.unpack_from("<I", data, e_phoff + i * e_phentsize)[0]
    if p_type == 3:
        raise SystemExit("PT_INTERP found in binary")
PYEOF
    fi
}

echo "[*] Checking version parity across all Strata targets..."
node "${ROOT_DIR}/scripts/bump-version.mjs" --check

echo "[*] Running Rust quality gates (daemon + tools/strata-debug-agent)..."
for ws in daemon tools/strata-debug-agent; do
    (
        cd "${ROOT_DIR}/${ws}"
        run_in_env cargo fmt --check
        run_in_env cargo clippy --workspace --all-targets -- -D warnings
        run_in_env cargo test --workspace
    )
done

echo "[*] Building and checking SideVeil plugin & standalone KernelSU WebUI..."
(
    cd "${ROOT_DIR}/webui"
    run_in_env pnpm check
)

echo "[*] Cross-compiling stratad and strata CLI for aarch64-linux-android..."
(
    cd "${ROOT_DIR}/daemon"
    run_in_env cargo build --release --target aarch64-linux-android -p stratad -p strata-ctl
)

echo "[*] Cross-compiling static musl strata-debug for aarch64-unknown-linux-musl..."
(
    cd "${ROOT_DIR}/tools/strata-debug-agent"
    run_in_env cargo build --release --target aarch64-unknown-linux-musl
)

DEBUG_BIN="${ROOT_DIR}/tools/strata-debug-agent/target/aarch64-unknown-linux-musl/release/strata-debug"
verify_static_elf "${DEBUG_BIN}"

mkdir -p "${STAGE_BIN_DIR}" "${OUTPUT_DIR}"
cp "${ROOT_DIR}/daemon/target/aarch64-linux-android/release/stratad" "${STAGE_BIN_DIR}/stratad"
cp "${ROOT_DIR}/daemon/target/aarch64-linux-android/release/strata" "${STAGE_BIN_DIR}/strata"
cp "${DEBUG_BIN}" "${STAGE_BIN_DIR}/strata-debug"
chmod 0755 "${STAGE_BIN_DIR}/stratad" "${STAGE_BIN_DIR}/strata" "${STAGE_BIN_DIR}/strata-debug"

echo "[*] Packaging KernelSU archive -> ${OUTPUT_DIR}/${PACKAGE_NAME}..."
rm -f "${OUTPUT_DIR}/${PACKAGE_NAME}"

python3 - "${ROOT_DIR}" "${OUTPUT_DIR}/${PACKAGE_NAME}" <<'EOF'
import os
import stat
import sys
import zipfile

root_dir = sys.argv[1]
output_zip = sys.argv[2]

# (archive_path, source_path_relative_to_root, posix_mode)
entries = [
    ("module.prop", "module/module.prop", 0o644),
    ("config.json", "module/config.json", 0o644),
    ("service.sh", "module/service.sh", 0o755),
    ("post-fs-data.sh", "module/post-fs-data.sh", 0o755),
    ("customize.sh", "module/customize.sh", 0o755),
    ("uninstall.sh", "module/uninstall.sh", 0o755),
    ("bin/stratad", "out/stage/bin/stratad", 0o755),
    ("bin/strata", "out/stage/bin/strata", 0o755),
    ("bin/strata-debug", "out/stage/bin/strata-debug", 0o755),
    ("container/host-exec", "module/container/host-exec", 0o755),
    ("container/setup-container.sh", "module/container/setup-container.sh", 0o755),
    ("scripts/sync-sideveil-plugin.sh", "scripts/sync-sideveil-plugin.sh", 0o755),
    ("plugins/strata.manager/manifest.json", "module/plugins/strata.manager/manifest.json", 0o644),
]

def add_tree(src_rel_root: str, arc_prefix: str, required_file: str) -> None:
    src_root = os.path.join(root_dir, src_rel_root)
    if not os.path.isfile(os.path.join(src_root, required_file)):
        raise FileNotFoundError(f"Missing {required_file} under {src_root}")
    for dirpath, dirnames, filenames in os.walk(src_root):
        dirnames.sort()
        for name in sorted(filenames):
            full = os.path.join(dirpath, name)
            rel_in_tree = os.path.relpath(full, src_root).replace(os.sep, "/")
            arc_path = f"{arc_prefix}/{rel_in_tree}"
            src_rel = os.path.relpath(full, root_dir).replace(os.sep, "/")
            entries.append((arc_path, src_rel, 0o644))

add_tree("out/stage/plugins/strata.manager/web", "plugins/strata.manager/web", "index.html")
add_tree("out/stage/webroot", "webroot", "index.html")

with zipfile.ZipFile(output_zip, "w", zipfile.ZIP_DEFLATED) as zf:
    for arc_path, src_rel, mode in entries:
        full_path = os.path.join(root_dir, src_rel)
        if not os.path.isfile(full_path):
            raise FileNotFoundError(f"Required module file missing: {full_path}")
        with open(full_path, "rb") as f:
            data = f.read()
        info = zipfile.ZipInfo(arc_path)
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = (stat.S_IFREG | mode) << 16
        zf.writestr(info, data)

print("[+] Packaged module archive:", output_zip)
EOF
