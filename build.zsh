#!/bin/zsh
# Build script for code with multi-platform and multi-configuration support
# Usage: ./build.zsh platform="android" --release
#        ./build.zsh                      (default: native build)
#        ./build.zsh --release            (default platform, release mode)
#        ./build.zsh platform="android"   (Android, debug mode)

set -eu

# Script configuration
typeset SCRIPT_DIR="${0:a:h}"
typeset WORKSPACE_ROOT="$SCRIPT_DIR"
typeset CODE_RS_DIR="$WORKSPACE_ROOT/code-rs"

# Build configuration variables
typeset -l PLATFORM="${PLATFORM:-native}"
typeset -l BUILD_MODE="debug"
typeset BUILD_FLAGS=()
typeset CARGO_BUILD_FLAGS=()

# Color output
typeset -r RED='\033[0;31m'
typeset -r GREEN='\033[0;32m'
typeset -r YELLOW='\033[1;33m'
typeset -r BLUE='\033[0;34m'
typeset -r NC='\033[0m'  # No Color

# Helper functions
log_info() {
  print "${BLUE}ℹ${NC} $*"
}

log_success() {
  print "${GREEN}✓${NC} $*"
}

log_warn() {
  print "${YELLOW}⚠${NC} $*"
}

log_error() {
  print "${RED}✗${NC} $*" >&2
}

show_usage() {
  cat << 'EOF'
Build script for code - Multi-platform build orchestration

USAGE:
  ./build.zsh [OPTIONS]

OPTIONS:
  platform="<name>"   Target platform (native, android)
                      Default: native

  --release           Build in release mode (optimized, smaller size)
                      Default: debug mode

  --help              Show this help message

EXAMPLES:
  # Build for native platform in debug mode
  ./build.zsh

  # Build for Android in release mode
  ./build.zsh platform="android" --release

  # Build for native platform in release mode
  ./build.zsh --release

  # Build for Android in debug mode
  ./build.zsh platform="android"

SUPPORTED PLATFORMS:
  native              Build for macOS (host system)
  android             Build for Android aarch64 (ARM64)

EOF
}

# Parse command line arguments
parse_args() {
  local arg
  for arg in "$@"; do
    case "$arg" in
      platform=*)
        PLATFORM="${arg#platform=}"
        ;;
      --release)
        BUILD_MODE="release"
        CARGO_BUILD_FLAGS+=("--release")
        ;;
      --debug)
        BUILD_MODE="debug"
        ;;
      --help|-h)
        show_usage
        exit 0
        ;;
      *)
        log_error "Unknown argument: $arg"
        show_usage
        exit 1
        ;;
    esac
  done
}

# Validate platform
validate_platform() {
  case "$PLATFORM" in
    native|android)
      return 0
      ;;
    *)
      log_error "Unknown platform: $PLATFORM"
      echo "Supported platforms: native, android"
      exit 1
      ;;
  esac
}

# Setup Android environment
setup_android_env() {
  log_info "Setting up Android build environment..."
  
  # Check if OpenSSL is built
  if [[ ! -d "/tmp/openssl-android-aarch64" ]]; then
    log_error "OpenSSL not found at /tmp/openssl-android-aarch64"
    log_info "Building OpenSSL for Android..."
    
    local OPENSSL_TMP="/tmp/openssl-1.1.1w"
    if [[ ! -d "$OPENSSL_TMP" ]]; then
      cd /tmp
      curl -sSL https://www.openssl.org/source/openssl-1.1.1w.tar.gz | tar xz
    fi
    
    cd "$OPENSSL_TMP"
    local NDK_ROOT="/opt/homebrew/share/android-ndk"
    local TOOLCHAIN_PATH="$NDK_ROOT/toolchains/llvm/prebuilt/darwin-x86_64"
    
    export CC="$TOOLCHAIN_PATH/bin/aarch64-linux-android24-clang"
    export AR="$TOOLCHAIN_PATH/bin/llvm-ar"
    export PATH="$TOOLCHAIN_PATH/bin:$PATH"
    
    ./Configure android-arm64 --prefix=/tmp/openssl-android-aarch64
    make -j$(sysctl -n hw.ncpu)
    make install
    
    log_success "OpenSSL built successfully"
  fi
  
  # Export Android-specific environment variables
  export OPENSSL_DIR="/tmp/openssl-android-aarch64"
  export ANDROID_NDK_ROOT="/opt/homebrew/share/android-ndk"
  
  local TOOLCHAIN_PATH="$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/darwin-x86_64"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$TOOLCHAIN_PATH/bin/aarch64-linux-android24-clang"
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_AR="$TOOLCHAIN_PATH/bin/llvm-ar"
  
  # Add Android target flags
  BUILD_FLAGS+=("--target" "aarch64-linux-android")
  
  log_success "Android environment ready"
}

