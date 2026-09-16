#!/usr/bin/env bash
set -euo pipefail

source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
for binary in orchest orchest-api; do
  if [[ ! -f "$source_dir/$binary" ]]; then
    echo "Missing $binary next to install-linux.sh" >&2
    exit 1
  fi
done

install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"
install -m 0755 "$source_dir/orchest" "$install_dir/orchest"
install -m 0755 "$source_dir/orchest-api" "$install_dir/orchest-api"

path_line='export PATH="$HOME/.local/bin:$PATH"'
for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
  if [[ "$rc" != "$HOME/.profile" && ! -f "$rc" ]]; then
    continue
  fi
  if [[ ! -f "$rc" ]] || ! grep -Fqx "$path_line" "$rc"; then
    printf '\n# Orchest command-line tools\n%s\n' "$path_line" >> "$rc"
  fi
done

echo "Installed Orchest commands in $install_dir"
echo 'Open a new terminal, then run: orchest init'
