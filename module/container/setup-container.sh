#!/bin/sh
#
# Idempotent inside-container provisioning script for Debian/Ubuntu ARM64 under Strata.
# Configures canonical Strata layout (/workspace, /opt/strata, /etc/strata, /var/cache/strata),
# installs lean base packages + ColorOS-safe Dropbear SSH + openssh-sftp-server,
# bootstraps mise and uv into /opt/strata/bin, and installs host-exec + static strata-debug.
#

set -eu

export DEBIAN_FRONTEND=noninteractive
export LANG=C.UTF-8
export LC_ALL=C.UTF-8

STAGE_DIR="/data/local/tmp/strata_container_assets"
DEBUG_PROFILE=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --debug-profile) DEBUG_PROFILE=1; shift ;;
        *) STAGE_DIR="$1"; shift ;;
    esac
done

CANONICAL_PATH="/opt/strata/mise/shims:/opt/strata/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
STRATA_PROFILE_START="# >>> strata-env >>>"

detect_arch() {
    case "$(uname -m 2>/dev/null || echo aarch64)" in
        aarch64|arm64) echo "arm64" ;;
        x86_64|amd64) echo "x64" ;;
        *) uname -m ;;
    esac
}

append_profile_hook() {
    target_file="$1"
    [ -f "$target_file" ] || : > "$target_file"
    if head -n 1 "$target_file" 2>/dev/null | grep -Fq "$STRATA_PROFILE_START"; then
        return 0
    fi
    tmp_hook="${target_file}.strata.tmp"
    cat > "$tmp_hook" <<'EOF'
# >>> strata-env >>>
[ -r /etc/profile.d/strata-env.sh ] && . /etc/profile.d/strata-env.sh
# <<< strata-env <<<

EOF
    awk '/^# >>> strata-env >>>$/ {s=1; next} /^# <<< strata-env <<<$/ {s=0; next} !s {print}' "$target_file" >> "$tmp_hook"
    mv -f "$tmp_hook" "$target_file"
    chmod 0644 "$target_file"
}

echo "[*] Creating canonical Strata directory layout..."
mkdir -p \
    /workspace /opt/strata/bin /opt/strata/mise/shims /opt/strata/uv/tools \
    /etc/strata/mise /etc/profile.d /var/cache/strata/mise /var/cache/strata/uv \
    /run/strata /etc/dropbear /usr/sbin /usr/local/bin \
    /root/.ssh /root/.config /root/.local/share /root/.local/state /root/.cache

chmod 0755 \
    /workspace /opt/strata /opt/strata/bin /opt/strata/mise /opt/strata/mise/shims \
    /opt/strata/uv /opt/strata/uv/tools /etc/strata /etc/strata/mise /etc/profile.d \
    /var/cache/strata /var/cache/strata/mise /var/cache/strata/uv \
    /run/strata /etc/dropbear /usr/local/bin /root/.local /root/.local/share /root/.cache
chmod 0700 /root /root/.ssh /root/.config /root/.local/state

printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d
chmod 0755 /usr/sbin/policy-rc.d
if [ ! -s /etc/resolv.conf ]; then
    printf "nameserver 223.5.5.5\nnameserver 1.1.1.1\nnameserver 8.8.8.8\n" > /etc/resolv.conf
fi

echo "[*] Writing canonical environment profiles, mise baseline, and layout metadata..."
rm -f /etc/apt/apt.conf.d/99ruri-devbox /etc/apt/apt.conf.d/99strata /etc/profile.d/ruri-devbox.sh /etc/profile.d/strata.sh

# Keep apt running as root so Android netd/VPN and OEM UID watchdogs stay unaffected.
mkdir -p /etc/apt/apt.conf.d
cat > /etc/apt/apt.conf.d/99strata-sandbox <<'EOF'
APT::Install-Recommends "false";
APT::Install-Suggests "false";
APT::Sandbox::User "root";
EOF

cat > /etc/profile.d/strata-env.sh <<'EOF'
# Strata managed container environment (do not edit manually)
export HOME="/root"
export USER="root"
export LOGNAME="root"
export LANG="C.UTF-8"
export LC_ALL="C.UTF-8"
export XDG_CONFIG_HOME="/root/.config"
export XDG_DATA_HOME="/root/.local/share"
export XDG_CACHE_HOME="/root/.cache"
export XDG_STATE_HOME="/root/.local/state"
export MISE_DATA_DIR="/opt/strata/mise"
export MISE_CACHE_DIR="/var/cache/strata/mise"
export MISE_CONFIG_DIR="/etc/strata/mise"
export MISE_GLOBAL_CONFIG_FILE="/etc/strata/mise/config.toml"
export UV_CACHE_DIR="/var/cache/strata/uv"
export UV_TOOL_DIR="/opt/strata/uv/tools"
export UV_TOOL_BIN_DIR="/opt/strata/bin"
export UV_LINK_MODE="copy"
export PIP_DISABLE_PIP_VERSION_CHECK="1"
export BASH_ENV="/etc/profile.d/strata-env.sh"
case ":${PATH:-}:" in
  *":/opt/strata/mise/shims:"*) ;;
  *) export PATH="/opt/strata/mise/shims:/opt/strata/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" ;;
