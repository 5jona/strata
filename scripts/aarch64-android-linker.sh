#!/usr/bin/env bash
set -euo pipefail

sdk_root="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "${sdk_root}" ]; then
    echo "ANDROID_HOME or ANDROID_SDK_ROOT must be set to link aarch64-linux-android targets" >&2
    exit 1
fi

ndk_root="${ANDROID_NDK_HOME:-}"
if [ -z "${ndk_root}" ] && [ -d "${sdk_root}/ndk" ]; then
    ndk_root="$(find "${sdk_root}/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)"
fi

if [ -z "${ndk_root}" ] || [ ! -d "${ndk_root}" ]; then
    echo "Android NDK not found under ${sdk_root}/ndk" >&2
    exit 1
fi

bin_dir="${ndk_root}/toolchains/llvm/prebuilt/linux-x86_64/bin"
for api in 37 36; do
    linker="${bin_dir}/aarch64-linux-android${api}-clang"
    if [ -x "${linker}" ]; then
        exec "${linker}" "$@"
    fi
done

echo "No executable aarch64-linux-android{37,36}-clang found under ${bin_dir}" >&2
exit 1
