#!/system/bin/sh
#
# Cleanup script executed when the Strata KernelSU module is removed.
# Stops all containers cleanly with zero residual mounts/loops while preserving user rootfs images.
#

unset ASH_STANDALONE 2>/dev/null || true

RUN_DIR="/data/adb/strata/run"
STRATA_BIN="/data/adb/ksu/bin/strata"

terminate_pid() {
    target_pid="$1"
    case "$target_pid" in
        ''|*[!0-9]*) return 0 ;;
    esac
    if kill -0 "$target_pid" 2>/dev/null; then
        kill -TERM "$target_pid" 2>/dev/null || true
        steps=0
        while [ "$steps" -lt 25 ] && kill -0 "$target_pid" 2>/dev/null; do
            /system/bin/sleep 0.2
            steps=$((steps + 1))
        done
        kill -9 "$target_pid" 2>/dev/null || true
    fi
}

stop_all_containers() {
    [ -x "$STRATA_BIN" ] || return 0
    containers_out=$("$STRATA_BIN" list 2>/dev/null || true)
    if [ -n "$containers_out" ]; then
        printf '%s\n' "$containers_out" | while IFS=' ' read -r raw_id phase _pid _port mounted _rest; do
            case "$raw_id" in
                ''|CONTAINER) continue ;;
            esac
            cid="${raw_id%\*}"
            if [ "$phase" != "stopped" ] || [ "$mounted" = "yes" ]; then
                "$STRATA_BIN" -c "$cid" stop >/dev/null 2>&1 || true
            fi
        done
    else
        "$STRATA_BIN" stop >/dev/null 2>&1 || true
    fi
}

stop_all_containers

for pidfile in "${RUN_DIR}/supervisor.pid" "${RUN_DIR}/stratad.pid"; do
    if [ -f "$pidfile" ] && [ ! -L "$pidfile" ]; then
        p=$(/system/bin/cat "$pidfile" 2>/dev/null || true)
        terminate_pid "$p"
    fi
done

if /system/bin/pkill -TERM -x "stratad" 2>/dev/null; then
    steps=0
    while [ "$steps" -lt 25 ]; do
        if [ -x /system/bin/pgrep ]; then
            /system/bin/pgrep -x "stratad" >/dev/null 2>&1 || break
        else
            /system/bin/pkill -0 -x "stratad" 2>/dev/null || break
        fi
        /system/bin/sleep 0.2
        steps=$((steps + 1))
    done
    /system/bin/pkill -KILL -x "stratad" 2>/dev/null || true
fi

/system/bin/rm -f "$STRATA_BIN"
/system/bin/rm -rf "$RUN_DIR"
/system/bin/rmdir /sys/fs/cgroup/strata/* /sys/fs/cgroup/strata 2>/dev/null || true

if [ -d /data/system/sideveil/plugins/strata.manager ]; then
    /system/bin/rm -rf /data/system/sideveil/plugins/strata.manager
    if [ -x /data/adb/modules/sideveil/bin/sideveil-ctl ]; then
        /data/adb/modules/sideveil/bin/sideveil-ctl plugins reload >/dev/null 2>&1 || true
    fi
fi
