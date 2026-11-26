#!/usr/bin/env bash
#
# Rootfs image preparation and chroot provisioning helpers sourced by scripts/bootstrap-device.sh.
#

prepare_rootfs_image() {
    adb_su "${REMOTE_MOD_DIR}/bin/strata -c ${CONTAINER_ID} stop >/dev/null 2>&1 || true"
    local exists
    exists="$(adb_su "[ -f ${IMAGE_PATH} ] && echo YES || echo NO" | tr -d '\r')"
    if [ "${RECREATE}" -eq 0 ] && [ "${exists}" = "YES" ]; then
        echo "[*] Reusing existing rootfs image: ${IMAGE_PATH}"
        return 0
    fi

    mkdir -p "${CACHE_DIR}"
    local sums_file="${CACHE_DIR}/SHA256SUMS" tarball expected_sha local_tar actual_sha
    echo "[*] Fetching Ubuntu 24.04 ARM64 checksums from ${UBUNTU_BASE_URL}..."
    curl -fsSL -o "${sums_file}" "${UBUNTU_BASE_URL}/SHA256SUMS"

    if [ "${UBUNTU_VERSION}" = "auto" ]; then
        tarball="$(awk '$2 ~ /^\*?ubuntu-base-24\.04(\.[0-9]+)?-base-arm64\.tar\.gz$/ {sub(/^\*/, "", $2); print $2}' "${sums_file}" | sort -V | tail -n 1)"
    else
        tarball="ubuntu-base-${UBUNTU_VERSION}-base-arm64.tar.gz"
    fi
    [ -n "${tarball}" ] || { echo "[!] Failed to resolve Ubuntu 24.04 ARM64 tarball from ${sums_file}" >&2; exit 1; }
    expected_sha="$(awk -v f="${tarball}" '{name=$2; sub(/^\*/, "", name); if (name == f) print $1}' "${sums_file}")"
    [ -n "${expected_sha}" ] || { echo "[!] Failed to find SHA256 checksum for ${tarball}" >&2; exit 1; }

    local_tar="${CACHE_DIR}/${tarball}"
    if [ ! -f "${local_tar}" ] || [ "$(sha256sum "${local_tar}" | awk '{print $1}')" != "${expected_sha}" ]; then
        echo "[*] Downloading ${tarball} into ${CACHE_DIR}..."
        curl -fSL -o "${local_tar}" "${UBUNTU_BASE_URL}/${tarball}"
        actual_sha="$(sha256sum "${local_tar}" | awk '{print $1}')"
        [ "${actual_sha}" = "${expected_sha}" ] || { echo "[!] SHA256 mismatch: got ${actual_sha}, expected ${expected_sha}" >&2; exit 1; }
    else
        echo "[*] Using cached verified tarball: ${local_tar}"
    fi

    echo "[*] Pushing ${tarball} and unpacking into sparse ext4 image (${IMAGE_SIZE})..."
    local volume_label="strata_${CONTAINER_ID}"
    volume_label="${volume_label:0:16}"
    adb_cmd push "${local_tar}" "/data/local/tmp/${tarball}" >/dev/null
    adb_su "
        set -e
        LOOP_DEV=''
        cleanup() {
            /system/bin/umount ${MOUNT_DIR} 2>/dev/null || true
            [ -n \"\${LOOP_DEV}\" ] && /system/bin/losetup -d \"\${LOOP_DEV}\" 2>/dev/null || true
            /system/bin/rm -f /data/local/tmp/${tarball}
        }
        trap cleanup EXIT
        /system/bin/umount ${MOUNT_DIR} 2>/dev/null || true
        /system/bin/mkdir -p $(dirname "${IMAGE_PATH}") ${MOUNT_DIR}
        /system/bin/chmod 0755 /data/local/strata /data/local/strata/containers $(dirname "${IMAGE_PATH}") ${MOUNT_DIR}
        /system/bin/rm -f ${IMAGE_PATH}
        /system/bin/truncate -s ${IMAGE_SIZE} ${IMAGE_PATH}
        /system/bin/mkfs.ext4 -F -L ${volume_label} ${IMAGE_PATH} >/dev/null
        /system/bin/chcon u:object_r:vold_data_file:s0 ${IMAGE_PATH} 2>/dev/null || true
        LOOP_DEV=\$(/system/bin/losetup -f --show ${IMAGE_PATH})
        /system/bin/mount -t ext4 -o rw,relatime \"\${LOOP_DEV}\" ${MOUNT_DIR}
        if [ -x /data/adb/ksu/bin/busybox ]; then /data/adb/ksu/bin/busybox tar -xzpf /data/local/tmp/${tarball} -C ${MOUNT_DIR}
        elif [ -x /system/bin/tar ]; then /system/bin/tar -xzpf /data/local/tmp/${tarball} -C ${MOUNT_DIR}
        else echo 'No suitable tar binary found on device' >&2; exit 1; fi
        /system/bin/mkdir -p ${MOUNT_DIR}/data/local/tmp ${MOUNT_DIR}/root/work
        /system/bin/sync
    "
}

provision_rootfs_via_chroot() {
    echo "[*] Staging container scripts, static strata-debug, and SSH public key..."
    adb_cmd shell "rm -rf ${CONTAINER_STAGE} && mkdir -p ${CONTAINER_STAGE}"
    adb_cmd push \
        "${SCRIPT_DIR}/module/container/setup-container.sh" \
        "${SCRIPT_DIR}/module/container/host-exec" \
        "${CONTAINER_STAGE}/" >/dev/null
    if [ -f "${SCRIPT_DIR}/out/stage/bin/strata-debug" ]; then
        adb_cmd push "${SCRIPT_DIR}/out/stage/bin/strata-debug" "${CONTAINER_STAGE}/strata-debug" >/dev/null
    fi
    [ -n "${PUBKEY_PATH}" ] && adb_cmd push "${PUBKEY_PATH}" "${CONTAINER_STAGE}/authorized_keys" >/dev/null

    local setup_flags=""
    [ "${DEBUG_PROFILE}" -eq 1 ] && setup_flags="--debug-profile"

    echo "[*] Running module/container/setup-container.sh inside chroot (installing Dropbear, mise, uv, tools)..."
    adb_su "
        set -eu
        LOOP_DEV=''
        cleanup() {
            for p in ${MOUNT_DIR}/data/local/tmp ${MOUNT_DIR}/dev/pts ${MOUNT_DIR}/dev ${MOUNT_DIR}/sys ${MOUNT_DIR}/proc; do
                /system/bin/umount -l \"\$p\" 2>/dev/null || true
            done
            /system/bin/umount -l ${MOUNT_DIR} 2>/dev/null || true
            [ -n \"\${LOOP_DEV}\" ] && /system/bin/losetup -d \"\${LOOP_DEV}\" 2>/dev/null || true
            /system/bin/rm -rf ${CONTAINER_STAGE}
        }
        trap cleanup EXIT
        ${REMOTE_MOD_DIR}/bin/strata -c ${CONTAINER_ID} stop >/dev/null 2>&1 || true
        /system/bin/mkdir -p ${MOUNT_DIR}
        LOOP_DEV=\$(/system/bin/losetup -f --show ${IMAGE_PATH})
        /system/bin/mount -t ext4 -o rw,relatime \"\${LOOP_DEV}\" ${MOUNT_DIR}
        /system/bin/mkdir -p ${MOUNT_DIR}/proc ${MOUNT_DIR}/sys ${MOUNT_DIR}/dev ${MOUNT_DIR}/dev/pts ${MOUNT_DIR}/data/local/tmp
        /system/bin/mount -t proc proc ${MOUNT_DIR}/proc
        /system/bin/mount -t sysfs sysfs ${MOUNT_DIR}/sys
        /system/bin/mount --bind /dev ${MOUNT_DIR}/dev
        /system/bin/mount --bind /dev/pts ${MOUNT_DIR}/dev/pts 2>/dev/null || true
        /system/bin/mount --bind /data/local/tmp ${MOUNT_DIR}/data/local/tmp
        /system/bin/chroot ${MOUNT_DIR} /usr/bin/env -i HOME=/root TERM=xterm-256color \
            PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
            /bin/sh ${CONTAINER_STAGE}/setup-container.sh ${setup_flags} ${CONTAINER_STAGE}
        /system/bin/sync
    "
}
