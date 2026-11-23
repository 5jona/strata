#!/usr/bin/env bash
#
# Idempotent bootstrapper for deploying Strata + Ubuntu 24.04 ARM64 container +
# Dropbear SSH + host-exec + strata-debug + uinput/L5 diagnostics onto any
# KernelSU-rooted Android device over ADB.
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/strata-bootstrap"
REMOTE_MOD_DIR="/data/adb/modules/strata"
IMAGE_PATH="/data/local/strata/containers/main/rootfs.img"
MOUNT_DIR="/data/local/strata/containers/main/mnt"
CONTAINER_STAGE="/data/local/tmp/strata_container_assets"
ZIP_PATH="${SCRIPT_DIR}/out/strata.zip"

UBUNTU_BASE_URL="${UBUNTU_BASE_URL:-https://cdimage.ubuntu.com/ubuntu-base/releases/24.04/release}"
UBUNTU_VERSION="${UBUNTU_VERSION:-auto}"

SERIAL="${STRATA_ADB_SERIAL:-}"
IMAGE_SIZE="32G"
PUBKEY_PATH=""
RECREATE=0
SKIP_BUILD=0
SKIP_MODULE_INSTALL=0
NO_REBOOT_WAIT=0
ENABLE_DEBUG_MOUNTS=1
DEBUG_PROFILE=0

usage() {
    cat <<'EOF'
Usage: scripts/bootstrap-device.sh [options]

Options:
  --serial <adb-serial>   Target ADB serial (auto-detected if single device attached)
  --size <size>           Sparse ext4 rootfs image cap (default: 32G)
  --pubkey <path>         SSH public key (default: ~/.ssh/id_ed25519.pub or id_rsa.pub)
  --recreate              Force recreate /data/local/strata/containers/main/rootfs.img
  --skip-build            Skip build.sh and use existing out/strata.zip
  --skip-module-install   Skip module build/install (resume container setup after reboot)
  --no-reboot-wait        Exit 0 with instructions if KernelSU staged modules_update
  --enable-debug-mounts   Enable uinput char dev and L5 diagnostics mounts (default: on)
  --disable-debug-mounts  Disable automatic uinput/L5 diagnostics mount configuration
  --debug-profile         Install extra debug packages in setup-container.sh
  -h, --help              Show this help message
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --serial) SERIAL="$2"; shift 2 ;;
        --size) IMAGE_SIZE="$2"; shift 2 ;;
        --pubkey) PUBKEY_PATH="$2"; shift 2 ;;
        --recreate) RECREATE=1; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --skip-module-install) SKIP_MODULE_INSTALL=1; shift ;;
        --no-reboot-wait) NO_REBOOT_WAIT=1; shift ;;
        --enable-debug-mounts) ENABLE_DEBUG_MOUNTS=1; shift ;;
        --disable-debug-mounts) ENABLE_DEBUG_MOUNTS=0; shift ;;
        --debug-profile) DEBUG_PROFILE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "[!] Unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

adb_cmd() { adb -s "${SERIAL}" "$@"; }
adb_su() { adb -s "${SERIAL}" shell "su -c $(printf '%q' "$1")"; }

resolve_serial() {
    [ -n "${SERIAL}" ] && return 0
    local serials count
    serials="$(adb devices | awk 'NR>1 && $2=="device" {print $1}')"
    count="$(printf '%s\n' "${serials}" | sed '/^$/d' | wc -l | tr -d ' ')"
    if [ "${count}" -eq 1 ]; then
        SERIAL="${serials}"
    elif [ "${count}" -eq 0 ]; then
        echo "[!] No online ADB device found. Connect a device or pass --serial <serial>." >&2
        exit 1
    else
        echo "[!] Multiple ADB devices detected. Specify one with --serial <serial>:" >&2
        printf '    %s\n' ${serials} >&2
        exit 1
    fi
}

