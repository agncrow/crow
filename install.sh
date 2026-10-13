#!/usr/bin/env bash
# install.sh — Crow in one line.
#
#     curl -fsSL https://raw.githubusercontent.com/agncrow/crow/main/install.sh | bash
#
# Fetches the prebuilt crow binary for this machine from the latest release
# into ~/.crow/bin, held to its published hash, and prints the PATH line.
# Nothing else is installed: the interpreter, the tests, crow check, the
# language server and served apps work at once. The first compiled build
# fetches the Rust toolchain and the runtime, once — `crow setup` does it
# up front, `crow setup --check` says what is present.
#
# From a checkout (git clone https://github.com/agncrow/crow && cd crow):
#
#   ./install.sh --source   build crow from this checkout and snapshot the
#                           source to ~/.crow/src for compiled programs to
#                           link; installs a C toolchain and Rust if missing
#   ./install.sh --dev      build this checkout in place, for working on Crow
#                           itself; compiled programs link against it, so
#                           keep it where it is
#
# Flags
#   --version X   the release to fetch (default: the latest)
#   --quick       skip the proof (a hello program)
#   --uninstall   remove ~/.crow and the binary (Rust, if any, stays)
#   --help        this text
#
# Environment
#   CROW_HOME            where Crow lives (default ~/.crow)
#   CROW_DIST_URL        where a release's files are fetched from (default
#                        https://github.com/agncrow/crow/releases/download/v<version>)
#   CARGO_INSTALL_ROOT   --source and --dev: where cargo puts the binary (default ~/.cargo)

say()  { printf '(o.o) %s\n' "$*"; }
step() { printf '\n(o.o) %s\n' "$*"; }
die()  { printf '(x.x) %s\n' "$*" >&2; exit 1; }
usage() { sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")" 2>/dev/null && pwd)"
CROW_HOME="${CROW_HOME:-$HOME/.crow}"
ORIG_PATH="$PATH"
MODE=fetch; DEV=0; QUICK=0; UNINSTALL=0; VERSION=""
while [ $# -gt 0 ]; do
    case "$1" in
        --source) MODE=source ;;
        --dev) MODE=dev; DEV=1 ;;
        --version) shift; VERSION="${1:-}"; [ -n "$VERSION" ] || die "--version takes a release, e.g. --version 0.6.0" ;;
        --version=*) VERSION="${1#--version=}" ;;
        --quick) QUICK=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown flag: $1 (try --help)" ;;
    esac
    shift
done
if [ "$MODE" = fetch ]; then
    BIN_DIR="$CROW_HOME/bin"
else
    BIN_DIR="${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin"
    grep -q '^name = "crow"' "$REPO/Cargo.toml" 2>/dev/null || die "--$MODE builds from a checkout: run this from the crow checkout (Cargo.toml not found next to install.sh)"
fi

OS="$(uname -s)"
case "$OS" in
    Darwin|Linux) ;;
    *) die "unsupported platform: $OS — on Windows, run this inside WSL" ;;
esac

# --- uninstall -------------------------------------------------------------
if [ "$UNINSTALL" = 1 ]; then
    export PATH="$BIN_DIR:$PATH"
    if command -v cargo >/dev/null 2>&1; then cargo uninstall crow >/dev/null 2>&1 || true; fi
    rm -f "$BIN_DIR/crow" "${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin/crow"
    rm -rf "$CROW_HOME"
    say "removed crow and $CROW_HOME — a Rust toolchain of your own stays ('rustup self uninstall' removes it)"
    exit 0
fi