# Setup native environment
setup_native_env() {
  log_info "Setting up native build environment..."
  # Native builds don't need special setup
  log_success "Native environment ready"
}

# Validate environment
validate_env() {
  log_info "Validating environment..."
  
  if [[ ! -d "$CODE_RS_DIR" ]]; then
    log_error "code-rs directory not found at $CODE_RS_DIR"
    exit 1
  fi
  
  if ! command -v rustup &> /dev/null; then
    log_error "rustup not found. Please install Rust."
    exit 1
  fi
  
  if ! command -v cargo &> /dev/null; then
    log_error "cargo not found. Please install Rust."
    exit 1
  fi
  
  if [[ "$PLATFORM" == "android" ]]; then
    if ! rustup target list | grep -q "aarch64-linux-android (installed)"; then
      log_warn "aarch64-linux-android target not installed"
      log_info "Installing target..."
      rustup target add aarch64-linux-android
    fi
    
    if [[ ! -d "/opt/homebrew/share/android-ndk" ]]; then
      log_error "Android NDK not found at /opt/homebrew/share/android-ndk"
      log_info "Install with: brew install android-ndk"
      exit 1
    fi
  fi
  
  log_success "Environment validated"
}

# Perform the build
perform_build() {
  local OUTPUT_DIR="$CODE_RS_DIR/target"
  
  if [[ "$PLATFORM" == "android" ]]; then
    OUTPUT_DIR="$OUTPUT_DIR/aarch64-linux-android"
  fi
  
  if [[ "$BUILD_MODE" == "release" ]]; then
    OUTPUT_DIR="$OUTPUT_DIR/release"
  else
    OUTPUT_DIR="$OUTPUT_DIR/debug"
  fi
  
  log_info "Building code for $PLATFORM in $BUILD_MODE mode..."
  log_info "Output will be: $OUTPUT_DIR/code"
  
  cd "$CODE_RS_DIR"
  
  # Build with appropriate flags
  log_info "Running: cargo build --bin code $BUILD_FLAGS $CARGO_BUILD_FLAGS"
  
  if ! rustup run 1.90.0 cargo build \
    --bin code \
    $BUILD_FLAGS \
    $CARGO_BUILD_FLAGS; then
    log_error "Build failed"
    exit 1
  fi
  
  # Verify output
  if [[ ! -f "$OUTPUT_DIR/code" ]]; then
    log_error "Binary not found at $OUTPUT_DIR/code"
    exit 1
  fi
  
  # Get binary info
  local BINARY_SIZE
  BINARY_SIZE=$(ls -lh "$OUTPUT_DIR/code" | awk '{print $5}')
  
  local BINARY_TYPE
  BINARY_TYPE=$(file "$OUTPUT_DIR/code" | cut -d: -f2-)
  
  log_success "Build completed successfully!"
  log_info "Binary size: $BINARY_SIZE"
  log_info "Binary type: $BINARY_TYPE"
  
  # Show platform-specific next steps
  case "$PLATFORM" in
    android)
      log_info ""
      log_info "Android binary ready for deployment to Termux:"
      log_info "  adb push '$OUTPUT_DIR/code' /data/data/com.termux/files/usr/bin/code"
      log_info "  adb shell chmod +x /data/data/com.termux/files/usr/bin/code"
      log_info "  adb shell code --version"
      ;;
    native)
      log_info ""
      log_info "Native binary ready:"
      log_info "  $OUTPUT_DIR/code"
      ;;
  esac
}

# Main execution
main() {
  log_info "Code build system"
  
  # Parse arguments
  parse_args "$@"
  
  # Validate and show configuration
  validate_platform
  log_info "Platform: $PLATFORM"
  log_info "Build mode: $BUILD_MODE"
  
  # Validate build environment
  validate_env
  
  # Setup platform-specific environment
  case "$PLATFORM" in
    android)
      setup_android_env
      ;;
    native)
      setup_native_env
      ;;
  esac
  
  # Perform the build
  perform_build
}

# Run main function with all arguments
main "$@"
