#!/usr/bin/env zsh
# Fast build script for local development - optimized for speed
# Zsh-native rewrite with improved error handling and Android cross-compile support

emulate -L zsh
setopt err_exit pipe_fail extended_glob

#═══════════════════════════════════════════════════════════════════════════════
# USAGE & HELPERS
#═══════════════════════════════════════════════════════════════════════════════

function usage
{   print -r "Usage: ./build-fast.zsh [options] [env flags]

Options:
  --workspace codex|code|both    Select workspace to build (default: code)
  --target android|<rust-triple> Cross-compile target (android = aarch64-linux-android)
  --android-ndk <path>           Override Android NDK path (auto-detected if not set)
  -h, --help                     Show this help

Environment flags:
  PROFILE=dev-fast|dev|release-prod   Build profile (default: dev-fast)
  TRACE_BUILD=1                       Print toolchain/env and artifact SHA
  KEEP_ENV=0                          Sanitize env for reproducible builds (default skips)
  DETERMINISTIC=1                     Add -C debuginfo=0; promotes to release-prod
  BUILD_FAST_BINS=\"code code-tui\"     Override bins to build (space or comma separated)
  ANDROID_NDK=<path>                  Android NDK location (if not auto-detected)

Examples:
  ./build-fast.zsh                              # Normal build
  ./build-fast.zsh --target android             # Cross-compile for Android
  ./build-fast.zsh --target android run         # Build for Android then run (if on Android)
  TRACE_BUILD=1 ./build-fast.zsh               # Verbose diagnostics
  ./build-fast.zsh perf run                     # Build perf profile and run
  
Android Build:
  Automatically detects NDK from common locations or use ANDROID_NDK env var.
  Configures linker, adds rustls-tls features, sets up cross-compilation."
}

