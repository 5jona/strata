#!/usr/bin/env bash
#
# Zero-reboot development deploy and hot-reload script for Strata.
# Supports both ADB deployment and ADB-free container SSH deployment (--ssh root@<host>).
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE_ID="strata"
REMOTE_MOD_DIR="/data/adb/modules/${MODULE_ID}"
STAGE_DIR="/data/local/tmp/strata_stage"

SKIP_BUILD=0
RESET_CONFIG=0
TARGET_SERIAL="${TARGET_SERIAL:-}"
SSH_TARGET="${SSH_TARGET:-}"
SSH_PORT="${SSH_PORT:-2222}"

usage() {
    cat <<'EOF'
Usage: ./reload.sh [options] [adb-serial]

Options:
  --ssh <host|user@host>  Deploy and reload over container SSH without ADB
  --ssh-port <port>       Container SSH port for --ssh mode (default: 2222)
  --skip-build            Skip build.sh quality gate and cross-build
  --reset-config          Overwrite /data/adb/strata/config.json with repo defaults
  -h, --help              Show this help message
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1; shift ;;
        --reset-config) RESET_CONFIG=1; shift ;;
        --ssh) SSH_TARGET="$2"; shift 2 ;;
        --ssh-port) SSH_PORT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) TARGET_SERIAL="$1"; shift ;;
    esac
done

if [ -n "$SSH_TARGET" ] && [[ "$SSH_TARGET" != *@* ]]; then
    SSH_TARGET="root@${SSH_TARGET}"
fi

MODULE_FILES=(
    module.prop config.json service.sh post-fs-data.sh customize.sh uninstall.sh
    bin/stratad bin/strata
    container/host-exec container/adb-shim.py container/setup-container.sh
    scripts/sync-sideveil-plugin.sh
    plugins/strata.manager
)

if [ "$SKIP_BUILD" -eq 0 ]; then
    bash "${SCRIPT_DIR}/build.sh"
fi

install_staged_script() {
    cat <<EOF
set -e
/system/bin/mkdir -p ${REMOTE_MOD_DIR}/bin ${REMOTE_MOD_DIR}/container ${REMOTE_MOD_DIR}/scripts ${REMOTE_MOD_DIR}/plugins /data/adb/${MODULE_ID}/run/logs /data/local/strata/containers/main/mnt
/system/bin/chown 0:0 /data/adb/${MODULE_ID} /data/adb/${MODULE_ID}/run /data/adb/${MODULE_ID}/run/logs
/system/bin/chmod 0700 /data/adb/${MODULE_ID} /data/adb/${MODULE_ID}/run /data/adb/${MODULE_ID}/run/logs
/system/bin/chmod 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main /data/local/strata/containers/main/mnt

for f in module.prop config.json service.sh post-fs-data.sh customize.sh uninstall.sh \
    bin/stratad bin/strata container/host-exec container/adb-shim.py container/setup-container.sh \
    scripts/sync-sideveil-plugin.sh; do
    /system/bin/cp "${STAGE_DIR}/\$f" "${REMOTE_MOD_DIR}/\$f"
done

if [ -d "${STAGE_DIR}/plugins/strata.manager" ]; then
    /system/bin/rm -rf ${REMOTE_MOD_DIR}/plugins/strata.manager
    /system/bin/cp -r "${STAGE_DIR}/plugins/strata.manager" ${REMOTE_MOD_DIR}/plugins/strata.manager
    /system/bin/find ${REMOTE_MOD_DIR}/plugins/strata.manager -type d -exec /system/bin/chmod 0755 {} +
    /system/bin/find ${REMOTE_MOD_DIR}/plugins/strata.manager -type f -exec /system/bin/chmod 0644 {} +
fi

if [ "${RESET_CONFIG}" -eq 1 ] || [ ! -f /data/adb/${MODULE_ID}/config.json ]; then
    /system/bin/cp "${STAGE_DIR}/config.json" /data/adb/${MODULE_ID}/config.json
    /system/bin/chown 0:0 /data/adb/${MODULE_ID}/config.json
    /system/bin/chmod 0600 /data/adb/${MODULE_ID}/config.json
fi

/system/bin/chmod 0755 \
    ${REMOTE_MOD_DIR}/service.sh \
    ${REMOTE_MOD_DIR}/post-fs-data.sh \
    ${REMOTE_MOD_DIR}/customize.sh \
    ${REMOTE_MOD_DIR}/uninstall.sh \
    ${REMOTE_MOD_DIR}/bin/stratad \
    ${REMOTE_MOD_DIR}/bin/strata \
    ${REMOTE_MOD_DIR}/container/host-exec \
    ${REMOTE_MOD_DIR}/container/adb-shim.py \
    ${REMOTE_MOD_DIR}/container/setup-container.sh \
    ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh
/system/bin/chmod 0644 ${REMOTE_MOD_DIR}/module.prop ${REMOTE_MOD_DIR}/config.json

if [ -d /data/adb/ksu/bin ]; then
    /system/bin/ln -sf ${REMOTE_MOD_DIR}/bin/strata /data/adb/ksu/bin/strata
fi
/system/bin/sh ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh ${REMOTE_MOD_DIR} || true
/system/bin/rm -rf "${STAGE_DIR}"
EOF
}

