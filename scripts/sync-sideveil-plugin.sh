#!/system/bin/sh
#
# Atomically mirrors Strata's bundled SideVeil plugin package (`strata.manager`)
# into `/data/system/sideveil/plugins/strata.manager` when SideVeil is installed.
# Silently no-ops when SideVeil is absent so Strata remains 100% standalone.
#

unset ASH_STANDALONE 2>/dev/null || true

MODDIR="${1:-${0%/*}/..}"
PLUGIN_ID="strata.manager"
SRC_DIR="${MODDIR}/plugins/${PLUGIN_ID}"
SV_PLUGINS_DIR="/data/system/sideveil/plugins"
DST_DIR="${SV_PLUGINS_DIR}/${PLUGIN_ID}"
STAGE_DIR="${SV_PLUGINS_DIR}/.${PLUGIN_ID}.staging"
BACKUP_DIR="${SV_PLUGINS_DIR}/.${PLUGIN_ID}.old"
SV_CTL="/data/adb/modules/sideveil/bin/sideveil-ctl"
SV_HUB_SOCK="/data/system/sideveil/run/hub.sock"

if [ ! -d "$SV_PLUGINS_DIR" ]; then
    exit 0
fi

if [ ! -f "${SRC_DIR}/manifest.json" ] || [ ! -f "${SRC_DIR}/web/index.html" ]; then
    exit 0
fi

if [ -x "$SV_CTL" ] && [ -S "$SV_HUB_SOCK" ]; then
    if "$SV_CTL" plugins install "$SRC_DIR" >/dev/null 2>&1; then
        exit 0
    fi
fi

/system/bin/rm -rf "$STAGE_DIR" "$BACKUP_DIR"
/system/bin/mkdir -p "$STAGE_DIR"
/system/bin/cp -r "${SRC_DIR}/." "${STAGE_DIR}/"
/system/bin/chown -R 0:1000 "$STAGE_DIR" 2>/dev/null || true
/system/bin/find "$STAGE_DIR" -type d -exec /system/bin/chmod 0755 {} + 2>/dev/null || true
/system/bin/find "$STAGE_DIR" -type f -exec /system/bin/chmod 0644 {} + 2>/dev/null || true

if [ -d "$DST_DIR" ]; then
    /system/bin/mv "$DST_DIR" "$BACKUP_DIR"
fi
/system/bin/mv "$STAGE_DIR" "$DST_DIR"
/system/bin/rm -rf "$BACKUP_DIR"

if [ -x "$SV_CTL" ]; then
    "$SV_CTL" plugins reload >/dev/null 2>&1 || true
fi
