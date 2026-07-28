export LIBCLANG_PATH=/usr/lib/llvm18/lib
export LLVM_CONFIG_PATH=/usr/lib/llvm18/bin/llvm-config
export FFMPEG_PKG_CONFIG_PATH=/usr/lib/pkgconfig
RUSTC_WRAPPER="" cargo run -p lumit-app