resolve_pubkey() {
    if [ -n "${PUBKEY_PATH}" ]; then
        [ -f "${PUBKEY_PATH}" ] || { echo "[!] SSH public key not found: ${PUBKEY_PATH}" >&2; exit 1; }
        return 0
    fi
    for cand in "${HOME}/.ssh/id_ed25519.pub" "${HOME}/.ssh/id_rsa.pub"; do
        if [ -f "${cand}" ]; then
            PUBKEY_PATH="${cand}"
            return 0
        fi
    done
}

check_device_root() {
    local state id_out
    state="$(adb_cmd get-state 2>/dev/null | tr -d '\r\n' || true)"
    [ "${state}" = "device" ] || { echo "[!] ADB device '${SERIAL}' offline (state='${state:-unknown}')." >&2; exit 1; }
    id_out="$(adb_su "id" 2>/dev/null | tr -d '\r' || true)"
    case "${id_out}" in
        *uid=0*) ;;
        *) echo "[!] Root shell (su -c id) unavailable on '${SERIAL}': ${id_out:-no output}" >&2; exit 1 ;;
    esac
}

install_module_zip() {
    if [ "${SKIP_BUILD}" -eq 0 ] || [ ! -f "${ZIP_PATH}" ]; then
        echo "[*] Building Strata module archive..."
        bash "${SCRIPT_DIR}/build.sh"
    fi
    [ -f "${ZIP_PATH}" ] || { echo "[!] Module archive missing: ${ZIP_PATH}" >&2; exit 1; }

    echo "[*] Pushing ${ZIP_PATH} to device..."
    adb_cmd push "${ZIP_PATH}" /data/local/tmp/strata.zip >/dev/null

    if [ "$(adb_su '[ -x /data/adb/ksu/bin/ksud ] && echo YES || echo NO' | tr -d '\r')" = "YES" ]; then
        echo "[*] Installing module via KernelSU (ksud module install)..."
        adb_su "/data/adb/ksu/bin/ksud module install /data/local/tmp/strata.zip && /system/bin/rm -f /data/local/tmp/strata.zip"
    else
        echo "[*] ksud not found; extracting module directly to ${REMOTE_MOD_DIR}..."
        adb_su "
            set -e
            if [ -x /data/adb/ksu/bin/busybox ]; then UNZIP_BIN='/data/adb/ksu/bin/busybox unzip'
            elif [ -x /system/bin/unzip ]; then UNZIP_BIN='/system/bin/unzip'
            else echo 'unzip binary not found on device' >&2; exit 1; fi
            /system/bin/mkdir -p ${REMOTE_MOD_DIR} /data/adb/strata/run/logs ${MOUNT_DIR}
            \$UNZIP_BIN -qo /data/local/tmp/strata.zip -d ${REMOTE_MOD_DIR}
            /system/bin/rm -f /data/local/tmp/strata.zip
            /system/bin/chown 0:0 /data/adb/strata /data/adb/strata/run /data/adb/strata/run/logs
            /system/bin/chmod 0700 /data/adb/strata /data/adb/strata/run /data/adb/strata/run/logs
            /system/bin/chmod 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main ${MOUNT_DIR} \
                ${REMOTE_MOD_DIR}/service.sh ${REMOTE_MOD_DIR}/post-fs-data.sh ${REMOTE_MOD_DIR}/customize.sh \
                ${REMOTE_MOD_DIR}/uninstall.sh ${REMOTE_MOD_DIR}/bin/stratad ${REMOTE_MOD_DIR}/bin/strata \
                ${REMOTE_MOD_DIR}/container/host-exec ${REMOTE_MOD_DIR}/container/setup-container.sh \
                ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh
            /system/bin/chmod 0644 ${REMOTE_MOD_DIR}/module.prop ${REMOTE_MOD_DIR}/config.json
            [ -d ${REMOTE_MOD_DIR}/container/strata-debug ] && /system/bin/chmod 0755 ${REMOTE_MOD_DIR}/container/strata-debug/cli.py ${REMOTE_MOD_DIR}/container/strata-debug/forward_bridge.py 2>/dev/null || true
            if [ ! -f /data/adb/strata/config.json ]; then
                /system/bin/cp ${REMOTE_MOD_DIR}/config.json /data/adb/strata/config.json
                /system/bin/chown 0:0 /data/adb/strata/config.json
                /system/bin/chmod 0600 /data/adb/strata/config.json
            fi
            [ -d /data/adb/ksu/bin ] && /system/bin/ln -sf ${REMOTE_MOD_DIR}/bin/strata /data/adb/ksu/bin/strata || true
            [ -x ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh ] && /system/bin/sh ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh ${REMOTE_MOD_DIR} || true
        "
    fi

    if [ "$(adb_su "[ -x ${REMOTE_MOD_DIR}/bin/stratad ] && echo YES || echo NO" | tr -d '\r')" != "YES" ]; then
        echo ""
        echo "[!] Stage 1 complete: KernelSU staged Strata under /data/adb/modules_update/strata."
        echo "[!] Reboot the device to activate ${REMOTE_MOD_DIR}, then run Stage 2:"
        echo "    adb -s ${SERIAL} reboot"
        echo "    bash scripts/bootstrap-device.sh --serial ${SERIAL} --skip-module-install --size ${IMAGE_SIZE}${PUBKEY_PATH:+ --pubkey ${PUBKEY_PATH}}"
        [ "${NO_REBOOT_WAIT}" -eq 1 ] && exit 0
        exit 2
    fi
}