if [ -n "$SSH_TARGET" ]; then
    SSH_OPTS=(-p "${SSH_PORT}" -o BatchMode=yes -o ConnectTimeout=6 -o StrictHostKeyChecking=accept-new)
    echo "[*] Deploying via ADB-free SSH to ${SSH_TARGET}:${SSH_PORT}..."
    tar -C "${SCRIPT_DIR}" -cf - "${MODULE_FILES[@]}" \
        | ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" \
            "rm -rf ${STAGE_DIR} && mkdir -p ${STAGE_DIR} && tar -xf - -C ${STAGE_DIR}"
    install_staged_script | ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" "host-exec /system/bin/sh -s"

    echo "[*] Scheduling detached host-side stratad restart..."
    ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" "host-exec /system/bin/sh -c '
        for cg in /sys/fs/cgroup/cgroup.procs /dev/cpuctl/cgroup.procs /dev/cpuset/cgroup.procs; do
            [ -w \"\$cg\" ] && printf \"%s\\n\" \"\$\$\" > \"\$cg\" 2>/dev/null || true
        done
        nohup /system/bin/sh -c \"sleep 1; ${REMOTE_MOD_DIR}/service.sh --now\" >/dev/null 2>&1 </dev/null &
    '"
    sleep 3
    for _ in $(seq 1 20); do
        if ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" "host-exec /data/adb/ksu/bin/strata status" 2>/dev/null; then
            echo "[+] ADB-free SSH hot-reload succeeded."
            exit 0
        fi
        sleep 1
    done
    echo "[!] Timed out waiting for stratad after SSH reload." >&2
    exit 1
fi

if [ -z "$TARGET_SERIAL" ]; then
    DETECTED=$(adb devices | awk 'NR>1 && $2=="device" {print $1}')
    COUNT=$(printf '%s\n' "$DETECTED" | sed '/^$/d' | wc -l | tr -d ' ')
    if [ "$COUNT" -eq 1 ]; then
        TARGET_SERIAL="$DETECTED"
    else
        echo "[!] Error: Specify --ssh <user@host> or an ADB serial." >&2
        exit 1
    fi
fi

echo "[*] Target device (ADB): ${TARGET_SERIAL}"
echo "[*] Stopping existing Strata supervisor/daemon if running..."
adb -s "${TARGET_SERIAL}" shell "su -c '
    if [ -x /data/adb/ksu/bin/strata ]; then
        /data/adb/ksu/bin/strata stop >/dev/null 2>&1 || true
    fi
    for pf in /data/adb/strata/run/supervisor.pid /data/adb/strata/run/stratad.pid; do
        if [ -f \"\$pf\" ]; then
            p=\$(/system/bin/cat \"\$pf\" 2>/dev/null || true)
            [ -n \"\$p\" ] && kill -TERM \"\$p\" 2>/dev/null || true
        fi
    done
    /system/bin/sleep 0.5
    /system/bin/pkill -KILL -x stratad 2>/dev/null || true
'"

echo "[*] Pushing module files to ${STAGE_DIR}..."
adb -s "${TARGET_SERIAL}" shell "rm -rf ${STAGE_DIR} && mkdir -p ${STAGE_DIR}/bin ${STAGE_DIR}/container ${STAGE_DIR}/scripts ${STAGE_DIR}/plugins"
adb -s "${TARGET_SERIAL}" push \
    "${SCRIPT_DIR}/module.prop" "${SCRIPT_DIR}/config.json" \
    "${SCRIPT_DIR}/service.sh" "${SCRIPT_DIR}/post-fs-data.sh" \
    "${SCRIPT_DIR}/customize.sh" "${SCRIPT_DIR}/uninstall.sh" \
    "${STAGE_DIR}/" >/dev/null
adb -s "${TARGET_SERIAL}" push \
    "${SCRIPT_DIR}/bin/stratad" "${SCRIPT_DIR}/bin/strata" \
    "${STAGE_DIR}/bin/" >/dev/null
adb -s "${TARGET_SERIAL}" push \
    "${SCRIPT_DIR}/container/host-exec" "${SCRIPT_DIR}/container/adb-shim.py" \
    "${SCRIPT_DIR}/container/setup-container.sh" \
    "${STAGE_DIR}/container/" >/dev/null
adb -s "${TARGET_SERIAL}" push \
    "${SCRIPT_DIR}/scripts/sync-sideveil-plugin.sh" \
    "${STAGE_DIR}/scripts/" >/dev/null
adb -s "${TARGET_SERIAL}" push \
    "${SCRIPT_DIR}/plugins/strata.manager" \
    "${STAGE_DIR}/plugins/" >/dev/null

echo "[*] Installing staged files to ${REMOTE_MOD_DIR}..."
install_staged_script | adb -s "${TARGET_SERIAL}" shell "su"

echo "[*] Launching Strata service..."
adb -s "${TARGET_SERIAL}" shell "su -c 'nohup ${REMOTE_MOD_DIR}/service.sh --now >/dev/null 2>&1 </dev/null &'"

for _ in $(seq 1 25); do
    if adb -s "${TARGET_SERIAL}" shell "su -c '/data/adb/ksu/bin/strata daemon ping'" >/dev/null 2>&1; then
        adb -s "${TARGET_SERIAL}" shell "su -c '/data/adb/ksu/bin/strata status'"
        echo "[+] Strata hot-reload completed."
        exit 0
    fi
    sleep 0.3
done

echo "[!] Timed out waiting for stratad socket readiness." >&2
adb -s "${TARGET_SERIAL}" shell "su -c '/system/bin/cat /data/adb/strata/run/logs/stratad.log 2>/dev/null || true'"
exit 1
