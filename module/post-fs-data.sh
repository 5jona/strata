#!/system/bin/sh
#
# Early-boot post-fs-data hook for Strata KernelSU module.
# Prepares root-only state directories and CLI symlink before late_start services.
#

unset ASH_STANDALONE 2>/dev/null || true

MODDIR="${0%/*}"
STATE_DIR="/data/adb/strata"
RUN_DIR="${STATE_DIR}/run"
CONTAINERS_DIR="/data/local/strata/containers"
KSU_BIN_DIR="/data/adb/ksu/bin"

/system/bin/mkdir -p "$RUN_DIR" "$CONTAINERS_DIR"
/system/bin/chown 0:0 "$STATE_DIR" "$RUN_DIR" 2>/dev/null || true
/system/bin/chmod 0700 "$STATE_DIR" "$RUN_DIR" 2>/dev/null || true
/system/bin/chmod 0755 /data/local/strata "$CONTAINERS_DIR" 2>/dev/null || true

if [ -d "$KSU_BIN_DIR" ] && [ -x "${MODDIR}/bin/strata" ]; then
    /system/bin/ln -sf "${MODDIR}/bin/strata" "${KSU_BIN_DIR}/strata"
fi
