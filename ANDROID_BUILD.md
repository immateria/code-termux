# Android Cross-Compilation Guide

This document explains how to build the Code CLI for Android ARM v8 (aarch64-linux-android) target.

## Prerequisites

### 1. Android NDK Installation

**Option A: Using Homebrew (macOS)**
```bash
brew install android-ndk
```

**Option B: Manual Installation**
- Download from: https://developer.android.com/ndk/downloads
- Extract to: `~/Android/Sdk/ndk/<version>` or `/opt/android-ndk`

**Option C: Android Studio**
- Open Android Studio
- Go to SDK Manager → SDK Tools
- Install "NDK (Side by side)" version 26+

### 2. Rust Target Installation

```bash
rustup target add aarch64-linux-android
```

## Quick Start

### Basic Android Build

```bash
cd code-termux
./build-fast.zsh --target android
```

### With Custom NDK Path

```bash
./build-fast.zsh --target android --android-ndk ~/Android/Sdk/ndk/27.0.12077973
```

### Using Environment Variable

```bash
export ANDROID_NDK=/path/to/ndk
./build-fast.zsh --target android
```

## Build Artifacts

Built binaries are located in:
```
code-rs/target/aarch64-linux-android/dev/code       # Debug build
code-rs/target/aarch64-linux-android/release/code   # Release build
```

## Transferring to Device

### Via ADB (Android Debug Bridge)

```bash
# Build for Android
./build-fast.zsh --target android

# Push binary to Termux on Android device
adb push code-rs/target/aarch64-linux-android/dev/code /data/data/com.termux/files/home/

# Connect to device and make executable
adb shell
chmod +x ~/code
./code --help
```

### Via SSH (if SSH server running on device)

```bash
# Build for Android
./build-fast.zsh --target android

# Copy to device
scp code-rs/target/aarch64-linux-android/dev/code user@device:~/
```

## Troubleshooting

### NDK Not Found

If you see: `❌ Android NDK not found`

**Solution:**
1. Install NDK using one of the methods above
2. Set `ANDROID_NDK` environment variable:
   ```bash
   export ANDROID_NDK=/path/to/your/ndk
   ./build-fast.zsh --target android
   ```

### Rust Target Not Installed

If you see: `the aarch64-linux-android target may not be installed`

**Solution:**
```bash
rustup target add aarch64-linux-android
```

### Linker Not Found

If you see: `aarch64-linux-android24-clang not found`

**Solution:**
The NDK prebuilt directory structure is incorrect. Verify:
```bash
ls -la $ANDROID_NDK/toolchains/llvm/prebuilt/darwin-x86_64/bin/aarch64-linux-android24-clang
# Should return a file, not "No such file or directory"
```

If missing, your NDK is incomplete or the version is wrong. Try:
```bash
brew install --force-bottle android-ndk
```

## How It Works

The `build-fast.zsh` script with `--target android` does the following:

1. **Detects NDK**: Searches common installation locations
2. **Validates Tools**: Checks for `aarch64-linux-android24-clang` and `llvm-ar`
3. **Sets Environment**: Configures cargo toolchain env vars
   - `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`
   - `CARGO_TARGET_AARCH64_LINUX_ANDROID_AR`
   - `CC_aarch64_linux_android`
   - `AR_aarch64_linux_android`
4. **Installs Rust Target**: Ensures `aarch64-linux-android` stdlib is available
5. **Builds**: Runs `cargo build --target aarch64-linux-android`

## Supported Profiles

```bash
# Debug build (default)
./build-fast.zsh --target android

# Release build (optimized)
./build-fast.zsh --target android --workspace code release

# Perf build (with debug symbols)
./build-fast.zsh --target android perf
```

## Environment Variables

```bash
# Automatic NDK detection
ANDROID_NDK=/path/to/ndk ./build-fast.zsh --target android

# Disable codex path guard (speeds up builds)
BUILD_FAST_SKIP_CODEX_GUARD=1 ./build-fast.zsh --target android

# Verbose build output
TRACE_BUILD=1 ./build-fast.zsh --target android

# Custom workspace
WORKSPACE=codex ./build-fast.zsh --target android
```

## Notes

- **Target**: Only `aarch64-linux-android` (Android ARM v8) is currently tested
- **API Level**: Default is Android 24 (7.0)
- **Host Support**: Tested on macOS (`darwin-x86_64`), Linux should also work
- **TLS**: Uses rustls-tls (no native OpenSSL dependency needed)
- **File Size**: Debug build ~30-50MB, Release build ~10-15MB

## For Developers

The Android build system automatically:
- Detects and validates NDK installation
- Configures proper linker and archiver tools
- Sets cargo environment variables for cross-compilation
- Installs required Rust targets
- Provides helpful error messages if setup is incomplete

All Android-specific configuration is in [build-fast.zsh](../build-fast.zsh) in the `setup_android_build()` and `detect_android_ndk()` functions.
