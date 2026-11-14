# CMake cross-compilation toolchain for the LG webOS ARM (soft-float gnueabi)
# SDK. whisper-rs-sys invokes CMake to build whisper.cpp/ggml, so these settings
# make that build target the webOS sysroot instead of the CI host.
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR arm)

if(NOT DEFINED ENV{WEBOS_NDK})
  message(FATAL_ERROR "WEBOS_NDK environment variable must point at the webOS SDK")
endif()
if(NOT DEFINED ENV{WEBOS_SYSROOT})
  message(FATAL_ERROR "WEBOS_SYSROOT environment variable must point at the webOS sysroot")
endif()

set(CMAKE_C_COMPILER "$ENV{WEBOS_NDK}/bin/arm-webos-linux-gnueabi-gcc")
set(CMAKE_CXX_COMPILER "$ENV{WEBOS_NDK}/bin/arm-webos-linux-gnueabi-g++")

set(CMAKE_SYSROOT "$ENV{WEBOS_SYSROOT}")
set(CMAKE_FIND_ROOT_PATH "$ENV{WEBOS_SYSROOT}")

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
