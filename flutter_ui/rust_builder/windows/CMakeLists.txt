# The Flutter tooling requires that developers have a version of Visual Studio
# installed that includes CMake 3.14 or later. You should not increase this
# version, as doing so will cause the plugin to fail to compile for some
# customers of the plugin.
cmake_minimum_required(VERSION 3.14)

# Project-level configuration.
#
# The name must be `lumit_bridge` — the plugin's pubspec name, which is what
# `windows/flutter/generated_plugins.cmake` iterates over and what it expects the
# `_bundled_libraries` variable below to be prefixed with. The frb template ships
# `rust_lib_lumit_flutter`, so the built `.dll` was never bundled into the app.
#
# The crate path needs care, and is *not* the same as the Linux sibling's.
# cargokit builds it as `${CMAKE_CURRENT_SOURCE_DIR}/<path>`, and this file is
# reached through the generated symlink
# `windows/flutter/ephemeral/.plugin_symlinks/lumit_bridge` → `rust_builder`.
# Win32 canonicalises `..` *lexically*, before the filesystem sees it, so `..`
# walks back up that symlink path rather than out of `rust_builder` as it would
# on Linux. Counting from the symlink: 6 levels reaches `flutter_ui` (which is
# where the frb template's own `rust/` would sit) and 7 reaches the repository
# root, where `crates/` is. Hence seven — verified, not guessed.
set(PROJECT_NAME "lumit_bridge")
project(${PROJECT_NAME} LANGUAGES CXX)

include("../cargokit/cmake/cargokit.cmake")
apply_cargokit(${PROJECT_NAME} ../../../../../../../crates/lumit-bridge lumit_bridge "")

# List of absolute paths to libraries that should be bundled with the plugin.
# This list could contain prebuilt libraries, or libraries created by an
# external build triggered from this build file.
set(lumit_bridge_bundled_libraries
  "${${PROJECT_NAME}_cargokit_lib}"
  PARENT_SCOPE
)
