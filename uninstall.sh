#!/system/bin/sh
#
# Cleanup script executed when the Strata KernelSU module is removed.
# Stops containers cleanly with zero residual mounts/loops while preserving user rootfs images.
#

unset ASH_STANDALONE 2>/dev/null || true

RUN_DIR="/data/adb/strata/run"

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

if [ -x /data/adb/ksu/bin/strata ]; then
    /data/adb/ksu/bin/strata stop >/dev/null 2>&1 || true
fi

for pidfile in "${RUN_DIR}/supervisor.pid" "${RUN_DIR}/stratad.pid"; do
    if [ -f "$pidfile" ] && [ ! -L "$pidfile" ]; then
        p=$(/system/bin/cat "$pidfile" 2>/dev/null || true)
        terminate_pid "$p"
    fi
done

/system/bin/pkill -TERM -x "stratad" 2>/dev/null || true
/system/bin/sleep 0.4
/system/bin/pkill -KILL -x "stratad" 2>/dev/null || true

/system/bin/rm -f /data/adb/ksu/bin/strata
/system/bin/rm -rf "$RUN_DIR"
/system/bin/rmdir /sys/fs/cgroup/strata/* /sys/fs/cgroup/strata 2>/dev/null || true

if [ -d /data/system/sideveil/plugins/strata.manager ]; then
    /system/bin/rm -rf /data/system/sideveil/plugins/strata.manager
    if [ -x /data/adb/modules/sideveil/bin/sideveil-ctl ]; then
        /data/adb/modules/sideveil/bin/sideveil-ctl plugins reload >/dev/null 2>&1 || true
    fi
fi