# --- the one line: a prebuilt binary, verified -----------------------------
if [ "$MODE" = fetch ]; then
    command -v curl >/dev/null 2>&1 || die "curl is needed to fetch crow — install it and re-run"
    case "$OS-$(uname -m)" in
        Darwin-arm64) TRIPLE=aarch64-apple-darwin ;;
        Darwin-x86_64) TRIPLE=x86_64-apple-darwin ;;
        Linux-x86_64) TRIPLE=x86_64-unknown-linux-gnu ;;
        Linux-aarch64|Linux-arm64) TRIPLE=aarch64-unknown-linux-gnu ;;
        *) die "no prebuilt crow for $OS $(uname -m) — ./install.sh --source builds it from a checkout" ;;
    esac
    if [ -z "$VERSION" ]; then
        [ -n "${CROW_DIST_URL:-}" ] && die "--version is needed with CROW_DIST_URL"
        VERSION="$(curl -fsSL https://api.github.com/repos/agncrow/crow/releases/latest | sed -n 's/.*"tag_name": *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1)"
        [ -n "$VERSION" ] || die "cannot read the latest release from api.github.com — pass --version X"
    fi
    BASE="${CROW_DIST_URL:-https://github.com/agncrow/crow/releases/download/v$VERSION}"
    ASSET="crow-$VERSION-$TRIPLE.tar.gz"
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/crow-install.XXXXXX")"
    step "crow $VERSION for $TRIPLE from $BASE"
    curl -fL --retry 3 --progress-bar -o "$tmp/$ASSET" "$BASE/$ASSET" || die "cannot fetch $BASE/$ASSET"
    curl -fsSL -o "$tmp/$ASSET.sha256" "$BASE/$ASSET.sha256" || die "cannot fetch the hash $BASE/$ASSET.sha256"
    expected="$(awk '{print $1}' "$tmp/$ASSET.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then seen="$(sha256sum "$tmp/$ASSET" | awk '{print $1}')"; else seen="$(shasum -a 256 "$tmp/$ASSET" | awk '{print $1}')"; fi
    [ "$seen" = "$expected" ] || die "$ASSET does not match its published hash (expected $expected, saw $seen)"
    mkdir -p "$BIN_DIR"
    tar -xzf "$tmp/$ASSET" -C "$BIN_DIR"
    chmod +x "$BIN_DIR/crow"
    rm -rf "$tmp" "$CROW_HOME/dev"
    say "installed $BIN_DIR/crow ($("$BIN_DIR/crow" -v | awk '{print $NF}'), verified)"
    if [ "$QUICK" = 1 ]; then
        say "proof skipped (--quick)"
    else
        tmp="$(mktemp -d "${TMPDIR:-/tmp}/crow-install.XXXXXX")"
        out="$( cd "$tmp" && "$BIN_DIR/crow" new hello >/dev/null && cd hello && "$BIN_DIR/crow" run -i 2>&1 )" || {
            printf '%s\n' "$out" >&2
            die "the hello program did not run — the output above is the reason"
        }
        rm -rf "$tmp"
        case "$out" in
            *"hello, hello"*) say "crow new hello && crow run -i → hello, hello" ;;
            *) printf '%s\n' "$out" >&2; die "the hello program ran but printed something unexpected" ;;
        esac
    fi
    printf '\n'
    "$BIN_DIR/crow" setup --check
    printf '\n'
    case ":$ORIG_PATH:" in
        *":$BIN_DIR:"*) ;;
        *) say "add it to your PATH:  export PATH=\"$BIN_DIR:\$PATH\"" ;;
    esac
    say "next:  crow new hello && cd hello && crow run    (crow setup fetches the compiler for native builds, once)"
    exit 0
fi

# --- 1. C toolchain --------------------------------------------------------
step "1/5 C toolchain"
have_cc() { command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1; }
if [ "$OS" = Darwin ]; then
    if xcode-select -p >/dev/null 2>&1; then
        say "Xcode Command Line Tools present"
    else
        say "installing the Xcode Command Line Tools — accept the dialog that opens"
        xcode-select --install >/dev/null 2>&1 || true
        waited=0
        until xcode-select -p >/dev/null 2>&1; do
            sleep 10; waited=$((waited + 10))
            [ "$waited" -ge 3600 ] && die "the Command Line Tools installer did not finish — run 'xcode-select --install', then re-run this script"
        done
        say "Command Line Tools installed"
    fi
else
    if have_cc; then
        say "C compiler present"
    else
        SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO="sudo"
        say "installing a C compiler with your package manager${SUDO:+ (sudo may ask for your password)}"
        if command -v apt-get >/dev/null 2>&1; then
            $SUDO apt-get update -qq && $SUDO apt-get install -y -qq build-essential curl ca-certificates
        elif command -v dnf >/dev/null 2>&1; then
            $SUDO dnf install -y gcc gcc-c++ make curl
        elif command -v yum >/dev/null 2>&1; then
            $SUDO yum install -y gcc gcc-c++ make curl
        elif command -v pacman >/dev/null 2>&1; then
            $SUDO pacman -Sy --noconfirm --needed base-devel curl
        elif command -v zypper >/dev/null 2>&1; then
            $SUDO zypper install -y gcc gcc-c++ make curl
        elif command -v apk >/dev/null 2>&1; then
            $SUDO apk add --no-cache build-base curl
        else
            die "no C compiler and no package manager I recognise — install gcc or clang, then re-run"
        fi
        have_cc || die "a C compiler still isn't on PATH after installing — check your package manager's output"
    fi
fi

