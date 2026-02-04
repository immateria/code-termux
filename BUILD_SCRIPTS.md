# Build Scripts Quick Reference

## Build Fast (build-fast.zsh)

**Zsh-native rewrite with:**
- ✅ All variables declared at function tops
- ✅ No external commands (sed/awk/grep) - uses zsh parameter expansion
- ✅ Android cross-compilation support
- ✅ Automatic NDK detection
- ✅ Proper error handling and diagnostics

### Quick Commands

```bash
# Standard native build
./build-fast.zsh

# With specific profile
./build-fast.zsh release-prod
./build-fast.zsh perf

# Android ARM v8 build
./build-fast.zsh --target android

# Both workspaces
./build-fast.zsh --workspace both

# Verbose diagnostics
TRACE_BUILD=1 ./build-fast.zsh

# Build and run
./build-fast.zsh dev run

# Cross-compile
./build-fast.zsh --target android
./build-fast.zsh --target aarch64-unknown-linux-musl
```

### Options

```bash
--workspace codex|code|both      # Select workspace (default: code)
--target <triple>               # Cross-compile target (e.g., android, aarch64-unknown-linux-musl)
--android-ndk <path>            # Override NDK path
-h, --help                       # Show help
```

### Environment Variables

```bash
PROFILE=dev-fast|dev|release-prod    # Build profile (default: dev-fast)
BUILD_FAST_BINS="code code-tui"      # Override which binaries to build
TRACE_BUILD=1                         # Verbose output
DETERMINISTIC=1                       # Deterministic builds (promotes to release-prod)
DEBUG_SYMBOLS=1                       # Include debug symbols
KEEP_ENV=0                            # Sanitize environment for reproducibility
BUILD_FAST_SKIP_CODEX_GUARD=1        # Skip codex path dependency check
ANDROID_NDK=/path/to/ndk             # Override NDK location
```

### Examples

```bash
# Development build
./build-fast.zsh

# Release build with optimizations
./build-fast.zsh release-prod

# Performance profile with debug symbols
./build-fast.zsh perf

# Android cross-compile
./build-fast.zsh --target android

# Verbose build with NDK override
ANDROID_NDK=~/Android/Sdk/ndk/27.0.12077973 TRACE_BUILD=1 ./build-fast.zsh --target android

# Both workspaces, specific profile
./build-fast.zsh --workspace both dev

# Build and immediately run
./build-fast.zsh release run
```

## Pre-Release Script (pre-release.zsh)

**Simple validation wrapper that:**
- ✅ Runs fast build
- ✅ Executes tests
- ✅ Validates release readiness

### Usage

```bash
./pre-release.zsh
```

## Build Output Locations

Native builds:
```
code-rs/target/debug/code          # Debug (dev-fast profile)
code-rs/target/release/code        # Release
code-rs/bin/code                   # Symlink to current binary
```

Android builds:
```
code-rs/target/aarch64-linux-android/debug/code    # Debug
code-rs/target/aarch64-linux-android/release/code  # Release
```

## Cache Bucket System

Build script uses intelligent caching:
- Cache key based on git branch + worktree hash + target
- Separate target directories per cache bucket
- Example: `main-0d6e4079e367-bd2f9194c9b3-aarch64-linux-android`

Set custom cache key:
```bash
BUILD_FAST_CACHE_KEY=my-custom-key ./build-fast.zsh
```

## Profile Comparison

| Profile      | Speed | Optimization | Use Case |
|--------------|-------|--------------|----------|
| dev-fast     | ⚡⚡⚡ | Minimal      | Local development |
| dev          | ⚡⚡  | Low          | Debugging |
| perf         | ⚡   | High + debug | Performance analysis |
| release      | ⚡   | Maximum      | Production |
| release-prod | ⚡   | Maximum + LTO| Final releases |

## Troubleshooting

### Build hangs
```bash
BUILD_FAST_SKIP_CODEX_GUARD=1 ./build-fast.zsh
```

### Environment issues
```bash
KEEP_ENV=0 ./build-fast.zsh  # Sanitize environment
```

### See detailed info
```bash
TRACE_BUILD=1 ./build-fast.zsh  # Toolchain info, build fingerprints
```

### Android NDK not found
```bash
# Install NDK
brew install android-ndk

# Or set path
export ANDROID_NDK=/path/to/ndk
./build-fast.zsh --target android
```

## File Sizes

Approximate binary sizes:
- Debug build: 30-50 MB
- Release build: 10-15 MB
- Android debug: 25-40 MB (aarch64-linux-android)

## Integration

The build script integrates with:
- **rustup** - Toolchain management
- **cargo** - Build system
- **sccache** - Optional build caching (if available)
- **git** - Branch detection for cache keys
- **Android NDK** - Cross-compilation tools (optional)

---

📖 For more details, see:
- [ANDROID_BUILD.md](ANDROID_BUILD.md) - Android build guide
- [ANDROID_BUILD_STATUS.md](ANDROID_BUILD_STATUS.md) - Status and next steps
- [code-rs/README.md](code-rs/README.md) - Rust workspace info
