# Build Script Documentation

## Overview

The `build.zsh` script provides a unified way to build the code project for multiple platforms and configurations. It handles all the complexity of setting up platform-specific environments (NDK, OpenSSL, environment variables) automatically.

## Quick Start

### Android Release Build
```bash
./build.zsh platform="android" --release
```

Output: `49MB ARM aarch64 stripped binary` ready for Termux

### Android Debug Build
```bash
./build.zsh platform="android"
```

Output: `848MB ARM aarch64 unstripped binary with debug symbols`

### Native Release Build
```bash
./build.zsh --release
```

Output: Native macOS binary in release mode

### Native Debug Build
```bash
./build.zsh
```

Output: Native macOS binary in debug mode

## Complete Usage

```
USAGE:
  ./build.zsh [OPTIONS]

OPTIONS:
  platform="<name>"   Target platform (native, android)
                      Default: native

  --release           Build in release mode (optimized, smaller size)
                      Default: debug mode

  --debug             Explicitly set debug mode (default)

  --help              Show help message
```

## Features

✅ **Automatic Environment Setup**
- Detects and validates build environment
- Installs missing Rust targets automatically
- Builds OpenSSL from source if needed (Android only)

✅ **Platform-Specific Configuration**
- Native: Standard Rust build
- Android: NDK cross-compilation with custom OpenSSL

✅ **Build Mode Support**
- Debug: Full symbols, slower execution, larger binary (~800-900MB)
- Release: Optimized, stripped, smaller binary (~50MB)

✅ **Helpful Output**
- Color-coded status messages
- Binary size and type information
- Platform-specific deployment instructions

## Build Outputs

### Android Builds
- **Debug**: `code-rs/target/aarch64-linux-android/debug/code` (848MB)
- **Release**: `code-rs/target/aarch64-linux-android/release/code` (49MB)

### Native Builds
- **Debug**: `code-rs/target/debug/code`
- **Release**: `code-rs/target/release/code`

## Environment Setup

The script automatically handles:

### For Android Builds:
1. **Validates Android NDK** at `/opt/homebrew/share/android-ndk`
2. **Builds OpenSSL 1.1.1w** for aarch64-linux-android (if not present)
3. **Installs Rust target** `aarch64-linux-android` (if not present)
4. **Configures environment variables**:
   - `OPENSSL_DIR`: Points to custom-built OpenSSL
   - `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER`: NDK clang compiler
   - `CARGO_TARGET_AARCH64_LINUX_ANDROID_AR`: NDK llvm-ar archiver

### For Native Builds:
- Uses standard macOS build environment
- No special setup required

## Deployment Examples

### Deploy to Termux (Android)
```bash
# Build release binary
./build.zsh platform="android" --release

# Copy to device
adb push code-rs/target/aarch64-linux-android/release/code /data/data/com.termux/files/usr/bin/code

# Make executable
adb shell chmod +x /data/data/com.termux/files/usr/bin/code

# Test
adb shell code --version
```

### Run Locally (Native)
```bash
# Build release binary
./build.zsh --release

# Run
code-rs/target/release/code --version
```

## Requirements

### Universal Requirements
- macOS (tested on macOS 14+)
- Rust (via rustup)
- zsh shell

### For Android Builds
- Android NDK (install via `brew install android-ndk`)
- curl (for downloading OpenSSL if needed)

### For Native Builds
- Xcode Command Line Tools (usually pre-installed)

## Troubleshooting

### "OpenSSL not found"
The script will automatically build it. Ensure you have ~2GB free space in /tmp.

### "Android NDK not found"
Install via:
```bash
brew install android-ndk
```

### "aarch64-linux-android target not installed"
The script installs it automatically if needed.

### Build still fails?
Check that:
1. Rust is up to date: `rustup update`
2. You have a recent version of zsh
3. You have sufficient disk space
4. For Android: NDK is installed at `/opt/homebrew/share/android-ndk`

## Build Times

Typical build times on Apple Silicon Mac:
- **First build**: 2-3 minutes (dependencies compiled)
- **Subsequent builds**: 30-60 seconds (incremental)
- **Release builds**: +8-10 minutes (first time), +1-2 minutes (incremental)

## File Size Comparison

| Configuration | Size | Symbols | Use Case |
|---|---|---|---|
| Android Debug | 848MB | Included | Development, debugging |
| Android Release | 49MB | Stripped | Production deployment |
| Native Debug | ~600MB | Included | Development |
| Native Release | ~100MB | Stripped | Production |

## Version Control

The script is tracked in git and can be updated with the codebase:
```bash
git pull origin main
```

---

**Last Updated**: February 3, 2026
**Compatible With**: Rust 1.90.0+, Android NDK r29+, zsh 5.0+
