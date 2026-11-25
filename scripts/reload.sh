#!/usr/bin/env bash
#
# Zero-reboot development deploy and hot-reload script for Strata.
# Deploys over container SSH (--ssh <host|user@host> or STRATA_SSH_HOST / SSH_TARGET),
# over ADB (--adb <serial> or STRATA_ADB_SERIAL), or locally on-device (host-exec / Android root).
#

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODULE_ID="strata"
REMOTE_MOD_DIR="/data/adb/modules/${MODULE_ID}"
STAGE_DIR="/data/local/tmp/strata_stage"

SKIP_BUILD=0
RESET_CONFIG=0
LOCAL_MODE=0
SSH_TARGET="${SSH_TARGET:-${STRATA_SSH_HOST:-}}"
SSH_PORT="${SSH_PORT:-${STRATA_SSH_PORT:-2222}}"
ADB_SERIAL=""
ADB_LOCAL_PORT=""
ADB_SSH_ISOLATED=0
ERR_FILE=""
LOCAL_STAGE_DIR=""

cleanup() {
    [ -n "${ERR_FILE}" ] && rm -f "${ERR_FILE}" 2>/dev/null || true
    [ -n "${LOCAL_STAGE_DIR}" ] && rm -rf "${LOCAL_STAGE_DIR}" 2>/dev/null || true
    if [ -n "${ADB_SERIAL}" ] && [ -n "${ADB_LOCAL_PORT}" ]; then
        adb -s "${ADB_SERIAL}" forward --remove "tcp:${ADB_LOCAL_PORT}" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

usage() {
    cat <<'EOF'
Usage: bash scripts/reload.sh [options] [ssh-target]

Options:
  --ssh <host|user@host>  Deploy and reload over Strata container SSH
  --ssh-port <port>       Container SSH port (default: $STRATA_SSH_PORT / $SSH_PORT / 2222)
  --adb <serial>          Deploy via ADB serial (temp local SSH forward or su fallback;
                          default env: $STRATA_ADB_SERIAL)
  --local                 Deploy locally on-device via host-exec or Android UID 0
  --skip-build            Skip scripts/build.sh quality gate and cross-build
  --reset-config          Overwrite /data/adb/strata/config.json with repo defaults
  -h, --help              Show this help message
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --skip-build) SKIP_BUILD=1; shift ;;
        --reset-config) RESET_CONFIG=1; shift ;;
        --local) LOCAL_MODE=1; SSH_TARGET=""; ADB_SERIAL=""; shift ;;
        --ssh)
            [ "$#" -ge 2 ] || { echo "[!] Option $1 requires a value." >&2; usage >&2; exit 2; }
            SSH_TARGET="$2"; ADB_SERIAL=""; shift 2
            ;;
        --ssh-port)
            [ "$#" -ge 2 ] || { echo "[!] Option $1 requires a value." >&2; usage >&2; exit 2; }
            SSH_PORT="$2"; shift 2
            ;;
        --adb)
            [ "$#" -ge 2 ] || { echo "[!] Option $1 requires a value." >&2; usage >&2; exit 2; }
            ADB_SERIAL="$2"; SSH_TARGET=""; LOCAL_MODE=0; shift 2
            ;;
        -h|--help) usage; exit 0 ;;
        *) SSH_TARGET="$1"; ADB_SERIAL=""; shift ;;
    esac
done

if [ "$LOCAL_MODE" -eq 0 ] && [ -z "$SSH_TARGET" ] && [ -z "$ADB_SERIAL" ]; then
    if [ -n "${STRATA_ADB_SERIAL:-}" ]; then
        ADB_SERIAL="${STRATA_ADB_SERIAL}"
    elif [ -x /usr/local/bin/host-exec ] || { [ -x /system/bin/getprop ] && [ "$(id -u)" -eq 0 ]; }; then
        LOCAL_MODE=1
    else
        echo "[!] Error: Specify --ssh <host|user@host>, --adb <serial> (or STRATA_SSH_HOST / STRATA_ADB_SERIAL), or run on-device." >&2
        exit 1
    fi
