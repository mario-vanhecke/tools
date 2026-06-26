#!/usr/bin/env sh
# Install the rag toolkit (rag + md) from the latest GitHub release.
#
# Usage:
#   curl -fsSL https://github.com/mario-vanhecke/tools/raw/main/install.sh | sh
#
# Environment overrides:
#   RAG_VERSION   pin a specific version (default: latest)
#   RAG_PREFIX    install dir (default: ~/.local/bin if writable, else /usr/local/bin)
#   RAG_TOOLS     space-separated list of binaries to install (default: "rag md")

set -eu

REPO="mario-vanhecke/tools"
VERSION="${RAG_VERSION:-latest}"
TOOLS="${RAG_TOOLS:-rag md crawl distill recall}"

red()    { printf "\033[31m%s\033[0m" "$1"; }
green()  { printf "\033[32m%s\033[0m" "$1"; }
yellow() { printf "\033[33m%s\033[0m" "$1"; }
bold()   { printf "\033[1m%s\033[0m"  "$1"; }

err() { red  "error: "; printf "%s\n" "$1" >&2; exit 1; }
ok()  { green "ok    "; printf " %s\n" "$1"; }
note(){ yellow "note  "; printf " %s\n" "$1"; }

# ---------- detect platform ----------
os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"

case "$os" in
  darwin) os_label="macOS"  ;;
  linux)  os_label="Linux"  ;;
  *) err "unsupported OS: $os (this script handles macOS and Linux; Windows uses install.ps1)" ;;
esac

case "$arch" in
  x86_64|amd64)        rust_arch="x86_64"  ;;
  arm64|aarch64)       rust_arch="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac

case "$os-$rust_arch" in
  darwin-aarch64) target="aarch64-apple-darwin"        ;;
  darwin-x86_64)  target="x86_64-apple-darwin"         ;;
  linux-x86_64)   target="x86_64-unknown-linux-gnu"    ;;
  linux-aarch64)  target="aarch64-unknown-linux-gnu"   ;;
  *) err "unsupported platform: $os $arch" ;;
esac

bold "Installing rag toolkit ($TOOLS) for $os_label ($rust_arch)"; printf "\n"

# ---------- pick install prefix ----------
if [ -n "${RAG_PREFIX:-}" ]; then
  prefix="$RAG_PREFIX"
elif [ -d "$HOME/.local/bin" ] || mkdir -p "$HOME/.local/bin" 2>/dev/null; then
  prefix="$HOME/.local/bin"
elif [ -w /usr/local/bin ]; then
  prefix="/usr/local/bin"
else
  err "no writable install directory; set RAG_PREFIX=<dir> or run as a user with write access to /usr/local/bin"
fi
ok "install prefix: $prefix"

# ---------- need curl or wget ----------
if   command -v curl >/dev/null 2>&1; then dl() { curl -fsSL "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then dl() { wget -q -O "$2" "$1"; }
else err "neither curl nor wget found; please install one and retry"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# ---------- install each tool ----------
for tool in $TOOLS; do
  if [ "$VERSION" = "latest" ]; then
    asset_url="https://github.com/${REPO}/releases/latest/download/${tool}-${target}.tar.gz"
  else
    asset_url="https://github.com/${REPO}/releases/download/${VERSION}/${tool}-${target}.tar.gz"
  fi

  ok "downloading $tool: $asset_url"
  if ! dl "$asset_url" "$tmp/$tool.tar.gz"; then
    err "download failed. check that a release exists at $asset_url"
  fi

  toolstage="$tmp/$tool-stage"
  mkdir -p "$toolstage"
  tar -xzf "$tmp/$tool.tar.gz" -C "$toolstage"

  bin_src=""
  for candidate in "$toolstage/$tool" "$toolstage/$tool-${target}/$tool"; do
    [ -f "$candidate" ] && bin_src="$candidate" && break
  done
  [ -z "$bin_src" ] && err "binary '$tool' not found inside the tarball"

  if mv "$bin_src" "$prefix/$tool" 2>/dev/null; then
    :
  elif command -v sudo >/dev/null 2>&1 && [ "$prefix" = "/usr/local/bin" ]; then
    note "elevating with sudo to write to $prefix"
    sudo mv "$bin_src" "$prefix/$tool"
  else
    err "could not move binary to $prefix"
  fi
  chmod +x "$prefix/$tool" 2>/dev/null || sudo chmod +x "$prefix/$tool"
  ok "installed: $prefix/$tool"
done

# ---------- post-install hints ----------
if ! printf "%s" "$PATH" | tr ':' '\n' | grep -qx "$prefix"; then
  note "$prefix is not on your PATH. Add this to your shell rc:"
  printf "        export PATH=\"%s:\$PATH\"\n" "$prefix"
fi

if ! command -v pandoc >/dev/null 2>&1; then
  note "pandoc is not installed. DOCX/EPUB support requires it. Markdown/text vaults work without it."
  printf "        macOS:  brew install pandoc\n"
  printf "        Debian: sudo apt install pandoc\n"
fi

if ! command -v pdftotext >/dev/null 2>&1; then
  note "poppler is not installed. Without it, PDF extraction uses a pure-Rust fallback that fails on some unusual font encodings."
  printf "        macOS:  brew install poppler\n"
  printf "        Debian: sudo apt install poppler-utils\n"
fi

printf "\n"
bold "Next:"; printf "\n"
case " $TOOLS " in
  *" rag "*)
    printf "  rag --version\n"
    printf "  rag init . && rag add <path> && rag index && rag search \"<query>\"\n"
    ;;
esac
case " $TOOLS " in
  *" md "*)
    printf "  md --version\n"
    printf "  md init . && md add <path> && md convert\n"
    ;;
esac
case " $TOOLS " in
  *" crawl "*)
    printf "  crawl --version\n"
    printf "  crawl init . && crawl source add docs local ./docs && crawl run && crawl ls\n"
    ;;
esac
case " $TOOLS " in
  *" distill "*)
    printf "  distill --version\n"
    printf "  distill init && distill build && distill search \"<query>\"\n"
    ;;
esac
case " $TOOLS " in
  *" recall "*)
    printf "  recall --version\n"
    printf "  recall serve knowledge.kb --stdio   # then add it as an MCP server in your harness\n"
    ;;
esac