esac
[ -r /etc/strata/mise/mirrors.env ] && . /etc/strata/mise/mirrors.env
[ -r /etc/strata/mise/secrets.env ] && . /etc/strata/mise/secrets.env
if [ -n "${BASH_VERSION:-}" ]; then
  case "$-" in
    *i*) [ -x /opt/strata/bin/mise ] && eval "$(/opt/strata/bin/mise activate bash)" 2>/dev/null || true ;;
  esac
fi
EOF
chmod 0644 /etc/profile.d/strata-env.sh

cat > /etc/environment <<EOF
HOME="/root"
USER="root"
LOGNAME="root"
LANG="C.UTF-8"
LC_ALL="C.UTF-8"
PATH="${CANONICAL_PATH}"
XDG_CONFIG_HOME="/root/.config"
XDG_DATA_HOME="/root/.local/share"
XDG_CACHE_HOME="/root/.cache"
XDG_STATE_HOME="/root/.local/state"
MISE_DATA_DIR="/opt/strata/mise"
MISE_CACHE_DIR="/var/cache/strata/mise"
MISE_CONFIG_DIR="/etc/strata/mise"
MISE_GLOBAL_CONFIG_FILE="/etc/strata/mise/config.toml"
UV_CACHE_DIR="/var/cache/strata/uv"
UV_TOOL_DIR="/opt/strata/uv/tools"
UV_TOOL_BIN_DIR="/opt/strata/bin"
UV_LINK_MODE="copy"
PIP_DISABLE_PIP_VERSION_CHECK="1"
BASH_ENV="/etc/profile.d/strata-env.sh"
EOF
chmod 0644 /etc/environment

append_profile_hook /etc/profile
append_profile_hook /etc/bash.bashrc
append_profile_hook /root/.bashrc

if [ ! -f /etc/strata/mise/config.toml ]; then
    cat > /etc/strata/mise/config.toml <<'EOF'
# Strata canonical global mise configuration (/etc/strata/mise/config.toml)
[settings]
jobs = 4
http_timeout = "60s"
fetch_remote_versions_cache = "1h"
yes = true
experimental = false
idiomatic_version_file_enable_tools = ["node", "python", "rust"]

[settings.python]
compile = false
uv_venv_auto = "create|source"

[settings.node]
compile = false

[tools]
EOF
    chmod 0644 /etc/strata/mise/config.toml
fi

cat > /etc/strata/layout.json <<EOF
{
  "cacheRoot": "/var/cache/strata",
  "etcDir": "/etc/strata",
  "etcEnvironment": "/etc/environment",
  "miseCacheDir": "/var/cache/strata/mise",
  "miseConfigDir": "/etc/strata/mise",
  "miseConfigFile": "/etc/strata/mise/config.toml",
  "miseDataDir": "/opt/strata/mise",
  "miseShimsDir": "/opt/strata/mise/shims",
  "optBinDir": "/opt/strata/bin",
  "optRoot": "/opt/strata",
  "path": "${CANONICAL_PATH}",
  "profileEnv": "/etc/profile.d/strata-env.sh",
  "schemaVersion": 1,
  "uvCacheDir": "/var/cache/strata/uv",
  "uvToolDir": "/opt/strata/uv/tools",
  "workspaceDir": "/workspace"
}
EOF
chmod 0644 /etc/strata/layout.json

. /etc/profile.d/strata-env.sh

rewrite_apt_mirror() {
    mirror_url="$1"
    for src in /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources; do
        [ -f "$src" ] || continue
        sed -i "s|https\?://ports\.ubuntu\.com/ubuntu-ports|${mirror_url}|g" "$src"
        sed -i "s|http://mirrors\.tuna\.tsinghua\.edu\.cn/ubuntu-ports|${mirror_url}|g" "$src"
    done
}

echo "[*] Installing lean base packages and ColorOS-safe Dropbear SSH..."
[ -n "${APT_MIRROR:-}" ] && rewrite_apt_mirror "${APT_MIRROR}"
apt-get update -o Acquire::Retries=1 -o Acquire::http::Timeout=10 || true
if ! apt-cache show dropbear-bin >/dev/null 2>&1; then
    echo "[*] Primary APT mirror unavailable; falling back to TUNA ubuntu-ports mirror..."
    rewrite_apt_mirror "http://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports"
    apt-get update -o Acquire::Retries=2 -o Acquire::http::Timeout=15