fi

if [ -n "$SSH_TARGET" ] && [[ "$SSH_TARGET" != *@* ]]; then
    SSH_TARGET="root@${SSH_TARGET}"
fi

if [ "$SKIP_BUILD" -eq 0 ]; then
    bash "${ROOT_DIR}/scripts/build.sh"
fi

prepare_local_stage() {
    LOCAL_STAGE_DIR="$(mktemp -d)"
    mkdir -p \
        "${LOCAL_STAGE_DIR}/bin" \
        "${LOCAL_STAGE_DIR}/container" \
        "${LOCAL_STAGE_DIR}/scripts" \
        "${LOCAL_STAGE_DIR}/plugins/strata.manager"

    for f in module.prop config.json service.sh post-fs-data.sh customize.sh uninstall.sh; do
        cp "${ROOT_DIR}/module/${f}" "${LOCAL_STAGE_DIR}/${f}"
    done
    for b in stratad strata strata-debug; do
        cp "${ROOT_DIR}/out/stage/bin/${b}" "${LOCAL_STAGE_DIR}/bin/${b}"
    done
    cp "${ROOT_DIR}/module/container/host-exec" "${LOCAL_STAGE_DIR}/container/host-exec"
    cp "${ROOT_DIR}/module/container/setup-container.sh" "${LOCAL_STAGE_DIR}/container/setup-container.sh"
    cp "${ROOT_DIR}/scripts/sync-sideveil-plugin.sh" "${LOCAL_STAGE_DIR}/scripts/sync-sideveil-plugin.sh"
    cp "${ROOT_DIR}/module/plugins/strata.manager/manifest.json" "${LOCAL_STAGE_DIR}/plugins/strata.manager/manifest.json"
    cp -r "${ROOT_DIR}/out/stage/plugins/strata.manager/web" "${LOCAL_STAGE_DIR}/plugins/strata.manager/web"
    cp -r "${ROOT_DIR}/out/stage/webroot" "${LOCAL_STAGE_DIR}/webroot"
}

