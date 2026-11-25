#!/system/bin/sh
#
# Late-start service executed by KernelSU after boot completion.
# Prepares runtime directories and supervises Rust `stratad` with zero polling.
#

unset ASH_STANDALONE 2>/dev/null || true

MODDIR="${0%/*}"
LOG_TAG="StrataSvc"
STATE_DIR="/data/adb/strata"
RUN_DIR="${STATE_DIR}/run"
LIVE_CONFIG="${STATE_DIR}/config.json"
MODULE_CONFIG="${MODDIR}/config.json"
DAEMON_BIN="${MODDIR}/bin/stratad"
CLI_BIN="${MODDIR}/bin/strata"
KSU_BIN_DIR="/data/adb/ksu/bin"
DAEMON_LOG="${RUN_DIR}/logs/stratad.log"
MAX_LOG_BYTES=5242880
DAEMON_PID=""

GETPROP_BIN="/system/bin/getprop"
MKDIR_BIN="/system/bin/mkdir"
CHOWN_BIN="/system/bin/chown"
CHMOD_BIN="/system/bin/chmod"
CP_BIN="/system/bin/cp"
MV_BIN="/system/bin/mv"
RM_BIN="/system/bin/rm"
LN_BIN="/system/bin/ln"
LOG_BIN="/system/bin/log"
SLEEP_BIN="/system/bin/sleep"
CAT_BIN="/system/bin/cat"
WC_BIN="/system/bin/wc"
PKILL_BIN="/system/bin/pkill"
PGREP_BIN="/system/bin/pgrep"

SKIP_BOOT_WAIT=0
[ "${1:-}" = "--now" ] && SKIP_BOOT_WAIT=1

log_info() {
    "$LOG_BIN" -t "$LOG_TAG" -p i "$1" 2>/dev/null || true
}

rotate_daemon_log() {
    [ -f "$DAEMON_LOG" ] || return 0
    raw_size=$("$WC_BIN" -c < "$DAEMON_LOG" 2>/dev/null || echo 0)
    # Strip any leading whitespace produced by wc -c.
    set -- $raw_size
    log_size="${1:-0}"
    case "$log_size" in
        ''|*[!0-9]*) log_size=0 ;;
    esac
    if [ "$log_size" -ge "$MAX_LOG_BYTES" ]; then
        "$MV_BIN" -f "$DAEMON_LOG" "${DAEMON_LOG}.1" 2>/dev/null || : > "$DAEMON_LOG"
    fi
}

terminate_pid_gracefully() {
    target_pid="$1"
    case "$target_pid" in
        ''|*[!0-9]*) return 0 ;;
    esac
    if kill -0 "$target_pid" 2>/dev/null; then
        kill -TERM "$target_pid" 2>/dev/null || true
        steps=0
        while [ "$steps" -lt 25 ] && kill -0 "$target_pid" 2>/dev/null; do
            "$SLEEP_BIN" 0.2
            steps=$((steps + 1))
        done
        kill -9 "$target_pid" 2>/dev/null || true
    fi
}

stop_existing_daemon() {
    if [ -f "${RUN_DIR}/stratad.pid" ] && [ ! -L "${RUN_DIR}/stratad.pid" ]; then
        old_pid=$("$CAT_BIN" "${RUN_DIR}/stratad.pid" 2>/dev/null || true)
        terminate_pid_gracefully "$old_pid"
        "$RM_BIN" -f "${RUN_DIR}/stratad.pid"
    fi
    if "$PKILL_BIN" -TERM -x "stratad" 2>/dev/null; then
        steps=0
        while [ "$steps" -lt 25 ]; do
            if [ -x "$PGREP_BIN" ]; then
                "$PGREP_BIN" -x "stratad" >/dev/null 2>&1 || break
            else
                "$PKILL_BIN" -0 -x "stratad" 2>/dev/null || break
            fi
            "$SLEEP_BIN" 0.2
            steps=$((steps + 1))
        done
        "$PKILL_BIN" -KILL -x "stratad" 2>/dev/null || true
    fi
}

cleanup_and_exit() {
    log_info "Supervisor received stop signal. Shutting down stratad..."
    terminate_pid_gracefully "$DAEMON_PID"
    "$RM_BIN" -f "${RUN_DIR}/supervisor.pid"
    exit 0
}
trap cleanup_and_exit TERM INT

if [ "$SKIP_BOOT_WAIT" -eq 0 ]; then
    until [ "$("$GETPROP_BIN" sys.boot_completed)" = "1" ]; do
        "$SLEEP_BIN" 2
    done
fi

"$MKDIR_BIN" -p "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs" /data/local/strata/containers/main/mnt
"$CHOWN_BIN" 0:0 "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs"
"$CHMOD_BIN" 0700 "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs"
"$CHMOD_BIN" 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main /data/local/strata/containers/main/mnt 2>/dev/null || true

if [ -d "$KSU_BIN_DIR" ] && [ -x "$CLI_BIN" ]; then
    "$LN_BIN" -sf "$CLI_BIN" "${KSU_BIN_DIR}/strata"
fi

if [ ! -f "$LIVE_CONFIG" ] && [ -f "$MODULE_CONFIG" ]; then
    "$CP_BIN" "$MODULE_CONFIG" "$LIVE_CONFIG"
    "$CHOWN_BIN" 0:0 "$LIVE_CONFIG"
    "$CHMOD_BIN" 0600 "$LIVE_CONFIG"
fi

if [ -x "${MODDIR}/scripts/sync-sideveil-plugin.sh" ]; then
    /system/bin/sh "${MODDIR}/scripts/sync-sideveil-plugin.sh" "$MODDIR" || true
fi

if [ ! -x "$DAEMON_BIN" ]; then
    log_info "Error: ${DAEMON_BIN} not found or not executable."
    exit 1
fi

if [ -f "${RUN_DIR}/supervisor.pid" ] && [ ! -L "${RUN_DIR}/supervisor.pid" ]; then
    old_sup=$("$CAT_BIN" "${RUN_DIR}/supervisor.pid" 2>/dev/null || true)
    if [ -n "$old_sup" ] && [ "$old_sup" != "$$" ]; then
        terminate_pid_gracefully "$old_sup"
    fi
fi

"$RM_BIN" -f "${RUN_DIR}/supervisor.pid"
printf '%s\n' "$$" > "${RUN_DIR}/supervisor.pid"
"$CHMOD_BIN" 0600 "${RUN_DIR}/supervisor.pid"

stop_existing_daemon
log_info "Strata supervisor started (pid=$$)."

while true; do
    if [ -f "${MODDIR}/remove" ]; then
        log_info "Module marked for removal. Stopping stratad and exiting."
        stop_existing_daemon
        break
    fi

    if [ -f "${MODDIR}/disable" ]; then
        stop_existing_daemon
        "$SLEEP_BIN" 5
        continue
    fi

    rotate_daemon_log
    log_info "Launching stratad..."
    "$DAEMON_BIN" \
        --config "$LIVE_CONFIG" \
        --run-dir "$RUN_DIR" \
        --module-dir "$MODDIR" >>"$DAEMON_LOG" 2>&1 &
    DAEMON_PID=$!

    wait "$DAEMON_PID" || true
    DAEMON_PID=""
    "$SLEEP_BIN" 2
done

"$RM_BIN" -f "${RUN_DIR}/supervisor.pid"