# --- 2. Rust ---------------------------------------------------------------
step "2/5 Rust toolchain"
export PATH="$BIN_DIR:$HOME/.cargo/bin:$PATH"
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
need_rust=1
if command -v cargo >/dev/null 2>&1; then
    ver="$(cargo --version | awk '{print $2}')"
    maj="${ver%%.*}"; rest="${ver#*.}"; min="${rest%%.*}"
    if [ "$maj" -gt 1 ] || { [ "$maj" -eq 1 ] && [ "$min" -ge 85 ]; }; then
        say "cargo $ver present"; need_rust=0
    elif command -v rustup >/dev/null 2>&1; then
        say "cargo $ver is older than 1.85 (Crow uses the 2024 edition) — updating stable"
        rustup update stable
        rustup default stable >/dev/null 2>&1 || true
        need_rust=0
    else
        say "cargo $ver is older than 1.85 and not managed by rustup — installing rustup alongside it"
    fi
fi
if [ "$need_rust" = 1 ]; then
    command -v curl >/dev/null 2>&1 || die "curl is needed to fetch rustup — install it and re-run"
    say "installing Rust with rustup (stable, minimal profile, no prompts)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
    RUST_WAS_INSTALLED=1
fi
say "rustc $(rustc --version | awk '{print $2}'), cargo $(cargo --version | awk '{print $2}')"

# --- 3. source snapshot ----------------------------------------------------
step "3/5 source"
mkdir -p "$CROW_HOME"
if [ "$DEV" = 1 ]; then
    SRC="$REPO"; TARGET="$REPO/target"
    # the binary reads home/dev to find the checkout (src/home.rs)
    printf '%s\n' "$REPO" > "$CROW_HOME/dev"
    say "dev mode: building $REPO in place — compiled programs will link against this checkout, so keep it where it is"
else
    rm -f "$CROW_HOME/dev"
    SRC="$CROW_HOME/src"; TARGET="$CROW_HOME/target"
    rm -rf "$SRC"; mkdir -p "$SRC"
    if git -C "$REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        # every file git sees — tracked or untracked, not ignored — as it is on disk
        ( cd "$REPO" && git ls-files -z --cached --others --exclude-standard --deduplicate \
            | while IFS= read -r -d '' f; do [ -e "$f" ] && printf '%s\0' "$f"; done \
            | tar --null -T - -cf - ) | tar -xf - -C "$SRC"
        # the lockfile is not tracked, but a checkout's resolution is a tested one
        [ -f "$REPO/Cargo.lock" ] && cp "$REPO/Cargo.lock" "$SRC/Cargo.lock"
    else
        for p in Cargo.toml Cargo.lock LICENSE-MIT LICENSE-APACHE README.md src crow_runtime crow_web crow_db docs tests; do
            [ -e "$REPO/$p" ] && cp -R "$REPO/$p" "$SRC/"
        done
        rm -rf "$SRC/target" "$SRC/.crow_build" "$SRC/docs/book/crow-site"
    fi
    say "copied to $SRC ($(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo 'not a git checkout'))"
fi

# --- 4. build and install --------------------------------------------------
step "4/5 building crow — a release build, a few minutes the first time"
# with a lockfile, build exactly that resolution (no re-resolving, no surprise downloads)
LOCKED=""; [ -f "$SRC/Cargo.lock" ] && LOCKED="--locked"
cargo install --path "$SRC" --target-dir "$TARGET" --force --quiet $LOCKED
hash -r 2>/dev/null || true
command -v crow >/dev/null 2>&1 || die "crow was built but $BIN_DIR is not on PATH in this shell"
say "installed crow $(crow -v | awk '{print $NF}') at $(command -v crow)"

# --- 5. proof --------------------------------------------------------------
step "5/5 proof: a hello program, compiled and run"
if [ "$QUICK" = 1 ]; then
    say "skipped (--quick)"
else
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/crow-install.XXXXXX")"
    out="$( cd "$tmp" && crow new hello >/dev/null && cd hello && CROW_BUILD_PROFILE=dev crow run 2>&1 )" || {
        printf '%s\n' "$out" >&2
        die "the hello program did not compile — the output above is the reason"
    }
    rm -rf "$tmp"
    case "$out" in
        *"hello, hello"*) say "crow new hello && crow run → hello, hello" ;;
        *) printf '%s\n' "$out" >&2; die "the hello program ran but printed something unexpected" ;;
    esac
fi

# --- done ------------------------------------------------------------------
printf '\n'
say "done. crow lives at $(command -v crow)"
case ":$ORIG_PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        if [ "${RUST_WAS_INSTALLED:-0}" = 1 ]; then
            say "open a new terminal (rustup added $BIN_DIR to your shell profile), or run:  source \"\$HOME/.cargo/env\""
        else
            say "add it to your PATH:  export PATH=\"$BIN_DIR:\$PATH\""
        fi
        ;;
esac
say "next:  crow new hello && cd hello && crow run"