install_staged_script() {
    cat <<EOF
set -e
/system/bin/mkdir -p ${REMOTE_MOD_DIR}/bin ${REMOTE_MOD_DIR}/container ${REMOTE_MOD_DIR}/scripts ${REMOTE_MOD_DIR}/plugins /data/adb/${MODULE_ID}/run/logs /data/local/strata/containers/main/mnt
/system/bin/chown 0:0 /data/adb/${MODULE_ID} /data/adb/${MODULE_ID}/run /data/adb/${MODULE_ID}/run/logs
/system/bin/chmod 0700 /data/adb/${MODULE_ID} /data/adb/${MODULE_ID}/run /data/adb/${MODULE_ID}/run/logs
/system/bin/chmod 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main /data/local/strata/containers/main/mnt

for f in module.prop config.json service.sh post-fs-data.sh customize.sh uninstall.sh \
    bin/stratad bin/strata bin/strata-debug container/host-exec container/setup-container.sh \
    scripts/sync-sideveil-plugin.sh; do
    /system/bin/cp "${STAGE_DIR}/\$f" "${REMOTE_MOD_DIR}/\${f}.new"
    /system/bin/mv -f "${REMOTE_MOD_DIR}/\${f}.new" "${REMOTE_MOD_DIR}/\$f"
done
/system/bin/rm -rf ${REMOTE_MOD_DIR}/container/adb-shim.py ${REMOTE_MOD_DIR}/container/strata-debug

swap_dir_atomic() {
    src_dir="\$1"
    dst_dir="\$2"
    tmp_dir="\${dst_dir}.new.\$\$"
    old_dir="\${dst_dir}.old.\$\$"
    /system/bin/rm -rf "\$tmp_dir" "\$old_dir"
    /system/bin/cp -r "\$src_dir" "\$tmp_dir"
    /system/bin/find "\$tmp_dir" -type d -exec /system/bin/chmod 0755 {} +
    /system/bin/find "\$tmp_dir" -type f -exec /system/bin/chmod 0644 {} +
    [ -d "\$dst_dir" ] && /system/bin/mv "\$dst_dir" "\$old_dir"
    if ! /system/bin/mv "\$tmp_dir" "\$dst_dir"; then
        [ -d "\$old_dir" ] && [ ! -d "\$dst_dir" ] && /system/bin/mv "\$old_dir" "\$dst_dir" 2>/dev/null || true
        /system/bin/rm -rf "\$tmp_dir"
        return 1
    fi
    /system/bin/rm -rf "\$old_dir"
}

[ -d "${STAGE_DIR}/plugins/strata.manager" ] && swap_dir_atomic "${STAGE_DIR}/plugins/strata.manager" "${REMOTE_MOD_DIR}/plugins/strata.manager"
[ -d "${STAGE_DIR}/webroot" ] && swap_dir_atomic "${STAGE_DIR}/webroot" "${REMOTE_MOD_DIR}/webroot"

if [ "${RESET_CONFIG}" -eq 1 ] || [ ! -f /data/adb/${MODULE_ID}/config.json ]; then
    /system/bin/cp "${STAGE_DIR}/config.json" /data/adb/${MODULE_ID}/config.json
    /system/bin/chown 0:0 /data/adb/${MODULE_ID}/config.json
    /system/bin/chmod 0600 /data/adb/${MODULE_ID}/config.json
fi

/system/bin/chmod 0755 \
    ${REMOTE_MOD_DIR}/service.sh ${REMOTE_MOD_DIR}/post-fs-data.sh \
    ${REMOTE_MOD_DIR}/customize.sh ${REMOTE_MOD_DIR}/uninstall.sh \
    ${REMOTE_MOD_DIR}/bin/stratad ${REMOTE_MOD_DIR}/bin/strata ${REMOTE_MOD_DIR}/bin/strata-debug \
    ${REMOTE_MOD_DIR}/container/host-exec ${REMOTE_MOD_DIR}/container/setup-container.sh \
    ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh
/system/bin/chmod 0644 ${REMOTE_MOD_DIR}/module.prop ${REMOTE_MOD_DIR}/config.json

for mnt in /data/local/strata/containers/*/mnt; do
    if [ -d "\${mnt}/usr/local/bin" ]; then
        /system/bin/cp "${REMOTE_MOD_DIR}/bin/strata-debug" "\${mnt}/usr/local/bin/strata-debug.new"
        /system/bin/chmod 0755 "\${mnt}/usr/local/bin/strata-debug.new"
        /system/bin/mv -f "\${mnt}/usr/local/bin/strata-debug.new" "\${mnt}/usr/local/bin/strata-debug"
        /system/bin/rm -rf "\${mnt}/usr/local/lib/strata-debug"
    fi
done

[ -d /data/adb/ksu/bin ] && /system/bin/ln -sf ${REMOTE_MOD_DIR}/bin/strata /data/adb/ksu/bin/strata
/system/bin/sh ${REMOTE_MOD_DIR}/scripts/sync-sideveil-plugin.sh ${REMOTE_MOD_DIR} || true
/system/bin/rm -rf "${STAGE_DIR}"
EOF
}

prepare_local_stage

if [ -n "$ADB_SERIAL" ]; then
    ADB_LOCAL_PORT="$(adb -s "${ADB_SERIAL}" forward tcp:0 "tcp:${SSH_PORT}" 2>/dev/null | tr -d '\r\n' || true)"
    if [[ "${ADB_LOCAL_PORT}" =~ ^[0-9]+$ ]] && ssh \
        -p "${ADB_LOCAL_PORT}" -o BatchMode=yes -o ConnectTimeout=4 \
        -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
        root@127.0.0.1 true >/dev/null 2>&1; then
        echo "[*] Using isolated ADB forward (${ADB_SERIAL} -> 127.0.0.1:${ADB_LOCAL_PORT})..."
        SSH_TARGET="root@127.0.0.1"
        SSH_PORT="${ADB_LOCAL_PORT}"
        ADB_SSH_ISOLATED=1
    else
        echo "[*] Container SSH unavailable over ADB forward; deploying via adb -s ${ADB_SERIAL} su..."
        STAGE_TAR="/data/local/tmp/strata_stage.tar"
        LOCAL_TAR="$(mktemp)"
        ERR_FILE="${LOCAL_TAR}"
        tar -C "${LOCAL_STAGE_DIR}" -cf "${LOCAL_TAR}" .
        adb -s "${ADB_SERIAL}" push "${LOCAL_TAR}" "${STAGE_TAR}" >/dev/null
        adb -s "${ADB_SERIAL}" shell "su -c 'set -e; rm -rf ${STAGE_DIR}; mkdir -p ${STAGE_DIR}; tar -xf ${STAGE_TAR} -C ${STAGE_DIR}; rm -f ${STAGE_TAR}'"
        install_staged_script | adb -s "${ADB_SERIAL}" shell "su -c '/system/bin/sh -s'"
        echo "[*] Restarting stratad on ${ADB_SERIAL}..."
        adb -s "${ADB_SERIAL}" shell "su -c 'nohup /system/bin/sh ${REMOTE_MOD_DIR}/service.sh --now >/dev/null 2>&1 </dev/null &'"
        for _ in $(seq 1 25); do
            if adb -s "${ADB_SERIAL}" shell "su -c '/data/adb/ksu/bin/strata daemon ping'" >/dev/null 2>&1; then
                adb -s "${ADB_SERIAL}" shell "su -c '/data/adb/ksu/bin/strata status'"
                echo "[+] Strata ADB hot-reload succeeded (${ADB_SERIAL})."
                exit 0
            fi
            sleep 0.5
        done
        echo "[!] Timed out waiting for stratad after ADB reload (${ADB_SERIAL})." >&2
        adb -s "${ADB_SERIAL}" shell "su -c '/system/bin/cat /data/adb/strata/run/logs/stratad.log 2>/dev/null || true'" >&2
        exit 1
    fi
fi

if [ -n "$SSH_TARGET" ]; then
    SSH_OPTS=(-p "${SSH_PORT}" -o BatchMode=yes -o ConnectTimeout=6)
    if [ "${ADB_SSH_ISOLATED}" -eq 1 ]; then
        SSH_OPTS+=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
    else
        SSH_OPTS+=(-o StrictHostKeyChecking=accept-new)
    fi
    echo "[*] Deploying via SSH to ${SSH_TARGET}:${SSH_PORT}..."
    tar -C "${LOCAL_STAGE_DIR}" -cf - . \
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
    ERR_FILE="$(mktemp)"
    for _ in $(seq 1 25); do
        if ssh "${SSH_OPTS[@]}" "${SSH_TARGET}" "host-exec /data/adb/ksu/bin/strata status" 2>"${ERR_FILE}"; then
            echo "[+] Strata SSH hot-reload succeeded."
            exit 0
        fi
        last_ssh_err="$(cat "${ERR_FILE}" 2>/dev/null || true)"
        sleep 1
    done
    echo "[!] Timed out waiting for stratad after SSH reload (${SSH_TARGET}:${SSH_PORT})." >&2
    [ -n "${last_ssh_err}" ] && printf '[!] Last SSH error:\n%s\n' "${last_ssh_err}" >&2
    exit 1
fi

echo "[*] Deploying locally on-device..."
rm -rf "${STAGE_DIR}" && mkdir -p "${STAGE_DIR}"
tar -C "${LOCAL_STAGE_DIR}" -cf - . | tar -xf - -C "${STAGE_DIR}"

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