# Fast string trimming using parameter expansion
function trim
{   typeset value result
    
    value=$1
    result=${value##[[:space:]]#}
    result=${result%%[[:space:]]#}
    print -rn -- $result
}

# Hash string using zsh's pure parameter expansion where possible
function hash_string
{   typeset input sha
    
    input=$1
    if (( $+commands[shasum] )); then
      sha=${$(print -rn -- $input | shasum -a 256)[(w)1]}
      print -rn -- $sha
    elif (( $+commands[sha256sum] )); then
      sha=${$(print -rn -- $input | sha256sum)[(w)1]}
      print -rn -- $sha
    else
      print -rn -- $input | python3 -c 'import hashlib, sys; print(hashlib.sha256(sys.stdin.read().encode()).hexdigest())'
    fi
}

# Sanitize cache key for filesystem safety
function sanitize_cache_key
{   typeset raw result
    
    raw=$1
    result=${raw//[^A-Za-z0-9._-]/-}
    
    # Collapse repeated dashes
    while [[ $result == *--* ]]; do
      result=${result//--/-}
    done
    
    result=${result##-#}
    result=${result%%-#}
    [[ -z $result ]] && result="default"
    [[ ${#result} -gt 120 ]] && result=${result[1,120]}
    print -rn -- $result
}

# Check if binary is in target list
function bin_requested
{   typeset needle candidate
    
    needle=$1
    for candidate in $TARGET_BINS; do
      [[ $candidate == $needle ]] && return 0
    done
    return 1
}

# Resolve binary paths based on profile and target
function resolve_bin_path
{   typeset target_root
    
    case $PROFILE in
      dev-fast) BIN_SUBDIR="dev-fast" ;;
      dev)      BIN_SUBDIR="debug" ;;
      *)        BIN_SUBDIR=$PROFILE ;;
    esac
    
    # Add target triple to path if cross-compiling
    [[ -n $BUILD_TARGET ]] && BIN_SUBDIR="${BUILD_TARGET}/${BIN_SUBDIR}"

    target_root=${CARGO_TARGET_DIR:-${REPO_ROOT}/${WORKSPACE_DIR}/target}
    [[ $target_root != /* ]] && target_root=${target_root:a}

    TARGET_DIR_ABS=$target_root
    BIN_CARGO_FILENAME=$CRATE_PREFIX
    BIN_FILENAME=$CRATE_PREFIX
    [[ $PROFILE == perf ]] && BIN_FILENAME="${CRATE_PREFIX}-perf"
    
    BIN_SUBPATH="${BIN_SUBDIR}/${BIN_FILENAME}"
    BIN_CARGO_SUBPATH="${BIN_SUBDIR}/${BIN_CARGO_FILENAME}"
    BIN_PATH="${TARGET_DIR_ABS}/${BIN_SUBPATH}"
    BIN_CARGO_PATH="${TARGET_DIR_ABS}/${BIN_CARGO_SUBPATH}"
    BIN_LINK_PATH="./target/${BIN_SUBPATH}"

    if [[ -n $REPO_TARGET_ABS && $TARGET_DIR_ABS == $REPO_TARGET_ABS ]]; then
      BIN_DISPLAY_PATH="./${WORKSPACE_DIR}/target/${BIN_SUBPATH}"
    else
      BIN_DISPLAY_PATH=$BIN_PATH
    fi
}

# Parse Cargo.toml for package name (pure zsh, no external commands)
function extract_toml_package_name
{   typeset toml_file line name_value
    integer package_section
    
    toml_file=$1
    package_section=0
    
    [[ ! -f $toml_file ]] && return 1
    
    while IFS= read -r line; do
      line=${line%%[[:space:]]#}
      line=${line##[[:space:]]#}
      
      # Check for [package] section
      if [[ $line == '[package]' ]]; then
        package_section=1
        continue
      fi
      
      # Stop if we hit another section
      if (( package_section == 1 )) && [[ $line == '['* ]]; then
        break
      fi
      
      # Extract name field
      if (( package_section == 1 )) && [[ $line == name[[:space:]]#=* ]]; then
        name_value=${line#*=}
        name_value=${name_value##[[:space:]]#}
        name_value=${name_value%%[[:space:]]#}
        name_value=${name_value#\"}
        name_value=${name_value%\"}
        print -rn -- $name_value
        return 0
      fi
    done < $toml_file
    
    return 1
}

# Extract bin name from exec/Cargo.toml [[bin]] section
function extract_toml_bin_name
{   typeset toml_file line name_value
    integer bin_section
    
    toml_file=$1
    bin_section=0
    
    [[ ! -f $toml_file ]] && return 1
    
    while IFS= read -r line; do
      line=${line%%[[:space:]]#}
      line=${line##[[:space:]]#}
      
      if [[ $line == '[[bin]]' ]]; then
        bin_section=1
        continue
      fi
      
      if (( bin_section == 1 )) && [[ $line == '['* ]]; then
        break
      fi
      
      if (( bin_section == 1 )) && [[ $line == name[[:space:]]#=* ]]; then
        name_value=${line#*=}
        name_value=${name_value##[[:space:]]#}
        name_value=${name_value%%[[:space:]]#}
        name_value=${name_value#\"}
        name_value=${name_value%\"}
        print -rn -- $name_value
        return 0
      fi
    done < $toml_file
    
    return 1
}

# Detect Android NDK from common locations
function detect_android_ndk
{   typeset candidate
    typeset -a ndk_candidates
    
    # Check environment first
    [[ -n ${ANDROID_NDK:-} && -d $ANDROID_NDK ]] && { print -rn -- $ANDROID_NDK; return 0 }
    [[ -n ${ANDROID_NDK_HOME:-} && -d $ANDROID_NDK_HOME ]] && { print -rn -- $ANDROID_NDK_HOME; return 0 }
    
    # Common NDK locations (including Homebrew on macOS)
    ndk_candidates=(
      /opt/homebrew/share/android-ndk(-/N)
      ~/Android/Sdk/ndk/*(-/DN[1])
      ~/Android/ndk/*(-/DN[1])
      ~/android-ndk-r*(-/DN[1])
      /opt/android-ndk(-/N)
      /usr/local/android-ndk(-/N)
    )
    
    for candidate in $ndk_candidates; do
      if [[ -d $candidate/toolchains/llvm/prebuilt ]]; then
        print -rn -- $candidate
        return 0
      fi
    done
    
    return 1
}

# Configure Android build environment
function setup_android_build
{   typeset ndk_root prebuilt_dir host_tag linker ar cargo_target_upper
    
    ndk_root=$(detect_android_ndk)
    if [[ -z $ndk_root ]]; then
      print -u2 "❌ Android NDK not found. Set ANDROID_NDK or install NDK."
      print -u2 "   Common locations: ~/Android/Sdk/ndk/<version>, ~/Android/ndk/<version>"
      print -u2 "   Or download from: https://developer.android.com/ndk/downloads"
      exit 1
    fi
    
    print "🤖 Android NDK: ${ndk_root}"
    
    # Detect host platform for NDK prebuilt tools
    case ${OSTYPE} in
      linux*) host_tag="linux-x86_64" ;;
      darwin*) host_tag="darwin-x86_64" ;;
      *) print -u2 "❌ Unsupported host platform for Android NDK: ${OSTYPE}"; exit 1 ;;
    esac
    
    prebuilt_dir="${ndk_root}/toolchains/llvm/prebuilt/${host_tag}"
    if [[ ! -d $prebuilt_dir ]]; then
      print -u2 "❌ NDK prebuilt tools not found: ${prebuilt_dir}"
      exit 1
    fi
    
    # Set up linker and ar for aarch64-linux-android24
    linker="${prebuilt_dir}/bin/aarch64-linux-android24-clang"
    ar="${prebuilt_dir}/bin/llvm-ar"
    
    if [[ ! -x $linker ]]; then
      print -u2 "❌ Android linker not found: ${linker}"
      exit 1
    fi
    
    if [[ ! -x $ar ]]; then
      print -u2 "❌ Android ar not found: ${ar}"
      exit 1
    fi
    
    # Configure cargo for Android target with proper environment variable names
    cargo_target_upper=${BUILD_TARGET:u:gs/-/_/}
    
    # Set linker and ar for rustc
    export "CARGO_TARGET_${cargo_target_upper}_LINKER"=$linker
    export "CARGO_TARGET_${cargo_target_upper}_AR"=$ar
    
    # Set C compiler variables for cc-rs
    export "CC_${cargo_target_upper}"=$linker
    export "AR_${cargo_target_upper}"=$ar
    
    # Disable OpenSSL for Android (not needed, using rustls-tls)
    export OPENSSL_NO_PKG_CONFIG=1
    export OPENSSL_STATIC=1
    
    # Preserve Android runtime environment variables
    export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}"
    export LD_PRELOAD="${LD_PRELOAD:-}"
    
    # Add Android-specific features for crates that need them
    export CARGO_BUILD_TARGET=$BUILD_TARGET
    
    print "   Linker: ${linker:t}"
    print "   Target: ${BUILD_TARGET}"
    
    # Install target if not present
    typeset installed_targets
    installed_targets=$(rustup target list)
    if [[ $installed_targets != *"${BUILD_TARGET} (installed)"* ]]; then
      print "   Installing Rust target: ${BUILD_TARGET}..."
      rustup target add $BUILD_TARGET
    fi
}

# Build cache fingerprint collection
function collect_fingerprint
{   typeset cargo_v rustc_v host uname_srm
    
    cargo_v=$(CARGO_HOME=$CARGO_HOME RUSTUP_HOME=$RUSTUP_HOME ${=USE_CARGO} -V 2>/dev/null || true)
    rustc_v=$(rustup run $TOOLCHAIN rustc -vV 2>/dev/null || true)
    host=${${(M)${(f)rustc_v}:#host:*}#host: }
    uname_srm=$(uname -srm 2>/dev/null || true)
    
    print "profile=${PROFILE}
toolchain=${TOOLCHAIN:-}
build_target=${BUILD_TARGET:-}
host=${host}
cargo_bin=${REAL_CARGO_BIN}
rustc_bin=${REAL_RUSTC_BIN}
cargo_version=${cargo_v}
rustc_version=${rustc_v:gs/\n/ /}
uname=${uname_srm}
RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-}
CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-}
RUSTFLAGS=${RUSTFLAGS:-}
RUSTC_WRAPPER=${RUSTC_WRAPPER:-}
CARGO_BUILD_RUSTC_WRAPPER=${CARGO_BUILD_RUSTC_WRAPPER:-}
SCCACHE=${SCCACHE:-}
CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-}
MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-}
CODE_HOME=${CODE_HOME:-}
CODEX_HOME=${CODEX_HOME:-}"
}

# Create CLI symlinks helper
function create_cli_symlinks
{   typeset cli_dir default_target link_target dest PREFIX
    
    cli_dir=$1
    default_target=$2
    
    mkdir -p $cli_dir
    link_target=$default_target
    [[ -n ${CLI_LINK_ABSOLUTE:-} ]] && link_target=$CLI_LINK_ABSOLUTE
    
    for PREFIX in $SYMLINK_PREFIXES; do
      dest="${cli_dir}/${PREFIX}-${TRIPLE}"
      [[ -e $dest ]] && rm -f $dest
      ln -sf $link_target $dest
    done
    
    for PREFIX in $SYMLINK_PREFIXES; do
      dest="${cli_dir}/${PREFIX}-aarch64-apple-darwin"
      [[ -e $dest ]] && rm -f $dest
      ln -sf $link_target $dest
    done
}

#═══════════════════════════════════════════════════════════════════════════════
# MAIN SCRIPT
#═══════════════════════════════════════════════════════════════════════════════

# Global variable declarations
integer RUN_AFTER_BUILD KEEP_ENV PROFILE_ENV_SUPPLIED PROFILE_EXPLICIT CANONICAL_ENV_APPLIED
integer PRIMARY_PRESENT FPRINT_CHANGED
typeset ARG_PROFILE WORKSPACE_CHOICE BUILD_TARGET ANDROID_NDK_PATH SCRIPT_DIR CALLER_CWD
typeset REPO_NAME WORKTREE_PARENT REPO_ROOT CACHE_HOME WORKSPACE_DIR CRATE_PREFIX
typeset WORKSPACE_PATH TARGET_CACHE_ROOT WORKTREE_ROOT CACHE_KEY CACHE_KEY_SOURCE
typeset BRANCH_NAME_RAW short_sha BRANCH_HASH BRANCH_HASH_SHORT WORKTREE_HASH
typeset WORKTREE_HASH_SHORT CACHE_KEY_RAW TARGET_CACHE_DIR TARGET_CACHE_DIR_ABS
typeset CLI_PACKAGE TUI_PACKAGE EXEC_PACKAGE EXEC_BIN bin_candidate candidate
typeset PRIMARY_BIN PROFILE_VALUE PROFILE DET_FORCE_REL ORIGINAL_PROFILE USE_CARGO
typeset TOOLCHAIN PROFILE_UPPER CLEAN_RUSTFLAGS REAL_CARGO_BIN REAL_RUSTC_BIN
typeset TRIPLE USE_LOCKED FPRINT_FILE NEW_FPRINT_TEXT
typeset NEW_FPRINT_HASH OLD_FPRINT_HASH PERF_SOURCE PERF_TARGET PERF_DIR
typeset release_link_target dev_fast_link_target CLI_TARGET_CODE CLI_TARGET_CODEX
typeset CLI_LINK_ABSOLUTE BIN_DIR BIN_DIR_ABS BIN_NAME BIN_TARGET_PATH TMP_BIN_PATH
typeset RUN_BIN_PATH ABS_BIN_PATH BIN_SHA size RUN_PATH RUN_STATUS
typeset cargo_toml_content line
typeset -a PASSTHROUGH_ARGS TARGET_BINS CARGO_BIN_ARGS SYMLINK_PREFIXES

# Parse arguments - declare variables at top
[[ ${1:-} == (-h|--help) ]] && { usage; exit 0 }

RUN_AFTER_BUILD=0
ARG_PROFILE=""
WORKSPACE_CHOICE=${WORKSPACE:-}
BUILD_TARGET=""
ANDROID_NDK_PATH=""

while (( $# > 0 )); do
  case $1 in
    run)
      RUN_AFTER_BUILD=1
      PASSTHROUGH_ARGS+=($1)
      ;;
    --workspace)
      shift || { print -u2 "Error: --workspace requires a value."; usage; exit 1 }
      WORKSPACE_CHOICE=$1
      ;;
    --workspace=*)
      WORKSPACE_CHOICE=${1#*=}
      ;;
    --target)
      shift || { print -u2 "Error: --target requires a value."; usage; exit 1 }
      if [[ $1 == android ]]; then
        BUILD_TARGET="aarch64-linux-android"
      else
        BUILD_TARGET=$1
      fi
      ;;
    --target=*)
      if [[ ${1#*=} == android ]]; then
        BUILD_TARGET="aarch64-linux-android"
      else
        BUILD_TARGET=${1#*=}
      fi
      ;;
    --android-ndk)
      shift || { print -u2 "Error: --android-ndk requires a value."; usage; exit 1 }
      ANDROID_NDK_PATH=$1
      ;;
    --android-ndk=*)
      ANDROID_NDK_PATH=${1#*=}
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      if [[ -n $ARG_PROFILE ]]; then
        print -u2 "Error: Multiple profile arguments provided ('${ARG_PROFILE}' and '$1')."
        usage
        exit 1
      fi
      ARG_PROFILE=$1
      PASSTHROUGH_ARGS+=($1)
      ;;
  esac
  shift
done

# Set defaults
[[ -z $WORKSPACE_CHOICE ]] && WORKSPACE_CHOICE="code"
[[ $ARG_PROFILE == pref ]] && ARG_PROFILE="perf"
[[ -n $ANDROID_NDK_PATH ]] && export ANDROID_NDK=$ANDROID_NDK_PATH

# Resolve repository paths
SCRIPT_DIR=${0:A:h}

if [[ -n ${CODE_CALLER_CWD:-} ]]; then
  CALLER_CWD=${CODE_CALLER_CWD:a} || {
    print -u2 "Error: CODE_CALLER_CWD is not a valid directory: ${CODE_CALLER_CWD}"
    exit 1
  }
else
  CALLER_CWD=$PWD
fi

if [[ $SCRIPT_DIR == */.code/working/*/branches/* ]]; then
  WORKTREE_PARENT=${SCRIPT_DIR%/branches/*}
  REPO_NAME=${WORKTREE_PARENT:t}
else
  REPO_NAME=${SCRIPT_DIR:t}
fi

REPO_ROOT=$SCRIPT_DIR

# Guard against codex path dependencies
if [[ ${BUILD_FAST_SKIP_CODEX_GUARD:-0} != 1 ]]; then
  print "Running codex path dependency guard..."
  (
    cd $REPO_ROOT
    scripts/check-codex-path-deps.sh
  )
fi

# Handle --workspace both
if [[ $WORKSPACE_CHOICE == both ]]; then
  if (( RUN_AFTER_BUILD )); then
    print -u2 "Error: --workspace both cannot be combined with 'run'."
    exit 1
  fi
  for ws in codex code; do
    WORKSPACE=$ws $0 $PASSTHROUGH_ARGS --workspace $ws
  done
  exit 0
fi

# Determine cache home
if [[ -n ${CODE_HOME:-} ]]; then
  CACHE_HOME=${CODE_HOME%/}
elif [[ -n ${CODEX_HOME:-} ]]; then
  CACHE_HOME=${CODEX_HOME%/}
else
  if [[ -d /mnt/data && -w /mnt/data ]]; then
    CACHE_HOME="/mnt/data/.code"
  else
    CACHE_HOME="${REPO_ROOT}/.code"
  fi
fi

[[ $CACHE_HOME != /* ]] && CACHE_HOME="${REPO_ROOT}/${CACHE_HOME#./}"

# Select workspace
case $WORKSPACE_CHOICE in
  codex|codex-rs)
    WORKSPACE_DIR="codex-rs"
    CRATE_PREFIX="codex"
    ;;
  code|code-rs)
    WORKSPACE_DIR="code-rs"
    CRATE_PREFIX="code"
    ;;
  *)
    print -u2 "Error: Unknown workspace '${WORKSPACE_CHOICE}'. Use codex, code, or both."
    exit 1
    ;;
esac

WORKSPACE_PATH="${SCRIPT_DIR}/${WORKSPACE_DIR}"
[[ ! -d $WORKSPACE_PATH ]] && {
  print -u2 "Error: Workspace directory '${WORKSPACE_PATH}' not found."
  exit 1
}

TARGET_CACHE_ROOT="${CACHE_HOME}/working/_target-cache/${REPO_NAME}"

# Change to workspace
cd $WORKSPACE_PATH

# Determine cache key
WORKTREE_ROOT=${$(git rev-parse --show-toplevel 2>/dev/null):-$PWD}

if [[ -z ${BUILD_FAST_CACHE_KEY:-} ]]; then
  if git rev-parse --is-inside-work-tree &>/dev/null; then
    BRANCH_NAME_RAW=${$(git rev-parse --abbrev-ref HEAD 2>/dev/null):-HEAD}
    if [[ $BRANCH_NAME_RAW == HEAD ]]; then
      short_sha=${$(git rev-parse --short HEAD 2>/dev/null):-$(date +%Y%m%d%H%M%S)}
      BRANCH_NAME_RAW="detached-${short_sha}"
    fi
  else
    BRANCH_NAME_RAW="unknown"
  fi
  
  BRANCH_HASH=$(hash_string $BRANCH_NAME_RAW)
  BRANCH_HASH_SHORT=${BRANCH_HASH[1,12]}
  WORKTREE_HASH=$(hash_string $WORKTREE_ROOT)
  WORKTREE_HASH_SHORT=${WORKTREE_HASH[1,12]}
  CACHE_KEY_RAW="${BRANCH_NAME_RAW}-${BRANCH_HASH_SHORT}-${WORKTREE_HASH_SHORT}"
  CACHE_KEY_SOURCE="branch/worktree"
else
  CACHE_KEY_RAW=$BUILD_FAST_CACHE_KEY
  CACHE_KEY_SOURCE="override"
fi

# Add target to cache key if cross-compiling
[[ -n $BUILD_TARGET ]] && CACHE_KEY_RAW="${CACHE_KEY_RAW}-${BUILD_TARGET}"

CACHE_KEY=$(sanitize_cache_key $CACHE_KEY_RAW)
[[ -z $CACHE_KEY ]] && CACHE_KEY="default"

TARGET_CACHE_DIR="${TARGET_CACHE_ROOT}/${CACHE_KEY}/${WORKSPACE_DIR}"

if [[ -z ${CARGO_TARGET_DIR:-} ]]; then
  TARGET_CACHE_DIR_ABS=$TARGET_CACHE_DIR
  [[ $TARGET_CACHE_DIR_ABS != /* ]] && TARGET_CACHE_DIR_ABS="${REPO_ROOT}/${TARGET_CACHE_DIR_ABS#./}"
  mkdir -p $TARGET_CACHE_DIR_ABS 2>/dev/null
  export CARGO_TARGET_DIR=$TARGET_CACHE_DIR_ABS
else
  TARGET_CACHE_DIR_ABS=$CARGO_TARGET_DIR
fi

print "Cache bucket: ${CACHE_KEY} (${CACHE_KEY_SOURCE})"
[[ -n $BUILD_TARGET ]] && print "Build target: ${BUILD_TARGET}"

# Extract package names using pure zsh
CLI_PACKAGE=$(extract_toml_package_name cli/Cargo.toml) || CLI_PACKAGE="code-cli"
TUI_PACKAGE=$(extract_toml_package_name tui/Cargo.toml) || TUI_PACKAGE="code-tui"
EXEC_PACKAGE=$(extract_toml_package_name exec/Cargo.toml) || EXEC_PACKAGE="code-exec"
CRATE_PREFIX=${CLI_PACKAGE%%-*}

EXEC_BIN=$(extract_toml_bin_name exec/Cargo.toml) || EXEC_BIN=$EXEC_PACKAGE

# Determine target binaries
if [[ -n ${BUILD_FAST_BINS:-} ]]; then
  for raw_bin in ${(s:,:)BUILD_FAST_BINS} ${=BUILD_FAST_BINS}; do
    bin_candidate=$(trim $raw_bin)
    [[ -n $bin_candidate ]] && TARGET_BINS+=($bin_candidate)
  done
fi
(( ${#TARGET_BINS} == 0 )) && TARGET_BINS=($CRATE_PREFIX)

# Ensure primary binary is first
PRIMARY_PRESENT=0
for candidate in $TARGET_BINS; do
  [[ $candidate == $CRATE_PREFIX ]] && { PRIMARY_PRESENT=1; break }
done
(( ! PRIMARY_PRESENT )) && TARGET_BINS=($CRATE_PREFIX $TARGET_BINS)
PRIMARY_BIN=$TARGET_BINS[1]

# Environment handling
KEEP_ENV=${KEEP_ENV:-1}
PROFILE_ENV_SUPPLIED=0

if [[ -n ${PROFILE+x} ]]; then
  PROFILE_ENV_SUPPLIED=1
  PROFILE_VALUE=$PROFILE
else
  PROFILE_VALUE="dev-fast"
fi

[[ -n $ARG_PROFILE ]] && PROFILE_VALUE=$ARG_PROFILE

PROFILE_EXPLICIT=0
(( PROFILE_ENV_SUPPLIED || ${#ARG_PROFILE} > 0 )) && PROFILE_EXPLICIT=1

PROFILE=$PROFILE_VALUE

# Deterministic build mode
if [[ ${DETERMINISTIC:-} == 1 ]]; then
  print "Deterministic build: enabled"
  DET_FORCE_REL=${DETERMINISTIC_FORCE_RELEASE:-1}
  if [[ $PROFILE == dev-fast && $DET_FORCE_REL == 1 ]]; then
    PROFILE="release-prod"
    print "Deterministic build: switching profile to ${PROFILE}"
  elif [[ $PROFILE == dev-fast ]]; then
    print "Deterministic build: keeping profile ${PROFILE} (DETERMINISTIC_FORCE_RELEASE=0)"
  fi
  
  if (( $+commands[git] )) && git -C $REPO_ROOT rev-parse --is-inside-work-tree &>/dev/null; then
    export SOURCE_DATE_EPOCH=$(git -C $REPO_ROOT log -1 --pretty=%ct 2>/dev/null || true)
  fi
fi

ORIGINAL_PROFILE=$PROFILE
if [[ $PROFILE != (dev|release) ]]; then
  # Check if profile exists in Cargo.toml using pure zsh
  cargo_toml_content=$(<Cargo.toml)
  if [[ $cargo_toml_content != *"[profile.${PROFILE}]"* ]]; then
    case $PROFILE in
      dev-fast)          PROFILE="dev" ;;
      perf|release-prod) PROFILE="release" ;;
      *)                 PROFILE="dev" ;;
    esac
    [[ $ORIGINAL_PROFILE != $PROFILE ]] && print "Profile ${ORIGINAL_PROFILE} not defined in ${WORKSPACE_DIR}/Cargo.toml; falling back to ${PROFILE}."
  fi
fi

# Select cargo toolchain
USE_CARGO="cargo"
if (( $+commands[rustup] )); then
  TOOLCHAIN=${RUSTUP_TOOLCHAIN:-}
  
  if [[ -z $TOOLCHAIN && -f rust-toolchain.toml ]]; then
    # Pure zsh TOML parsing for channel field
    while IFS= read -r line; do
      line=${line##[[:space:]]#}
      line=${line%%[[:space:]]#}
      if [[ $line == channel[[:space:]]#=* ]]; then
        TOOLCHAIN=${line#*=}
        TOOLCHAIN=${TOOLCHAIN##[[:space:]]#}
        TOOLCHAIN=${TOOLCHAIN%%[[:space:]]#}
        TOOLCHAIN=${TOOLCHAIN#\"}
        TOOLCHAIN=${TOOLCHAIN%\"}
        break
      fi
    done < rust-toolchain.toml
  fi
  
  [[ -z $TOOLCHAIN ]] && TOOLCHAIN=${$(rustup show active-toolchain 2>/dev/null)[(w)1]}
  
  if [[ -n $TOOLCHAIN ]]; then
    if ! rustup which rustc --toolchain $TOOLCHAIN &>/dev/null; then
      print "rustup: installing toolchain $TOOLCHAIN ..."
      rustup toolchain install $TOOLCHAIN &>/dev/null
    fi
    USE_CARGO="rustup run $TOOLCHAIN cargo"
    print "Using rustup toolchain: $TOOLCHAIN"
    rustup run $TOOLCHAIN rustc --version 2>/dev/null || true
  else
    print "rustup found but no toolchain detected; using system cargo"
  fi
else
  print -u2 "Error: rustup is required for consistent builds."
  print -u2 "Please install rustup: https://rustup.rs/"
  exit 1
fi

# Set up Android cross-compilation if requested
if [[ -n $BUILD_TARGET ]]; then
  setup_android_build
fi

# Canonicalize environment
CANONICAL_ENV_APPLIED=0
if (( ! KEEP_ENV )); then
  [[ -z ${DETERMINISTIC:-} ]] && export RUSTFLAGS=""
  unset RUSTC_WRAPPER CARGO_BUILD_RUSTC_WRAPPER SCCACHE SCCACHE_BIN
  unset MACOSX_DEPLOYMENT_TARGET CARGO_PROFILE_RELEASE_LTO CARGO_PROFILE_DEV_FAST_LTO
  unset CARGO_PROFILE_RELEASE_CODEGEN_UNITS CARGO_PROFILE_DEV_FAST_CODEGEN_UNITS
  unset CARGO_INCREMENTAL
  CANONICAL_ENV_APPLIED=1
fi

[[ -z ${CARGO_TARGET_DIR:-} ]] && export CARGO_TARGET_DIR=$TARGET_CACHE_DIR_ABS

# Configure sccache
if (( $+commands[sccache] )); then
  [[ -z ${RUSTC_WRAPPER:-} ]] && export RUSTC_WRAPPER=$(whence -p sccache)
  [[ -z ${SCCACHE_DIR:-} ]] && export SCCACHE_DIR="${CACHE_HOME}/sccache"
  [[ -z ${SCCACHE_CACHE_SIZE:-} ]] && export SCCACHE_CACHE_SIZE="50G"
  mkdir -p $SCCACHE_DIR 2>/dev/null
fi

# Debug symbols handling
if [[ ${DEBUG_SYMBOLS:-} == 1 ]]; then
  if [[ $PROFILE == perf ]]; then
    print "Debug symbols: profile 'perf' already preserves debuginfo"
  elif (( ! PROFILE_EXPLICIT )) && [[ $PROFILE == dev-fast ]]; then
    print "Debug symbols requested: switching profile to perf"
    PROFILE="perf"
  else
    PROFILE_UPPER=${PROFILE:u:gs/-/_/}
    typeset -g "CARGO_PROFILE_${PROFILE_UPPER}_DEBUG"=2
    typeset -g "CARGO_PROFILE_${PROFILE_UPPER}_STRIP"=none
    typeset -g "CARGO_PROFILE_${PROFILE_UPPER}_SPLIT_DEBUGINFO"=packed
    export "CARGO_PROFILE_${PROFILE_UPPER}_DEBUG" "CARGO_PROFILE_${PROFILE_UPPER}_STRIP" "CARGO_PROFILE_${PROFILE_UPPER}_SPLIT_DEBUGINFO"
    print "Debug symbols: forcing debuginfo for profile ${PROFILE}"
  fi
  
  if [[ -n ${RUSTFLAGS:-} ]]; then
    CLEAN_RUSTFLAGS=${RUSTFLAGS//-C debuginfo=0/}
    CLEAN_RUSTFLAGS=${${CLEAN_RUSTFLAGS//  / }## }
    CLEAN_RUSTFLAGS=${CLEAN_RUSTFLAGS%% }
    export RUSTFLAGS=$CLEAN_RUSTFLAGS
  fi
  
  export CARGO_PROFILE_RELEASE_STRIP="none"
  export CARGO_PROFILE_RELEASE_PROD_STRIP="none"
fi

print "Building ${CRATE_PREFIX} binary (${PROFILE} mode)..."

# Configure Cargo directories
if [[ ${STRICT_CARGO_HOME:-} == 1 ]]; then
  export CARGO_HOME=${CARGO_HOME_ENFORCED:-${REPO_ROOT}/.cargo-home}
else
  [[ -z ${CARGO_HOME:-} ]] && export CARGO_HOME="${REPO_ROOT}/.cargo-home"
fi

[[ -z ${RUSTUP_HOME:-} ]] && export RUSTUP_HOME="${CARGO_HOME%/}/rustup"
[[ -z ${CARGO_TARGET_DIR:-} ]] && export CARGO_TARGET_DIR="${WORKSPACE_PATH}/target"
mkdir -p $CARGO_HOME $CARGO_TARGET_DIR 2>/dev/null

mkdir -p ./target
typeset REPO_TARGET_ABS=${${:-./target}:a}
resolve_bin_path

export CARGO_REGISTRIES_CRATES_IO_PROTOCOL="sparse"

# Resolve actual cargo/rustc for fingerprinting
REAL_CARGO_BIN=$(rustup which cargo 2>/dev/null || whence -p cargo || print cargo)
REAL_RUSTC_BIN=$(rustup which rustc 2>/dev/null || whence -p rustc || print rustc)

# Determine host triple
TRIPLE=${$(rustup run $TOOLCHAIN rustc -vV 2>/dev/null)[(f)2]}
if [[ -z $TRIPLE ]]; then
  if [[ $(uname -s) == Darwin ]]; then
    TRIPLE="$(uname -m)-apple-darwin"
    [[ $TRIPLE == arm64-apple-darwin ]] && TRIPLE="aarch64-apple-darwin"
  else
    TRIPLE="unknown-unknown-unknown"
  fi
fi

# Check Cargo.lock validity
if CARGO_HOME=$CARGO_HOME RUSTUP_HOME=$RUSTUP_HOME ${=USE_CARGO} metadata --locked --format-version 1 &>/dev/null; then
  USE_LOCKED="--locked"
else
  print "⚠️  Warning: Cargo.lock appears out of date or inconsistent"
  print "  Continuing with unlocked build for development..."
  USE_LOCKED=""
fi

# Trace mode
if [[ ${TRACE_BUILD:-} == 1 ]]; then
  print "--- TRACE_BUILD environment ---"
  print "whoami: $(whoami)"
  print "pwd: $PWD"
  print "SHELL: ${SHELL:-}"
  print "zsh: $(zsh --version)"
  [[ -n ${TOOLCHAIN:-} ]] && {
    print "TOOLCHAIN: ${TOOLCHAIN}"
    rustup run $TOOLCHAIN rustc -vV 2>/dev/null
    rustup run $TOOLCHAIN cargo -vV 2>/dev/null
  }
  print "CANONICAL_ENV_APPLIED: ${CANONICAL_ENV_APPLIED} (KEEP_ENV=${KEEP_ENV})"
  print "BUILD_TARGET: ${BUILD_TARGET:-native}"
  print "--------------------------------"
fi

# Build cache fingerprint
FPRINT_FILE="./target/${PROFILE}/.env-fingerprint"
NEW_FPRINT_TEXT=$(collect_fingerprint)
NEW_FPRINT_HASH=${$(print -rn -- $NEW_FPRINT_TEXT | shasum -a 256 2>/dev/null)[(w)1]}

FPRINT_CHANGED=0
if [[ -f $FPRINT_FILE ]]; then
  # Pure zsh HASH= extraction
  while IFS= read -r line; do
    if [[ $line == HASH=* ]]; then
      OLD_FPRINT_HASH=${line#HASH=}
      break
    fi
  done < $FPRINT_FILE
  
  if [[ ${OLD_FPRINT_HASH:-} != $NEW_FPRINT_HASH ]]; then
    FPRINT_CHANGED=1
    print "⚠️  Build cache fingerprint changed since last run for profile '${PROFILE}'."
    [[ ${TRACE_BUILD:-} == 1 ]] && print "   Run with TRACE_BUILD=1 to see detailed differences."
  fi
fi

# Build
[[ -z $EXEC_BIN ]] && EXEC_BIN="${CRATE_PREFIX}-exec"

for bin in $TARGET_BINS; do
  CARGO_BIN_ARGS+=(--bin $bin)
done

print "Building bins: ${TARGET_BINS[*]}"

# Add --target flag if cross-compiling
if [[ -n $BUILD_TARGET ]]; then
  ${=USE_CARGO} build ${=USE_LOCKED} --profile $PROFILE --target $BUILD_TARGET $CARGO_BIN_ARGS
else
  ${=USE_CARGO} build ${=USE_LOCKED} --profile $PROFILE $CARGO_BIN_ARGS
fi

# Post-build handling
if (( $? == 0 )); then
  resolve_bin_path
  
  if [[ $PROFILE == perf ]]; then
    PERF_SOURCE=$BIN_CARGO_PATH
    PERF_TARGET=$BIN_PATH
    if [[ -e $PERF_SOURCE ]]; then
      PERF_DIR=${PERF_TARGET:h}
      mkdir -p $PERF_DIR
      [[ -e $PERF_TARGET || -L $PERF_TARGET ]] && rm -f $PERF_TARGET
      (
        cd $PERF_DIR
        ln -sf ${PERF_SOURCE:t} ${PERF_TARGET:t}
      )
    fi
  fi
  
  print "✅ Build successful!"
  print "Binary location: ${BIN_DISPLAY_PATH}"
  print ""
  
  # Create symlinks
  release_link_target="../${BIN_SUBDIR}/${BIN_FILENAME}"
  dev_fast_link_target="../${BIN_SUBDIR}/${BIN_FILENAME}"
  
  SYMLINK_PREFIXES=($CRATE_PREFIX)
  [[ $CRATE_PREFIX == code ]] && SYMLINK_PREFIXES+=(coder)
  
  CLI_TARGET_CODE="../../target/${BIN_SUBDIR}/${BIN_FILENAME}"
  CLI_TARGET_CODEX="../../${WORKSPACE_DIR}/target/${BIN_SUBDIR}/${BIN_FILENAME}"
  CLI_LINK_ABSOLUTE=""
  
  if [[ $TARGET_DIR_ABS != $REPO_TARGET_ABS ]]; then
    release_link_target=$BIN_PATH
    dev_fast_link_target=$BIN_PATH
    CLI_LINK_ABSOLUTE=$BIN_PATH
    
    if [[ -n ${BIN_LINK_PATH:-} ]]; then
      mkdir -p ${BIN_LINK_PATH:h}
      [[ -e $BIN_LINK_PATH ]] && rm -f $BIN_LINK_PATH
      ln -sf $BIN_PATH $BIN_LINK_PATH
    fi
  fi
  
  mkdir -p ./target/release
  [[ -e ./target/release/$CRATE_PREFIX ]] && rm -f ./target/release/$CRATE_PREFIX
  ln -sf $release_link_target ./target/release/$CRATE_PREFIX
  
  [[ -d ../codex-cli/bin ]] && create_cli_symlinks ../codex-cli/bin $CLI_TARGET_CODEX
  [[ -d ./code-cli/bin ]] && create_cli_symlinks ./code-cli/bin $CLI_TARGET_CODE
  
  BIN_DIR="./bin"
  mkdir -p $BIN_DIR
  BIN_DIR_ABS=${BIN_DIR:a}
  
  for BIN_NAME in $TARGET_BINS; do
    BIN_TARGET_PATH="${TARGET_DIR_ABS}/${BIN_SUBDIR}/${BIN_NAME}"
    if [[ -e $BIN_TARGET_PATH ]]; then
      TMP_BIN_PATH="${BIN_DIR}/${BIN_NAME}.tmp.$$"
      rm -f $TMP_BIN_PATH 2>/dev/null
      cp -f $BIN_TARGET_PATH $TMP_BIN_PATH
      mv -f $TMP_BIN_PATH ${BIN_DIR}/${BIN_NAME}
      chmod +x ${BIN_DIR}/${BIN_NAME} 2>/dev/null
    fi
  done
  
  RUN_BIN_PATH="${BIN_DIR_ABS}/${PRIMARY_BIN}"
  
  if [[ $PROFILE != dev-fast ]]; then
    mkdir -p ./target/dev-fast
    [[ -e ./target/dev-fast/$CRATE_PREFIX ]] && rm -f ./target/dev-fast/$CRATE_PREFIX
    ln -sf $dev_fast_link_target ./target/dev-fast/$CRATE_PREFIX
  fi
  
  # Compute SHA256
  ABS_BIN_PATH=${BIN_PATH:a}
  BIN_SHA=""
  
  if [[ -e $ABS_BIN_PATH ]]; then
    if (( $+commands[shasum] )); then
      BIN_SHA=${$(shasum -a 256 $ABS_BIN_PATH)[(w)1]}
    elif (( $+commands[sha256sum] )); then
      BIN_SHA=${$(sha256sum $ABS_BIN_PATH)[(w)1]}
    fi
  fi
  
  if [[ -n $BIN_SHA ]]; then
    size=${$(du -sh $ABS_BIN_PATH)[(w)1]}
    print "Binary Hash: ${BIN_SHA} (${size})"
  elif [[ -e $ABS_BIN_PATH ]]; then
    size=${$(du -h $ABS_BIN_PATH)[(w)1]}
    print "Binary Size: ${size}"
  else
    print "Binary artifact not found at ${ABS_BIN_PATH}"
  fi
  
  # Run if requested (only works on native builds or if on Android)
  if (( RUN_AFTER_BUILD )); then
    if [[ -n $BUILD_TARGET ]]; then
      print "⚠️  Cannot run cross-compiled binary for ${BUILD_TARGET}"
      print "   Transfer to target device and run there"
    else
      RUN_PATH=$RUN_BIN_PATH
      [[ ! -x $RUN_PATH ]] && RUN_PATH="${TARGET_DIR_ABS}/${BIN_SUBDIR}/${PRIMARY_BIN}"
      if [[ ! -x $RUN_PATH ]]; then
        print "❌ Run failed: ${RUN_PATH} is missing or not executable"
        exit 1
      fi
      print "Running ${RUN_PATH} (cwd: ${CALLER_CWD})..."
      ( cd $CALLER_CWD && $RUN_PATH )
      RUN_STATUS=$?
      if (( RUN_STATUS != 0 )); then
        print "❌ Run failed with status ${RUN_STATUS}"
        exit $RUN_STATUS
      fi
    fi
  fi
  
  # Persist fingerprint
  mkdir -p ./target/$PROFILE 2>/dev/null
  print "HASH=${NEW_FPRINT_HASH}\n${NEW_FPRINT_TEXT}" > $FPRINT_FILE
  (( FPRINT_CHANGED )) && print "🧰 Cache normalized to current environment (fingerprint ${NEW_FPRINT_HASH})."
  
  [[ -z $USE_LOCKED ]] && {
    print ""
    print "⚠️  Built without --locked due to Cargo.lock issues"
  }
else
  print "❌ Build failed"
  exit 1
fi
