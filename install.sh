#!/usr/bin/env bash
# install.sh — one step from a fresh clone to a working `crow`.
#
#     git clone <the crow repository> && cd crow && ./install.sh
#
# Crow compiles your programs to Rust, so the Rust toolchain is a real
# dependency, not just a build-time one. This script installs whatever is
# missing and skips whatever is present:
#
#   1. a C toolchain     macOS: the Xcode Command Line Tools
#                        Linux: gcc through your package manager (needs sudo)
#   2. Rust              rustup, stable, no prompts (Crow needs 1.85 or newer)
#   3. a permanent home  the source is copied to ~/.crow/src — compiled programs
#                        link against crow_runtime, crow_web and crow_db from
#                        there, so the clone can go once this script is done
#   4. the binary        cargo install → ~/.cargo/bin/crow
#   5. proof             a hello project, compiled and run
#
# Re-running on a newer checkout is the upgrade.
#
# Flags
#   --dev        build from this checkout in place (for working on Crow itself);
#                compiled programs then link against this directory, so keep it
#   --quick      skip the hello-program proof
#   --uninstall  remove the crow binary and ~/.crow (Rust stays)
#   --help       this text
#
# Environment
#   CROW_HOME           where the source copy and build cache live (default ~/.crow)
#   CARGO_INSTALL_ROOT  where cargo puts the binary (default ~/.cargo, so ~/.cargo/bin/crow)
set -euo pipefail

say()  { printf '(o.o) %s\n' "$*"; }
step() { printf '\n(o.o) %s\n' "$*"; }
die()  { printf '(x.x) %s\n' "$*" >&2; exit 1; }
usage() { sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; }

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CROW_HOME="${CROW_HOME:-$HOME/.crow}"
BIN_DIR="${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin"
ORIG_PATH="$PATH"
DEV=0; QUICK=0; UNINSTALL=0
for a in "$@"; do
    case "$a" in
        --dev) DEV=1 ;;
        --quick) QUICK=1 ;;
        --uninstall) UNINSTALL=1 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown flag: $a (try --help)" ;;
    esac
done

grep -q '^name = "crow"' "$REPO/Cargo.toml" 2>/dev/null || die "run this from the crow checkout (Cargo.toml not found next to install.sh)"

OS="$(uname -s)"
case "$OS" in
    Darwin|Linux) ;;
    *) die "unsupported platform: $OS — on Windows, run this inside WSL" ;;
esac

# --- uninstall -------------------------------------------------------------
if [ "$UNINSTALL" = 1 ]; then
    export PATH="$BIN_DIR:$PATH"
    if command -v cargo >/dev/null 2>&1; then cargo uninstall crow >/dev/null 2>&1 || true; fi
    rm -f "$BIN_DIR/crow"
    rm -rf "$CROW_HOME"
    say "removed crow and $CROW_HOME — the Rust toolchain stays ('rustup self uninstall' removes it)"
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
if [ "$DEV" = 1 ]; then
    SRC="$REPO"; TARGET="$REPO/target"
    say "dev mode: building $REPO in place — compiled programs will link against this checkout, so keep it where it is"
else
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
        for p in Cargo.toml Cargo.lock LICENSE README.md src crow_runtime crow_web crow_db docs tests; do
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
