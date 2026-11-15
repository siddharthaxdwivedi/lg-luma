#!/bin/sh
# Cross-build Vosk (libvosk) for the webOS ARM soft-float ABI.
#
# Every translation unit is compiled with -mfloat-abi=softfp: the generated code
# uses the hardware FP unit and NEON, while the calling convention stays the
# webOS soft-float ABI that the toolchain and the Rust gnueabi target require.
# Never use -mfloat-abi=hard here; it changes the ABI and breaks the link or the
# calls at runtime.
#
# Produces, under .build/vosk:
#   lib/libvosk.a                                vosk-api objects
#   lib/lib{openblas,fst*,lapack,blas,f2c}.a
#   src/kaldi/src/*/*.a                          Kaldi static libraries
# The Rust link step must place these archives inside a single
# -Wl,--start-group / -Wl,--end-group so the cyclic Kaldi references resolve.
set -eu

ROOT=$(dirname "$(dirname "$(realpath "$0")")")
: "${WEBOS_NDK:?Set WEBOS_NDK to the extracted webOS native toolchain}"
WEBOS_NDK=$(realpath "$WEBOS_NDK")
BIN_PREFIX="$WEBOS_NDK/bin/arm-webos-linux-gnueabi-"
CC="${BIN_PREFIX}gcc"
CXX="${BIN_PREFIX}g++"
AR="${BIN_PREFIX}ar"
RANLIB="${BIN_PREFIX}ranlib"
for tool in "$CC" "$CXX" "$AR" "$RANLIB"; do
  [ -x "$tool" ] || { printf '%s\n' "Missing webOS tool: $tool" >&2; exit 1; }
done

SYSROOT=${WEBOS_SYSROOT:-$("$CC" -print-sysroot)}
# An empty or host-root result would silently build against the CI host headers.
case "$SYSROOT" in
  ""|/) SYSROOT="";;
esac
[ -n "$SYSROOT" ] || SYSROOT="$WEBOS_NDK/arm-webos-linux-gnueabi/sysroot"
[ -f "$SYSROOT/usr/include/stdint.h" ] || { printf '%s\n' "Missing webOS sysroot: $SYSROOT" >&2; exit 1; }

# Soft-float ABI with hardware FP and NEON. -mfloat-abi=hard is forbidden.
ARCH_FLAGS="-mfloat-abi=softfp -mfpu=neon-vfpv4 -march=armv7-a -O3"
SYS_FLAG="--sysroot=$SYSROOT"
JOBS=${JOBS:-4}

BUILD="$ROOT/.build/vosk"
SRC="$BUILD/src"
PREFIX="$BUILD/lib"
mkdir -p "$SRC" "$PREFIX" "$BUILD/obj"

# Pinned upstream revisions: exact commits and tags, never branch tips.
# vosk-api v0.3.50: the release tag points at this commit.
VOSK_SHA=a7bf6a51e299152a8fb496b928a21eb79a1d7bea
KALDI_SHA=bc5baf14231660bd50b7d05788865b4ac6c34481
OPENFST_SHA=18e94e63870ebcf79ebb42b7035cd3cb626ec090
OPENBLAS_SHA=0b678b19dc03f2a999d6e038814c4c50b9640a4e
# clapack's v3.2.1 is a branch upstream, never a tag, so pin the commit.
CLAPACK_SHA=f4e1647de96343fb20eea08a9d4f0e0d04c31c51

fetch_sha() { # url dir sha
  rm -rf "$2"
  mkdir -p "$2"
  git -C "$2" init -q
  git -C "$2" remote add origin "$1"
  if ! git -C "$2" fetch -q --depth 1 origin "$3"; then
    git -C "$2" fetch -q origin
  fi
  git -C "$2" checkout -q --force "$3"
}

if [ ! -f "$PREFIX/lib/libopenblas.a" ]; then
  echo "== OpenBLAS =="
  fetch_sha https://github.com/xianyi/OpenBLAS "$SRC/OpenBLAS" "$OPENBLAS_SHA"
  # ARM_SOFTFP_ABI=1 makes OpenBLAS add -mfloat-abi=softfp itself.
  make -C "$SRC/OpenBLAS" -j"$JOBS" TARGET=ARMV7 ONLY_CBLAS=1 ARM_SOFTFP_ABI=1 \
    USE_THREAD=0 NUM_THREADS=1 AR="$AR" CC="$CC" HOSTCC=gcc
  make -C "$SRC/OpenBLAS" install PREFIX="$PREFIX"
fi

