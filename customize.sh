#!/system/bin/sh
#
# Module installation and permission setup script for KernelSU / Magisk.
#

SKIPUNZIP=0

ui_print "- Installing Strata..."

if [ "$ARCH" != "arm64" ]; then
    abort "! Unsupported architecture: $ARCH. Strata requires arm64."
fi

ui_print "- Setting module file permissions..."
set_perm "$MODPATH/service.sh" 0 0 0755
set_perm "$MODPATH/post-fs-data.sh" 0 0 0755
set_perm "$MODPATH/uninstall.sh" 0 0 0755
set_perm "$MODPATH/bin/stratad" 0 0 0755
set_perm "$MODPATH/bin/strata" 0 0 0755
set_perm "$MODPATH/container/host-exec" 0 0 0755
set_perm "$MODPATH/container/setup-container.sh" 0 0 0755
set_perm "$MODPATH/scripts/sync-sideveil-plugin.sh" 0 0 0755
set_perm "$MODPATH/module.prop" 0 0 0644
set_perm "$MODPATH/config.json" 0 0 0644
if [ -d "$MODPATH/container/strata-debug" ]; then
    set_perm_recursive "$MODPATH/container/strata-debug" 0 0 0755 0644
    set_perm "$MODPATH/container/strata-debug/cli.py" 0 0 0755
    set_perm "$MODPATH/container/strata-debug/forward_bridge.py" 0 0 0755
fi
if [ -d "$MODPATH/plugins/strata.manager" ]; then
    set_perm_recursive "$MODPATH/plugins/strata.manager" 0 0 0755 0644
fi
if [ -d "$MODPATH/webroot" ]; then
    set_perm_recursive "$MODPATH/webroot" 0 0 0755 0644
fi

STATE_DIR="/data/adb/strata"
RUN_DIR="${STATE_DIR}/run"
DEFAULT_MNT="/data/local/strata/containers/main/mnt"
/system/bin/mkdir -p "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs" "$DEFAULT_MNT"
/system/bin/chown 0:0 "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs"
/system/bin/chmod 0700 "$STATE_DIR" "$RUN_DIR" "${RUN_DIR}/logs"
/system/bin/chmod 0755 /data/local/strata /data/local/strata/containers /data/local/strata/containers/main "$DEFAULT_MNT"

if [ ! -f "${STATE_DIR}/config.json" ] && [ -f "${MODPATH}/config.json" ]; then
    /system/bin/cp "${MODPATH}/config.json" "${STATE_DIR}/config.json"
    /system/bin/chown 0:0 "${STATE_DIR}/config.json"
    /system/bin/chmod 0600 "${STATE_DIR}/config.json"
    ui_print "- Seeded default config to ${STATE_DIR}/config.json"
else
    ui_print "- Preserved existing ${STATE_DIR}/config.json"
fi

if [ -d /data/adb/ksu/bin ]; then
    /system/bin/ln -sf /data/adb/modules/strata/bin/strata /data/adb/ksu/bin/strata
    ui_print "- Linked /data/adb/ksu/bin/strata"
fi

if [ -x "$MODPATH/scripts/sync-sideveil-plugin.sh" ] && [ -d /data/system/sideveil/plugins ]; then
    /system/bin/sh "$MODPATH/scripts/sync-sideveil-plugin.sh" "$MODPATH"
    ui_print "- Synced SideVeil plugin package (strata.manager)"
fi

ui_print "- Strata installation completed."
