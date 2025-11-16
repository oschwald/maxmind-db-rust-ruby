#!/bin/bash
set -e

echo "Building maxmind-db-rust extension..."

# Auto-detect GCC include path for bindgen
# This is needed because libclang (used by bindgen) needs to find system headers like stdarg.h
if [ "$(uname)" = "Linux" ]; then
    # Find GCC's system include directory
    GCC_VERSION=$(gcc -dumpversion | cut -d. -f1)
    ARCH=$(gcc -dumpmachine)
    GCC_INCLUDE="/usr/lib/gcc/$ARCH/$GCC_VERSION/include"

    if [ -d "$GCC_INCLUDE" ]; then
        export BINDGEN_EXTRA_CLANG_ARGS="-I$GCC_INCLUDE"
        echo "Setting BINDGEN_EXTRA_CLANG_ARGS=$BINDGEN_EXTRA_CLANG_ARGS"
    else
        echo "Warning: Could not find GCC include directory at $GCC_INCLUDE"
    fi
elif [ "$(uname)" = "Darwin" ]; then
    # macOS: Use Xcode SDK paths
    if command -v xcrun >/dev/null 2>&1; then
        SDK_PATH=$(xcrun --show-sdk-path)
        export BINDGEN_EXTRA_CLANG_ARGS="-isysroot $SDK_PATH"
        echo "Setting BINDGEN_EXTRA_CLANG_ARGS=$BINDGEN_EXTRA_CLANG_ARGS"
    fi
fi

# Build the Rust extension
cd ext/maxmind_db_rust

# On macOS, we need to allow undefined symbols (Ruby C API functions)
# which will be resolved when Ruby loads the extension
if [ "$(uname)" = "Darwin" ]; then
    export RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup"
    echo "Setting RUSTFLAGS=$RUSTFLAGS for macOS"
fi

cargo build --release
cd ../..

# Copy to lib directory
mkdir -p lib/maxmind/db

# With workspace Cargo.toml, artifacts are in the workspace target/ directory
# Handle platform-specific library extensions
if [ "$(uname)" = "Darwin" ]; then
    cp target/release/libmaxmind_db_rust.dylib lib/maxmind/db/maxmind_db_rust.bundle
    echo "✓ Build complete! Extension is at lib/maxmind/db/maxmind_db_rust.bundle"
else
    cp target/release/libmaxmind_db_rust.so lib/maxmind/db/maxmind_db_rust.so
    echo "✓ Build complete! Extension is at lib/maxmind/db/maxmind_db_rust.so"
fi