if [ ! -f "$PREFIX/lib/liblapack.a" ]; then
  echo "== CLAPACK =="
  fetch_sha https://github.com/alphacep/clapack "$SRC/clapack" "$CLAPACK_SHA"
  mkdir -p "$SRC/clapack/BUILD"
  (
    cd "$SRC/clapack/BUILD"
    cmake -DCMAKE_SYSTEM_NAME=Generic \
      -DCMAKE_C_COMPILER="$CC" \
      -DCMAKE_C_COMPILER_TARGET=arm-webos-linux-gnueabi \
      -DCMAKE_C_FLAGS="$ARCH_FLAGS $SYS_FLAG -fPIC" \
      -DCMAKE_AR="$AR" \
      -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
      -DCMAKE_CROSSCOMPILING=True \
      ..
    make -j"$JOBS" -C F2CLIBS/libf2c
    make -j"$JOBS" -C BLAS/SRC
    make -j"$JOBS" -C SRC
    find . -name '*.a' -exec cp {} "$PREFIX/lib/" \;
  )
fi

if [ ! -f "$PREFIX/lib/libfst.a" ]; then
  echo "== OpenFst =="
  fetch_sha https://github.com/alphacep/openfst "$SRC/openfst" "$OPENFST_SHA"
  (
    cd "$SRC/openfst"
    autoreconf -i
    CXX="$CXX" \
    CFLAGS="$ARCH_FLAGS -DFST_NO_DYNAMIC_LINKING" \
    CXXFLAGS="$ARCH_FLAGS -DFST_NO_DYNAMIC_LINKING" \
      ./configure --prefix="$PREFIX" \
        --host=arm-webos-linux-gnueabi \
        --enable-static --disable-shared --disable-bin --with-pic \
        --enable-lookahead-fsts --enable-ngram-fsts
    make -j"$JOBS"
    make install
  )
fi

if [ ! -f "$SRC/kaldi/src/online2/kaldi-online2.a" ]; then
  echo "== Kaldi =="
  fetch_sha https://github.com/alphacep/kaldi "$SRC/kaldi" "$KALDI_SHA"
  MK="$SRC/kaldi/src/makefiles/linux_openblas_arm.mk"
  # This makefile hard-codes the hard-float ABI, after $(EXTRA_CXXFLAGS), so it
  # must be rewritten in place rather than overridden.
  sed -i 's/-mfloat-abi=hard -mfpu=neon/-mfloat-abi=softfp -mfpu=neon-vfpv4/' "$MK"
  # webOS may not ship execinfo.h; drop the feature macro if the sysroot lacks it.
  if [ ! -f "$SYSROOT/usr/include/execinfo.h" ]; then
    sed -i 's/-DHAVE_EXECINFO_H=1//' "$MK"
  fi
  (
    cd "$SRC/kaldi/src"
    # --static omits -fPIC; the Rust ARM target links PIE, so force it.
    CXX="$CXX" AR="$AR" RANLIB="$RANLIB" \
    CXXFLAGS="$ARCH_FLAGS $SYS_FLAG -fPIC -DFST_NO_DYNAMIC_LINKING" \
      ./configure --static --use-cuda=no \
        --mathlib=OPENBLAS_CLAPACK \
        --host=arm-webos-linux-gnueabi \
        --openblas-root="$PREFIX" \
        --fst-root="$PREFIX" \
        --fst-version=1.8.0
    make -j"$JOBS" depend
    make -j"$JOBS" online2 rnnlm
  )
fi

if [ ! -f "$PREFIX/lib/libvosk.a" ]; then
  echo "== vosk-api =="
  fetch_sha https://github.com/alphacep/vosk-api "$SRC/vosk-api" "$VOSK_SHA"
  rm -rf "$BUILD/obj"
  mkdir -p "$BUILD/obj"
  # Build only the objects. The default target would also link libvosk.so; the
  # final Rust link validates the archives instead.
  make -C "$SRC/vosk-api/src" -j"$JOBS" \
    OUTDIR="$BUILD/obj" \
    KALDI_ROOT="$SRC/kaldi" \
    OPENFST_ROOT="$PREFIX" \
    OPENBLAS_ROOT="$PREFIX" \
    USE_SHARED=0 \
    CXX="$CXX" \
    EXTRA_CFLAGS="$ARCH_FLAGS $SYS_FLAG" \
    "$BUILD/obj/recognizer.o" \
    "$BUILD/obj/language_model.o" \
    "$BUILD/obj/model.o" \
    "$BUILD/obj/spk_model.o" \
    "$BUILD/obj/vosk_api.o" \
    "$BUILD/obj/postprocessor.o"
  "$AR" rcs "$PREFIX/lib/libvosk.a" "$BUILD/obj"/*.o
fi

echo "Vosk build complete: $PREFIX/lib/libvosk.a"
