# **Aura's renderer, built for a Rivet.** `PLAN.md` §104.25.
#
# This is a SECOND build of Aura's C++ sources, not a replacement for the one in
# Aura's own tree. That one targets Raspberry Pi OS: GLFW with the Wayland
# backend, under the Cage kiosk compositor, with two patches that talk GLFW down
# from OpenGL 3.3 to what a Pi 4's V3D reports.
#
# **A Rivet has no Wayland, no Cage and no compositor**, and is not going to
# have one — `PLAN.md` §104.18.3 records why: painting over an application needs
# either a compositor or DRM leases, and the whole shape of kiosk mode is that
# exactly one program owns the screen (`ADR-015` §6.2).
#
# So this builds Raylib's **DRM backend** instead, which needs none of that. It
# opens `/dev/dri/card0` through GBM, makes an EGL context, and reads
# `/dev/input/event*` directly — which is precisely the pair the `display` and
# `input` device classes grant. The two Pi 4 GLFW patches are not carried over
# and are not missed: they exist because GLFW asks for desktop OpenGL, and this
# path asks for GLES2, which is what the hardware has.
#
# Nothing in Aura's tree is modified by this file. It compiles those sources
# where they lie, named by `AURA_SOURCE_DIR`.
cmake_minimum_required(VERSION 3.25)
project(aura-renderer CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT DEFINED AURA_SOURCE_DIR)
    message(FATAL_ERROR
        "AURA_SOURCE_DIR is not set. It must name Aura's checkout — the C++ "
        "sources are read from <AURA_SOURCE_DIR>/cpp/src and are not copied "
        "into this repository. Pass -DAURA_SOURCE_DIR=/path/to/Aura.")
endif()
if(NOT EXISTS "${AURA_SOURCE_DIR}/cpp/src/main.cpp")
    message(FATAL_ERROR
        "no main.cpp under ${AURA_SOURCE_DIR}/cpp/src — that is not an Aura "
        "checkout, or its layout has changed.")
endif()

include(FetchContent)

# **`PLATFORM=DRM`, and it is the whole point of this file.**
#
# Raylib's own CMake then reports `No GLFW required on PLATFORM_DRM` and
# `GRAPHICS=GRAPHICS_API_OPENGL_ES2`, which are the two lines worth checking in
# its configure output.
set(PLATFORM DRM CACHE STRING "" FORCE)
set(BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
FetchContent_Declare(
    raylib
    GIT_REPOSITORY https://github.com/raysan5/raylib.git
    GIT_TAG 5.5
    GIT_SHALLOW TRUE
)
FetchContent_MakeAvailable(raylib)

FetchContent_Declare(
    nlohmann_json
    GIT_REPOSITORY https://github.com/nlohmann/json.git
    GIT_TAG v3.11.3
    GIT_SHALLOW TRUE
)
FetchContent_MakeAvailable(nlohmann_json)

add_executable(aura-renderer "${AURA_SOURCE_DIR}/cpp/src/main.cpp")
target_include_directories(aura-renderer PRIVATE "${AURA_SOURCE_DIR}/cpp/src")
target_link_libraries(aura-renderer PRIVATE raylib nlohmann_json::nlohmann_json)

# **`libdrm`'s headers are not on the default include path**, and Raylib's DRM
# platform does not ask pkg-config for them. `xf86drm.h` includes `drm.h` as a
# bare name, so without this the first file to touch it stops with
# `fatal error: drm.h: No such file or directory` — which reads like a missing
# package and is a missing `-I`.
find_package(PkgConfig REQUIRED)
pkg_check_modules(LIBDRM REQUIRED libdrm)
target_include_directories(aura-renderer PRIVATE ${LIBDRM_INCLUDE_DIRS})
target_compile_options(raylib PRIVATE ${LIBDRM_CFLAGS_OTHER} -I${LIBDRM_INCLUDEDIR}/libdrm)