fi
# Dropbear avoids OpenSSH non-root privsep UID drops that trigger OEM security watchdogs.
apt-get purge -y openssh-server >/dev/null 2>&1 || true
apt-get install -y --no-install-recommends \
    ca-certificates curl dropbear-bin git gzip iproute2 libatomic1 locales \
    openssh-client openssh-sftp-server procps sqlite3 tar tzdata unzip xz-utils zstd
if grep -qs "http://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports" /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources 2>/dev/null; then
    rewrite_apt_mirror "https://mirrors.tuna.tsinghua.edu.cn/ubuntu-ports"
fi

if [ "$DEBUG_PROFILE" -eq 1 ]; then
    echo "[*] Installing optional --debug-profile packages..."
    apt-get install -y --no-install-recommends strace tcpdump gdb lsof binutils patchelf bpftrace || \
        apt-get install -y --no-install-recommends strace tcpdump gdb lsof binutils patchelf || true
fi

ARCH_TAG="$(detect_arch)"
MISE_URL="${MISE_INSTALL_URL:-https://mise.run}"
UV_URL="${UV_INSTALL_URL:-https://astral.sh/uv/install.sh}"

if [ ! -x /opt/strata/bin/mise ] || ! /opt/strata/bin/mise --version >/dev/null 2>&1; then
    echo "[*] Installing mise into /opt/strata/bin/mise (arch=${ARCH_TAG})..."
    curl -fsSL --connect-timeout 15 --max-time 120 "$MISE_URL" \
        | env MISE_INSTALL_PATH=/opt/strata/bin/mise sh \
        || echo "[!] Warning: mise installation skipped (network or mirror unavailable)."
    [ -f /opt/strata/bin/mise ] && chmod 0755 /opt/strata/bin/mise
fi

if [ ! -x /opt/strata/bin/uv ] || ! /opt/strata/bin/uv --version >/dev/null 2>&1; then
    echo "[*] Installing uv into /opt/strata/bin/uv (arch=${ARCH_TAG})..."
    if [ -x /opt/strata/bin/mise ] && /opt/strata/bin/mise use -g --yes uv@latest; then
        uv_bin="$(/opt/strata/bin/mise which uv 2>/dev/null || true)"
        [ -n "$uv_bin" ] && [ -x "$uv_bin" ] && ln -sf "$uv_bin" /opt/strata/bin/uv
    fi
    if [ ! -x /opt/strata/bin/uv ]; then
        curl -fsSL --connect-timeout 15 --max-time 120 "$UV_URL" \
            | env UV_INSTALL_DIR=/opt/strata/bin UV_NO_MODIFY_PATH=1 sh \
            || echo "[!] Warning: uv installation skipped (network or mirror unavailable)."
    fi
    [ -f /opt/strata/bin/uv ] && chmod 0755 /opt/strata/bin/uv
fi

echo "[*] Generating persistent Dropbear host keys if missing..."
[ -f /etc/dropbear/dropbear_ed25519_host_key ] || dropbearkey -t ed25519 -f /etc/dropbear/dropbear_ed25519_host_key
[ -f /etc/dropbear/dropbear_rsa_host_key ] || dropbearkey -t rsa -s 3072 -f /etc/dropbear/dropbear_rsa_host_key
chmod 0600 /etc/dropbear/dropbear_*_host_key

echo "[*] Installing host-exec, static strata-debug, and Android host wrappers..."
if [ -f "${STAGE_DIR}/host-exec" ]; then
    cp "${STAGE_DIR}/host-exec" /usr/local/bin/host-exec
    chmod 0755 /usr/local/bin/host-exec
fi

rm -rf /usr/local/bin/adb /usr/local/lib/strata-debug
if [ -f "${STAGE_DIR}/strata-debug" ]; then
    cp "${STAGE_DIR}/strata-debug" /usr/local/bin/strata-debug
    chmod 0755 /usr/local/bin/strata-debug
fi

if [ -f "${STAGE_DIR}/authorized_keys" ]; then
    touch /root/.ssh/authorized_keys
    while IFS= read -r keyline; do
        [ -n "$keyline" ] || continue
        grep -Fqx "$keyline" /root/.ssh/authorized_keys 2>/dev/null || printf '%s\n' "$keyline" >> /root/.ssh/authorized_keys
    done < "${STAGE_DIR}/authorized_keys"
    chmod 0600 /root/.ssh/authorized_keys
fi

for tool in getprop setprop dumpsys logcat am pm cmd wm settings input service screencap uiautomator getevent toybox resetprop ksud strata; do
    printf '#!/bin/sh\nexec /usr/local/bin/host-exec "%s" "$@"\n' "$tool" > "/usr/local/bin/${tool}"
    chmod 0755 "/usr/local/bin/${tool}"
done
printf '#!/bin/sh\nexec /usr/local/bin/host-exec /system/bin/sh "$@"\n' > /usr/local/bin/android-sh
chmod 0755 /usr/local/bin/android-sh

apt-get clean
rm -rf /var/lib/apt/lists/*
echo "[+] Container provisioning completed."
