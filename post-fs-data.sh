#!/system/bin/sh
#
# Early-boot post-fs-data hook for Strata KernelSU module.
# Prepares root-only state directories and CLI symlink before late_start services.
#

unset ASH_STANDALONE 2>/dev/null || true

MODDIR="${0%/*}"
STATE_DIR="/data/adb/strata"
RUN_DIR="${STATE_DIR}/run"
DEFAULT_MNT="/data/local/strata/containers/main/mnt"
KSU_BIN_DIR="/data/adb/ksu/bin"

/system/bin/mkdir -p "$RUN_DIR" "$DEFAULT_MNT"
/system/bin/chown 0:0 "$STATE_DIR" "$RUN_DIR" 2>/dev/null || true
/system/bin/chmod 0700 "$STATE_DIR" "$RUN_DIR" 2>/dev/null || true
/system/bin/chmod 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main "$DEFAULT_MNT" 2>/dev/null || true

if [ -d "$KSU_BIN_DIR" ] && [ -x "${MODDIR}/bin/strata" ]; then
    /system/bin/ln -sf "${MODDIR}/bin/strata" "${KSU_BIN_DIR}/strata"
fi
