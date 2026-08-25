#
# To learn more about a Podspec see http://guides.cocoapods.org/syntax/podspec.html.
# Run `pod lib lint lumit_bridge.podspec` to validate before publishing.
#
Pod::Spec.new do |s|
  s.name             = 'lumit_bridge'
  s.version          = '0.0.1'
  s.summary          = 'A new Flutter FFI plugin project.'
  s.description      = <<-DESC
A new Flutter FFI plugin project.
                       DESC
  s.homepage         = 'http://example.com'
  s.license          = { :file => '../LICENSE' }
  s.author           = { 'Your Company' => 'email@example.com' }

  # This will ensure the source files in Classes/ are included in the native
  # builds of apps using this FFI plugin. Podspec does not support relative
  # paths, so Classes contains a forwarder C file that relatively imports
  # `../src/*` so that the C sources can be shared among all target platforms.
  s.source           = { :path => '.' }
  s.source_files     = 'Classes/**/*'
  s.dependency 'FlutterMacOS'

  s.platform = :osx, '10.11'
  s.pod_target_xcconfig = { 'DEFINES_MODULE' => 'YES' }
  s.swift_version = '5.0'

  s.script_phase = {
    :name => 'Build Rust library',
    # First argument is relative path to the crate, second is the name of the Rust
    # library. The frb template ships `../../rust rust_lib_lumit_flutter`, which
    # assumes its own layout; our crate is a workspace member at
    # crates/lumit-bridge and its cdylib is `lumit_bridge`. Untested — the macOS
    # pass is K-033, still to come — but the template values are certainly wrong.
    :script => 'sh "$PODS_TARGET_SRCROOT/../cargokit/build_pod.sh" ../../../crates/lumit-bridge lumit_bridge',
    :execution_position => :before_compile,
    :input_files => ['${BUILT_PRODUCTS_DIR}/cargokit_phony'],
    # Let XCode know that the static library referenced in -force_load below is
    # created by this build step.
    :output_files => ["${BUILT_PRODUCTS_DIR}/liblumit_bridge.a"],
  }
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    # Flutter.framework does not contain a i386 slice.
    'EXCLUDED_ARCHS[sdk=iphonesimulator*]' => 'i386',
    # ffmpeg@7 is keg-only (deliberately not linked), so its lib directory has to
    # be named outright. Homebrew's prefix differs by architecture — /opt/homebrew
    # on Apple Silicon, /usr/local on Intel — and ld ignores a -L directory that
    # does not exist, so naming both covers either machine.
    #
    # Caveat for K-033 (notarisation and distribution): those dylibs are linked by
    # absolute path, so the produced .app is not relocatable — it runs only on a
    # machine that has the same Homebrew keg installed. Shipping needs the FFmpeg
    # libraries vendored into the bundle and their install names rewritten
    # (@rpath/@executable_path). Recorded here, not solved here.
    #
    # The -framework list below is hand-maintained, and it rots silently: those
    # are the transitive `cargo:rustc-link-lib` directives the Rust crates emit,
    # which Xcode never sees because it links the finished .a rather than
    # invoking cargo's linker. A new Rust dependency that needs another framework
    # fails at link time with undefined symbols, and nothing points back here.
    'OTHER_LDFLAGS' => '-force_load ${BUILT_PRODUCTS_DIR}/liblumit_bridge.a ' \
      '-L/opt/homebrew/opt/ffmpeg@7/lib -L/usr/local/opt/ffmpeg@7/lib ' \
      '-lavcodec -lavdevice -lavfilter -lavformat -lavutil -lswresample -lswscale ' \
      '-framework AudioUnit -framework CoreAudio -framework CoreFoundation ' \
      '-framework IOSurface -framework Foundation -framework QuartzCore ' \
      '-framework Metal -framework CoreGraphics ' \
      '-lobjc -liconv',
  }
end