#!/bin/sh
set -eu

# Use the webOS NDK's ARM soft-float ABI and sysroot for Rust AND SDL linking.
: "${WEBOS_NDK:?Set WEBOS_NDK to the extracted and relocated webOS native toolchain}"
ROOT=$(dirname "$(dirname "$(realpath "$0")")")
WEBOS_NDK=$(realpath "$WEBOS_NDK")
CC="$WEBOS_NDK/bin/arm-webos-linux-gnueabi-gcc"
[ -x "$CC" ] || { printf '%s\n' "Missing webOS compiler: $CC" >&2; exit 1; }
SYSROOT=$("$CC" -print-sysroot)
# Some relocated macOS SDKs report a nonexistent compiler-relative sysroot.
if [ ! -f "$SYSROOT/usr/include/stdint.h" ]; then
  SYSROOT="$WEBOS_NDK/arm-webos-linux-gnueabi/sysroot"
fi
[ -f "$SYSROOT/usr/include/stdint.h" ] || { printf '%s\n' "Missing webOS sysroot headers: $SYSROOT" >&2; exit 1; }
node "$ROOT/tools/webos-runtime.mjs"
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER="$CC"
# The target's soft-float *calling convention* is required by webOS, but its
# default software FP codegen is not. Match the NDK's ARMv7/VFP baseline while
# retaining the gnueabi ABI; keep NEON off for portable Rust scalar code.
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="${CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS:-} -C target-cpu=cortex-a9 -C target-feature=-neon,-soft-float -C link-arg=--sysroot=$SYSROOT -C link-arg=-Wl,-rpath,\$ORIGIN/lib -C link-arg=-Wl,--start-group -C link-arg=-lasound -C link-arg=-lSDL2 -C link-arg=-lssl -C link-arg=-lcrypto -C link-arg=-lgcc_s -C link-arg=-lutil -C link-arg=-lrt -C link-arg=-lpthread -C link-arg=-lm -C link-arg=-ldl -C link-arg=-lc -C link-arg=-Wl,--end-group"
# An executable-only Home mount changes $ORIGIN to /usr/bin. These fallbacks must
# work before Rust startup; keep the installation list shared with native discovery.
while IFS= read -r directory; do
  export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="$CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS -C link-arg=-Wl,-rpath,$directory/lib"
done < "$ROOT/tv/install-directories.txt"
export CC_armv7_unknown_linux_gnueabi="$CC"
export AR_armv7_unknown_linux_gnueabi="$WEBOS_NDK/bin/arm-webos-linux-gnueabi-ar"
export CFLAGS_armv7_unknown_linux_gnueabi="${CFLAGS_armv7_unknown_linux_gnueabi:-} --sysroot=$SYSROOT"
export PKG_CONFIG_ALLOW_CROSS=1
# Nix's desktop SDL pkg-config path must not leak into this ARM build.
# Only set PKG_CONFIG_PATH if not already provided by CI (needed for ALSA).
: "${PKG_CONFIG_PATH:=$ROOT/.build/sdl-webos/pkgconfig}"
# This metadata contains absolute paths to the verified cross-built SDL runtime,
# so pkg-config must not prepend the target sysroot to those build-host paths.
# Only set PKG_CONFIG_SYSROOT_DIR if not already provided by CI (needed for ALSA).
: "${PKG_CONFIG_SYSROOT_DIR:=/}"
export PKG_CONFIG_LIBDIR="$ROOT/.build/sdl-webos/pkgconfig"

# Cross-compile whisper.cpp (bundled by whisper-rs-sys) for the webOS ARM
# soft-float target. whisper-rs-sys runs CMake and forwards any CMAKE_*/GGML_*
# environment variables as cache definitions.
export WEBOS_SYSROOT="$SYSROOT"
export CMAKE_TOOLCHAIN_FILE="$ROOT/tools/webos-cmake-toolchain.cmake"
export CMAKE_C_COMPILER="$CC"
export CMAKE_CXX_COMPILER="$WEBOS_NDK/bin/arm-webos-linux-gnueabi-g++"
export CXX="$CMAKE_CXX_COMPILER"
export CXX_armv7_unknown_linux_gnueabi="$CXX"
# Disable host CPU feature detection (-march=native) for the cross build.
export GGML_NATIVE=OFF
export GGML_OPENMP=OFF
# The crate's bundled bindings are 64-bit glibc; their compile-time size
# assertions fail for 32-bit ARM, so bindgen must generate ARM bindings.
unset WHISPER_DONT_GENERATE_BINDINGS
BINDGEN_TARGET_ARGS="--target=armv7-unknown-linux-gnueabi --sysroot=$SYSROOT -isystem $SYSROOT/usr/include"
export BINDGEN_EXTRA_CLANG_ARGS_armv7_unknown_linux_gnueabi="$BINDGEN_TARGET_ARGS"
export BINDGEN_EXTRA_CLANG_ARGS="$BINDGEN_TARGET_ARGS"

exec cargo build --locked --manifest-path "$ROOT/Cargo.toml" --package luma --package luma-tv-helper --target armv7-unknown-linux-gnueabi --features webos,voice-assistant --release