prepare_rootfs_image() {
    adb_su "${REMOTE_MOD_DIR}/bin/strata stop >/dev/null 2>&1 || true"
    local exists
    exists="$(adb_su "[ -f ${IMAGE_PATH} ] && echo YES || echo NO" | tr -d '\r')"
    if [ "${RECREATE}" -eq 0 ] && [ "${exists}" = "YES" ]; then
        echo "[*] Reusing existing rootfs image: ${IMAGE_PATH}"
        return 0
    fi

    mkdir -p "${CACHE_DIR}"
    local sums_file="${CACHE_DIR}/SHA256SUMS" tarball expected_sha local_tar actual_sha
    echo "[*] Fetching Ubuntu 24.04 ARM64 checksums from ${UBUNTU_BASE_URL}..."
    curl -fsSL -o "${sums_file}" "${UBUNTU_BASE_URL}/SHA256SUMS"

    if [ "${UBUNTU_VERSION}" = "auto" ]; then
        tarball="$(awk '$2 ~ /^\*?ubuntu-base-24\.04(\.[0-9]+)?-base-arm64\.tar\.gz$/ {sub(/^\*/, "", $2); print $2}' "${sums_file}" | sort -V | tail -n 1)"
    else
        tarball="ubuntu-base-${UBUNTU_VERSION}-base-arm64.tar.gz"
    fi
    [ -n "${tarball}" ] || { echo "[!] Failed to resolve Ubuntu 24.04 ARM64 tarball from ${sums_file}" >&2; exit 1; }
    expected_sha="$(awk -v f="${tarball}" '{name=$2; sub(/^\*/, "", name); if (name == f) print $1}' "${sums_file}")"
    [ -n "${expected_sha}" ] || { echo "[!] Failed to find SHA256 checksum for ${tarball}" >&2; exit 1; }

    local_tar="${CACHE_DIR}/${tarball}"
    if [ ! -f "${local_tar}" ] || [ "$(sha256sum "${local_tar}" | awk '{print $1}')" != "${expected_sha}" ]; then
        echo "[*] Downloading ${tarball} into ${CACHE_DIR}..."
        curl -fSL -o "${local_tar}" "${UBUNTU_BASE_URL}/${tarball}"
        actual_sha="$(sha256sum "${local_tar}" | awk '{print $1}')"
        [ "${actual_sha}" = "${expected_sha}" ] || { echo "[!] SHA256 mismatch: got ${actual_sha}, expected ${expected_sha}" >&2; exit 1; }
    else
        echo "[*] Using cached verified tarball: ${local_tar}"
    fi

    echo "[*] Pushing ${tarball} and unpacking into sparse ext4 image (${IMAGE_SIZE})..."
    adb_cmd push "${local_tar}" "/data/local/tmp/${tarball}" >/dev/null
    adb_su "
        set -e
        LOOP_DEV=''
        cleanup() {
            /system/bin/umount ${MOUNT_DIR} 2>/dev/null || true
            [ -n \"\${LOOP_DEV}\" ] && /system/bin/losetup -d \"\${LOOP_DEV}\" 2>/dev/null || true
            /system/bin/rm -f /data/local/tmp/${tarball}
        }
        trap cleanup EXIT
        /system/bin/umount ${MOUNT_DIR} 2>/dev/null || true
        /system/bin/mkdir -p $(dirname "${IMAGE_PATH}") ${MOUNT_DIR}
        /system/bin/chmod 0755 /data/local/strata /data/local/strata/containers $(dirname "${IMAGE_PATH}") ${MOUNT_DIR}
        /system/bin/rm -f ${IMAGE_PATH}
        truncate -s ${IMAGE_SIZE} ${IMAGE_PATH}
        /system/bin/mkfs.ext4 -F -L strata_main ${IMAGE_PATH} >/dev/null
        /system/bin/chcon u:object_r:vold_data_file:s0 ${IMAGE_PATH} 2>/dev/null || true
        LOOP_DEV=\$(/system/bin/losetup -f --show ${IMAGE_PATH})
        /system/bin/mount -t ext4 -o rw,relatime \"\${LOOP_DEV}\" ${MOUNT_DIR}
        if [ -x /data/adb/ksu/bin/busybox ]; then /data/adb/ksu/bin/busybox tar -xzpf /data/local/tmp/${tarball} -C ${MOUNT_DIR}
        elif [ -x /system/bin/tar ]; then /system/bin/tar -xzpf /data/local/tmp/${tarball} -C ${MOUNT_DIR}
        else echo 'No suitable tar binary found on device' >&2; exit 1; fi
        /system/bin/mkdir -p ${MOUNT_DIR}/data/local/tmp ${MOUNT_DIR}/root/work
        /system/bin/sync
    "
}

provision_rootfs_via_chroot() {
    echo "[*] Staging container scripts, strata-debug, and SSH public key..."
    adb_cmd shell "rm -rf ${CONTAINER_STAGE} && mkdir -p ${CONTAINER_STAGE}"
    adb_cmd push \
        "${SCRIPT_DIR}/container/setup-container.sh" \
        "${SCRIPT_DIR}/container/host-exec" \
        "${SCRIPT_DIR}/container/strata-debug" \
        "${CONTAINER_STAGE}/" >/dev/null
    [ -n "${PUBKEY_PATH}" ] && adb_cmd push "${PUBKEY_PATH}" "${CONTAINER_STAGE}/authorized_keys" >/dev/null

    local setup_flags=""
    [ "${DEBUG_PROFILE}" -eq 1 ] && setup_flags="--debug-profile"

    echo "[*] Running container/setup-container.sh inside chroot (installing Dropbear, Python, tools)..."
    adb_su "
        set -eu
        LOOP_DEV=''
        cleanup() {
            for p in ${MOUNT_DIR}/data/local/tmp ${MOUNT_DIR}/dev/pts ${MOUNT_DIR}/dev ${MOUNT_DIR}/sys ${MOUNT_DIR}/proc; do
                /system/bin/umount -l \"\$p\" 2>/dev/null || true
            done
            /system/bin/umount -l ${MOUNT_DIR} 2>/dev/null || true
            [ -n \"\${LOOP_DEV}\" ] && /system/bin/losetup -d \"\${LOOP_DEV}\" 2>/dev/null || true
            /system/bin/rm -rf ${CONTAINER_STAGE}
        }
        trap cleanup EXIT
        ${REMOTE_MOD_DIR}/bin/strata stop >/dev/null 2>&1 || true
        /system/bin/mkdir -p ${MOUNT_DIR}
        LOOP_DEV=\$(/system/bin/losetup -f --show ${IMAGE_PATH})
        /system/bin/mount -t ext4 -o rw,relatime \"\${LOOP_DEV}\" ${MOUNT_DIR}
        /system/bin/mkdir -p ${MOUNT_DIR}/proc ${MOUNT_DIR}/sys ${MOUNT_DIR}/dev ${MOUNT_DIR}/dev/pts ${MOUNT_DIR}/data/local/tmp
        /system/bin/mount -t proc proc ${MOUNT_DIR}/proc
        /system/bin/mount -t sysfs sysfs ${MOUNT_DIR}/sys
        /system/bin/mount --bind /dev ${MOUNT_DIR}/dev
        /system/bin/mount --bind /dev/pts ${MOUNT_DIR}/dev/pts 2>/dev/null || true
        /system/bin/mount --bind /data/local/tmp ${MOUNT_DIR}/data/local/tmp
        /system/bin/chroot ${MOUNT_DIR} /usr/bin/env -i HOME=/root TERM=xterm-256color \
            PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
            /bin/sh ${CONTAINER_STAGE}/setup-container.sh ${setup_flags} ${CONTAINER_STAGE}
        /system/bin/sync
    "
}

start_daemon_and_configure() {
    echo "[*] Starting stratad via ${REMOTE_MOD_DIR}/service.sh --now..."
    adb_su "nohup /system/bin/sh ${REMOTE_MOD_DIR}/service.sh --now >/dev/null 2>&1 </dev/null &"
    sleep 1.2
    local ready=0
    for _ in $(seq 1 30); do
        if adb_su "${REMOTE_MOD_DIR}/bin/strata daemon ping >/dev/null 2>&1"; then
            ready=1; break
        fi
        sleep 0.5
    done
    if [ "${ready}" -ne 1 ]; then
        echo "[!] Timed out waiting for stratad socket readiness." >&2
        adb_su "/system/bin/cat /data/adb/strata/run/logs/stratad.log 2>/dev/null || true" >&2
        exit 1
    fi
    if [ "${ENABLE_DEBUG_MOUNTS}" -eq 1 ]; then
        echo "[*] Enabling uinput char device and L5 diagnostics read-only mounts..."
        adb_su "${REMOTE_MOD_DIR}/bin/strata config set-char-devs uinput"
        adb_su "${REMOTE_MOD_DIR}/bin/strata config set-l5-diag on"
    fi
    echo "[*] Restarting Strata container to apply configuration..."
    adb_su "${REMOTE_MOD_DIR}/bin/strata stop >/dev/null 2>&1 || true; ${REMOTE_MOD_DIR}/bin/strata start"
    adb_su "${REMOTE_MOD_DIR}/bin/strata status"
}

resolve_serial
resolve_pubkey
echo "[*] Target device  : ${SERIAL}"
echo "[*] Image cap      : ${IMAGE_SIZE}"
echo "[*] SSH pubkey     : ${PUBKEY_PATH:-none (warning: SSH login requires authorized_keys)}"
echo "[*] Debug mounts   : ${ENABLE_DEBUG_MOUNTS}"

check_device_root
if [ "${SKIP_MODULE_INSTALL}" -eq 0 ]; then
    install_module_zip
elif [ "$(adb_su "[ -x ${REMOTE_MOD_DIR}/bin/stratad ] && echo YES || echo NO" | tr -d '\r')" != "YES" ]; then
    echo "[!] ${REMOTE_MOD_DIR}/bin/stratad not found. Install the module and reboot first." >&2
    exit 1
fi

prepare_rootfs_image
provision_rootfs_via_chroot
start_daemon_and_configure

echo ""
echo "[+] Strata device bootstrap completed!"
echo "    USB SSH forward : adb -s ${SERIAL} forward tcp:2222 tcp:2222 && ssh -p 2222 root@127.0.0.1"
echo "    Live verify     : python3 scripts/verify-live.py --adb ${SERIAL}"
