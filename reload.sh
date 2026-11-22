#!/usr/bin/env bash
#
# Zero-reboot development deploy and hot-reload script for Strata.
# Deploys over container SSH (--ssh <host|user@host> or STRATA_SSH_HOST / SSH_TARGET)
# or locally on-device (host-exec / Android root).
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODULE_ID="strata"
REMOTE_MOD_DIR="/data/adb/modules/${MODULE_ID}"
STAGE_DIR="/data/local/tmp/strata_stage"

SKIP_BUILD=0
RESET_CONFIG=0
LOCAL_MODE=0
SSH_TARGET="${SSH_TARGET:-${STRATA_SSH_HOST:-}}"
SSH_PORT="${SSH_PORT:-${STRATA_SSH_PORT:-2222}}"

usage() {
    cat <<'EOF'
Usage: ./reload.sh [options] [ssh-target]

Options:
  --ssh <host|user@host>  Deploy and reload over Strata container SSH
  --ssh-port <port>       Container SSH port (default: $STRATA_SSH_PORT / $SSH_PORT / 2222)
  --local                 Deploy locally on-device via host-exec or Android UID 0
  --skip-build            Skip build.sh quality gate and cross-build
  --reset-config          Overwrite /data/adb/strata/config.json with repo defaults
  -h, --help              Show this help message
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1; shift ;;
        --reset-config) RESET_CONFIG=1; shift ;;
        --local) LOCAL_MODE=1; SSH_TARGET=""; shift ;;
        --ssh) SSH_TARGET="$2"; shift 2 ;;
        --ssh-port) SSH_PORT="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) SSH_TARGET="$1"; shift ;;
    esac
done

if [ "$LOCAL_MODE" -eq 0 ] && [ -z "$SSH_TARGET" ]; then
    if [ -x /usr/local/bin/host-exec ] || { [ -x /system/bin/getprop ] && [ "$(id -u)" -eq 0 ]; }; then
        LOCAL_MODE=1
    else
        echo "[!] Error: Specify --ssh <host|user@host> (or set STRATA_SSH_HOST / SSH_TARGET), or run on-device." >&2
        exit 1
    fi
fi

if [ -n "$SSH_TARGET" ] && [[ "$SSH_TARGET" != *@* ]]; then
    SSH_TARGET="root@${SSH_TARGET}"
fi

MODULE_FILES=(
    module.prop config.json service.sh post-fs-data.sh customize.sh uninstall.sh
    bin/stratad bin/strata
    container/host-exec container/setup-container.sh container/strata-debug
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
    bin/stratad bin/strata container/host-exec container/setup-container.sh \
    scripts/sync-sideveil-plugin.sh; do
    /system/bin/cp "${STAGE_DIR}/\$f" "${REMOTE_MOD_DIR}/\${f}.new"
    /system/bin/mv -f "${REMOTE_MOD_DIR}/\${f}.new" "${REMOTE_MOD_DIR}/\$f"
done
/system/bin/rm -f ${REMOTE_MOD_DIR}/container/adb-shim.py

if [ -d "${STAGE_DIR}/container/strata-debug" ]; then
    /system/bin/rm -rf ${REMOTE_MOD_DIR}/container/strata-debug
    /system/bin/cp -r "${STAGE_DIR}/container/strata-debug" ${REMOTE_MOD_DIR}/container/strata-debug
    /system/bin/rm -rf ${REMOTE_MOD_DIR}/container/strata-debug/__pycache__
    /system/bin/find ${REMOTE_MOD_DIR}/container/strata-debug -type d -exec /system/bin/chmod 0755 {} +
    /system/bin/find ${REMOTE_MOD_DIR}/container/strata-debug -type f -exec /system/bin/chmod 0644 {} +
    /system/bin/chmod 0755 ${REMOTE_MOD_DIR}/container/strata-debug/cli.py ${REMOTE_MOD_DIR}/container/strata-debug/forward_bridge.py 2>/dev/null || true
fi

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
    echo "[*] Deploying via SSH to ${SSH_TARGET}:${SSH_PORT}..."
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
    last_ssh_err=""
    err_file="$(mktemp)"
    trap 'rm -f "${err_file}"' EXIT
    for _ in $(seq 1 25); do
        if ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" "host-exec /data/adb/ksu/bin/strata status" 2>"${err_file}"; then
            rm -f "${err_file}"
            trap - EXIT
            echo "[+] Strata SSH hot-reload succeeded."
            exit 0
        fi
        last_ssh_err="$(cat "${err_file}" 2>/dev/null || true)"
        sleep 1
    done
    rm -f "${err_file}"
    trap - EXIT
    echo "[!] Timed out waiting for stratad after SSH reload (${SSH_TARGET}:${SSH_PORT})." >&2
    if [ -n "${last_ssh_err}" ]; then
        echo "[!] Last SSH error:" >&2
        printf '%s\n' "${last_ssh_err}" >&2
    fi
    echo "[!] Troubleshooting hints:" >&2
    echo "    - Verify device connectivity and SSH port (${SSH_PORT}) with: ssh -p ${SSH_PORT} ${SSH_TARGET} true" >&2
    echo "    - Inspect /data/adb/strata/run/logs/stratad.log on the Android host for startup failures." >&2
    echo "    - Check container/Dropbear status on-device via: /data/adb/ksu/bin/strata status" >&2
    exit 1
fi

echo "[*] Deploying locally on-device..."
rm -rf "${STAGE_DIR}" && mkdir -p "${STAGE_DIR}"
tar -C "${SCRIPT_DIR}" -cf - "${MODULE_FILES[@]}" | tar -xf - -C "${STAGE_DIR}"

if [ -x /usr/local/bin/host-exec ]; then
    install_staged_script | /usr/local/bin/host-exec /system/bin/sh -s
    echo "[*] Scheduling detached host-side stratad restart via host-exec..."
    /usr/local/bin/host-exec /system/bin/sh -c "
        for cg in /sys/fs/cgroup/cgroup.procs /dev/cpuctl/cgroup.procs /dev/cpuset/cgroup.procs; do
            [ -w \"\$cg\" ] && printf \"%s\\n\" \"\$\$\" > \"\$cg\" 2>/dev/null || true
        done
        nohup /system/bin/sh -c \"sleep 1; ${REMOTE_MOD_DIR}/service.sh --now\" >/dev/null 2>&1 </dev/null &
    "
    echo "[+] Strata local restart scheduled (active container session will reconnect after restart)."
    exit 0
fi

install_staged_script | /system/bin/sh -s
nohup /system/bin/sh -c "${REMOTE_MOD_DIR}/service.sh --now" >/dev/null 2>&1 </dev/null &
for _ in $(seq 1 25); do
    if /data/adb/ksu/bin/strata daemon ping >/dev/null 2>&1; then
        /data/adb/ksu/bin/strata status
        echo "[+] Strata local hot-reload completed."
        exit 0
    fi
    sleep 0.3
done

echo "[!] Timed out waiting for stratad socket readiness." >&2
/system/bin/cat /data/adb/strata/run/logs/stratad.log 2>/dev/null || true
exit 1
