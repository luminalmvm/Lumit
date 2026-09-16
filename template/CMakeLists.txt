# The C and C++ half of the template: build each example and lay it out as a
# bundle Lumit can open.
#
# In plain terms
#
#   cmake -B build -DCMAKE_BUILD_TYPE=Release
#   cmake --build build --config Release
#   lfx-validator build/bundles/Exposure.lfx.bundle
#
# A bundle is a folder with a listing and one directory per architecture:
#
#   Exposure.lfx.bundle/
#     Contents/
#       lfx.toml
#       linux-x86_64/Exposure.lfx
#
# so the two things this file does are compile the payload under the right
# architecture name and copy the listing beside it. The payload's extension is
# `.lfx` on every platform - it is an ordinary shared library underneath, and
# the extension is what a scan looks for.
#
# The Rust example is `cargo build --release` instead; `examples/saturation`
# says how, and `scripts/stage-bundle.py` lays its payload out the same way.

cmake_minimum_required(VERSION 3.16)

project(lumit_lfx_template LANGUAGES C CXX)

# --------------------------------------------------------- the architecture --

# The seven names LFX knows, and the one this build belongs under. A scan tries
# this machine's own first and falls back to a foreign CPU only where the
# platform emulates one, so a bundle may carry several of these directories at
# once and a machine that gains an architecture needs no new bundle.
#
# It is a cache variable because the answer is the *target's* and not the
# builder's: a cross build, or a host driving this file to lay out a bundle for
# a machine it is not, passes the name in.
if(NOT DEFINED LFX_ARCH_DIR)
  if(WIN32)
    if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(ARM64|arm64|aarch64)$")
      set(LFX_ARCH_DIR "win-arm64")
    else()
      set(LFX_ARCH_DIR "win-x86_64")
    endif()
  elseif(APPLE)
    list(LENGTH CMAKE_OSX_ARCHITECTURES lfx_mac_arch_count)
    if(lfx_mac_arch_count GREATER 1)
      set(LFX_ARCH_DIR "macos-universal")
    elseif(CMAKE_OSX_ARCHITECTURES STREQUAL "x86_64")
      set(LFX_ARCH_DIR "macos-x86_64")
    elseif(CMAKE_OSX_ARCHITECTURES STREQUAL "arm64")
      set(LFX_ARCH_DIR "macos-arm64")
    elseif(CMAKE_SYSTEM_PROCESSOR MATCHES "^(arm64|aarch64)$")
      set(LFX_ARCH_DIR "macos-arm64")
    else()
      set(LFX_ARCH_DIR "macos-x86_64")
    endif()
  else()
    if(CMAKE_SYSTEM_PROCESSOR MATCHES "^(aarch64|arm64)$")
      set(LFX_ARCH_DIR "linux-aarch64")
    else()
      set(LFX_ARCH_DIR "linux-x86_64")
    endif()
  endif()
endif()
set(LFX_ARCH_DIR "${LFX_ARCH_DIR}" CACHE STRING "The architecture directory the payload is laid out under")

set(LFX_BUNDLE_DIR "${CMAKE_BINARY_DIR}/bundles" CACHE PATH "Where the built bundles are laid out")

# The maths library, where the platform keeps one.
find_library(LFX_MATH_LIBRARY m)

message(STATUS "LFX: laying bundles out under ${LFX_BUNDLE_DIR} for ${LFX_ARCH_DIR}")

# -------------------------------------------------------------- one bundle --

# Compile one source file into `<name>.lfx.bundle/Contents/<arch>/<name>.lfx`
# and copy its listing into place.
function(lfx_add_plugin name source manifest)
  set(bundle "${LFX_BUNDLE_DIR}/${name}.lfx.bundle")
  set(payload_dir "${bundle}/Contents/${LFX_ARCH_DIR}")

  # MODULE rather than SHARED: a plugin is loaded by name at run time and
  # nothing ever links against it.
  add_library(${name} MODULE "${source}")
  target_include_directories(${name} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/include"
    "${CMAKE_CURRENT_SOURCE_DIR}/cpp")
  set_target_properties(${name} PROPERTIES
    OUTPUT_NAME "${name}"
    PREFIX ""
    SUFFIX ".lfx"
    C_STANDARD 99
    C_STANDARD_REQUIRED ON
    CXX_STANDARD 17
    CXX_STANDARD_REQUIRED ON
    LIBRARY_OUTPUT_DIRECTORY "${payload_dir}")
  # Default visibility, deliberately. The frozen header declares
  # `lfx_entry_point` with no export attribute - it cannot carry one, being the
  # same declaration a host reads - so hiding symbols by default would hide the
  # one symbol the loader wants. What keeps the surface small is the examples
  # themselves: everything in them but the entry is `static` or in an anonymous
  # namespace.
  # A multi-configuration generator - Visual Studio, Xcode - appends the
  # configuration's own name to the output directory unless each one is set,
  # which would put the payload one folder below where a scan looks.
  foreach(config IN LISTS CMAKE_CONFIGURATION_TYPES)
    string(TOUPPER "${config}" upper)
    set_target_properties(${name} PROPERTIES
      LIBRARY_OUTPUT_DIRECTORY_${upper} "${payload_dir}"
      RUNTIME_OUTPUT_DIRECTORY_${upper} "${payload_dir}")
  endforeach()

  # The listing is copied at configure time and again whenever it changes, so
  # a manifest edited without a rebuild is still the one in the bundle.
  configure_file("${manifest}" "${bundle}/Contents/lfx.toml" COPYONLY)

  # `pow`, `sqrt` and their neighbours live in a library of their own on most
  # Unixes, and a plugin that did not link it loads perfectly well and dies on
  # the first frame - the symbol is resolved lazily, so nothing says so until
  # the host asks for a picture. It is the first fault this template met.
  if(LFX_MATH_LIBRARY)
    target_link_libraries(${name} PRIVATE "${LFX_MATH_LIBRARY}")
  endif()

  if(NOT MSVC)
    target_compile_options(${name} PRIVATE -Wall -Wextra)
  endif()
endfunction()

# ----------------------------------------------------------- the examples --

# Core only, in C: every rule the frozen header asks of a plugin, and three
# lines of maths.
lfx_add_plugin(Exposure
  "${CMAKE_CURRENT_SOURCE_DIR}/examples/exposure/exposure.c"
  "${CMAKE_CURRENT_SOURCE_DIR}/examples/exposure/lfx.toml")

# Core only, in C++, through `cpp/lfx.hpp`: the same rules with the geometry
# that makes a per-pixel effect tile correctly.
lfx_add_plugin(Vignette
  "${CMAKE_CURRENT_SOURCE_DIR}/examples/vignette/vignette.cpp"
  "${CMAKE_CURRENT_SOURCE_DIR}/examples/vignette/lfx.toml")
