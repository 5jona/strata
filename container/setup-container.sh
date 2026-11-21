#!/bin/sh
#
# Idempotent inside-container provisioning script for Ubuntu 24.04 ARM64 under Strata.
# Installs Python, key-only Dropbear SSH + openssh-sftp-server, debugging tools,
# host-exec bridge, Android command wrappers, and the local zero-ADB shim.
#

set -eu

export DEBIAN_FRONTEND=noninteractive
export LANG=C.UTF-8
export LC_ALL=C.UTF-8

STAGE_DIR="${1:-/data/local/tmp/strata_container_assets}"

echo "[*] Configuring container APT policy and base resolvers..."
mkdir -p /usr/sbin /run/strata /root/.ssh /root/work /etc/dropbear /usr/local/bin
chmod 0755 /run/strata /root/work /etc/dropbear /usr/local/bin
chmod 0700 /root/.ssh

cat > /usr/sbin/policy-rc.d <<'EOF'
#!/bin/sh
exit 101
EOF
chmod 0755 /usr/sbin/policy-rc.d

if [ ! -s /etc/resolv.conf ]; then
    printf "nameserver 223.5.5.5\nnameserver 1.1.1.1\nnameserver 8.8.8.8\n" > /etc/resolv.conf
fi

# Keep apt running as root so Android netd/VPN and OEM UID monitors stay unaffected.
rm -f /etc/apt/apt.conf.d/99ruri-devbox /etc/profile.d/ruri-devbox.sh
cat > /etc/apt/apt.conf.d/99strata <<'EOF'
APT::Install-Recommends "false";
APT::Install-Suggests "false";
APT::Sandbox::User "root";
EOF

echo "[*] Updating APT index and installing core Python/SSH/debugging toolchain..."
apt-get update
# Dropbear avoids OpenSSH non-root privsep UID drops that trigger OEM security watchdogs.
apt-get purge -y openssh-server >/dev/null 2>&1 || true
apt-get install -y \
    ca-certificates \
    curl \
    wget \
    dropbear-bin \
    file \
    git \
    jq \
    kmod \
    less \
    lsof \
    make \
    gcc \
    g++ \
    libc6-dev \
    pkg-config \
    openssh-client \
    openssh-sftp-server \
    iproute2 \
    iputils-ping \
    dnsutils \
    netcat-openbsd \
    procps \
    psmisc \
    python3 \
    python3-pip \
    python3-venv \
    python3-dev \
    python3-pytest \
    ripgrep \
    rsync \
    strace \
    gdb \
    tmux \
    tree \
    unzip \
    vim-tiny \
    xz-utils \
    zstd \
    tzdata

echo "[*] Generating persistent Dropbear host keys if missing..."
if [ ! -f /etc/dropbear/dropbear_ed25519_host_key ]; then
    dropbearkey -t ed25519 -f /etc/dropbear/dropbear_ed25519_host_key
fi
if [ ! -f /etc/dropbear/dropbear_rsa_host_key ]; then
    dropbearkey -t rsa -s 3072 -f /etc/dropbear/dropbear_rsa_host_key
fi
chmod 0600 /etc/dropbear/dropbear_*_host_key

echo "[*] Installing host-exec and zero-ADB bridge scripts..."
if [ -f "${STAGE_DIR}/host-exec" ]; then
    cp "${STAGE_DIR}/host-exec" /usr/local/bin/host-exec
    chmod 0755 /usr/local/bin/host-exec
fi

if [ -f "${STAGE_DIR}/adb-shim.py" ]; then
    cp "${STAGE_DIR}/adb-shim.py" /usr/local/bin/adb
    chmod 0755 /usr/local/bin/adb
fi

if [ -f "${STAGE_DIR}/authorized_keys" ]; then
    touch /root/.ssh/authorized_keys
    while IFS= read -r keyline; do
        [ -n "$keyline" ] || continue
        if ! grep -Fqx "$keyline" /root/.ssh/authorized_keys 2>/dev/null; then
            printf '%s\n' "$keyline" >> /root/.ssh/authorized_keys
        fi
    done < "${STAGE_DIR}/authorized_keys"
    chmod 0600 /root/.ssh/authorized_keys
fi

echo "[*] Creating Android host tool wrappers in /usr/local/bin..."
ANDROID_TOOLS="getprop setprop dumpsys logcat am pm cmd wm settings input service screencap toybox resetprop ksud strata"
for tool in $ANDROID_TOOLS; do
    cat > "/usr/local/bin/${tool}" <<EOF
#!/bin/sh
exec /usr/local/bin/host-exec "${tool}" "\$@"
EOF
    chmod 0755 "/usr/local/bin/${tool}"
done

cat > /usr/local/bin/android-sh <<'EOF'
#!/bin/sh
exec /usr/local/bin/host-exec /system/bin/sh "$@"
EOF
chmod 0755 /usr/local/bin/android-sh

cat > /etc/profile.d/strata.sh <<'EOF'
export LANG=C.UTF-8
export LC_ALL=C.UTF-8
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PIP_DISABLE_PIP_VERSION_CHECK=1
EOF
chmod 0644 /etc/profile.d/strata.sh

apt-get clean
rm -rf /var/lib/apt/lists/*

echo "[+] Container provisioning completed."
