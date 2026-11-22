#!/system/bin/sh
#
# Atomically mirrors a bundled SideVeil plugin package into
# `/data/system/sideveil/plugins/<plugin_id>` when SideVeil is installed.
# Silently no-ops when SideVeil is absent so standalone daemons remain independent.
#
# Usage:
#   sync-sideveil-plugin.sh [MODDIR_OR_PLUGIN_DIR] [PLUGIN_ID]
#

unset ASH_STANDALONE 2>/dev/null || true

ARG1="${1:-${0%/*}/..}"
PLUGIN_ID="${2:-}"

if [ -f "${ARG1}/manifest.json" ]; then
    SRC_DIR="$ARG1"
    if [ -z "$PLUGIN_ID" ]; then
        PLUGIN_ID="$(/system/bin/sed -n 's/.*"id"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${SRC_DIR}/manifest.json" | /system/bin/head -n 1)"
    fi
else
    PLUGIN_ID="${PLUGIN_ID:-strata.manager}"
    SRC_DIR="${ARG1}/plugins/${PLUGIN_ID}"
fi

case "$PLUGIN_ID" in
    ""|*[!a-z0-9.-]*|*..*|.*|*.|-*|*-)
        echo "sync-sideveil-plugin: invalid plugin id '$PLUGIN_ID'" >&2
        exit 1
        ;;
esac

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
