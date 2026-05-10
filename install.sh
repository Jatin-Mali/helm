#!/usr/bin/env sh
set -eu

repo="${HELM_REPO:-https://github.com/Jatin-Mali/helm}"
version="${HELM_VERSION:-latest}"
install_dir="${HELM_INSTALL_DIR:-$HOME/.local/bin}"

mkdir -p "$install_dir"

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64)   target="x86_64-unknown-linux-gnu" ;;
  aarch64|arm64)  target="aarch64-unknown-linux-gnu" ;;
  *) echo "unsupported architecture: $arch" >&2; exit 2 ;;
esac

if [ "$version" = "latest" ]; then
  url="$repo/releases/latest/download/helm-$target"
else
  url="$repo/releases/download/$version/helm-$target"
fi

print_source_build_help() {
  cat >&2 <<EOF
release asset unavailable for $repo ($version, $target)

build from source instead:
  git clone https://github.com/Jatin-Mali/helm.git
  cd helm
  cargo build --release -p helm-cli
  ./target/release/helm init
  ./target/release/helm doctor

if this fork starts publishing release assets for your architecture later, this
installer will work without changes.
EOF
}

tmp="$(mktemp)"
cleanup() { rm -f "$tmp"; }
trap cleanup EXIT

echo "downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
  print_source_build_help
  exit 1
fi
chmod +x "$tmp"
mv "$tmp" "$install_dir/helm"
echo "installed $install_dir/helm"

# Add install_dir to PATH if it isn't already there
add_to_path() {
  shell_rc="$1"
  if [ -f "$shell_rc" ] && grep -qF "$install_dir" "$shell_rc" 2>/dev/null; then
    return 0
  fi
  printf '\nexport PATH="%s:$PATH"\n' "$install_dir" >> "$shell_rc"
  echo "added $install_dir to PATH in $shell_rc"
}

case "${SHELL:-}" in
  */zsh)  add_to_path "$HOME/.zshrc" ;;
  */fish)
          mkdir -p "$HOME/.config/fish"
          fish_rc="$HOME/.config/fish/config.fish"
          if [ ! -f "$fish_rc" ] || ! grep -qF "$install_dir" "$fish_rc" 2>/dev/null; then
            printf '\nfish_add_path "%s"\n' "$install_dir" >> "$fish_rc"
            echo "added $install_dir to PATH in ~/.config/fish/config.fish"
          fi
          ;;
  *)      add_to_path "$HOME/.bashrc" ;;
esac

if ! command -v helm >/dev/null 2>&1 || [ "$(command -v helm)" != "$install_dir/helm" ]; then
  echo ""
  echo "  NOTE: $install_dir is not in your current PATH."
  echo "  Either restart your shell or run:"
  echo "    export PATH=\"$install_dir:\$PATH\""
fi

echo ""
echo "  Next: helm init"
