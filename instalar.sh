#!/bin/sh
# Compila Ixtli y lo instala para tu usuario (sin sudo):
#   ~/.local/bin/ixtli, su icono y su entrada en el menú de aplicaciones.
set -eu

cd "$(dirname "$0")"

if ! command -v cargo >/dev/null 2>&1; then
    echo "Falta Rust. En Debian/Ubuntu: sudo apt install cargo" >&2
    exit 1
fi

cargo build --release --locked

bin="$HOME/.local/bin"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
icons="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/scalable/apps"
mkdir -p "$bin" "$apps" "$icons"

install -m 755 target/release/ixtli "$bin/ixtli"
install -m 644 assets/ixtli.svg "$icons/ixtli.svg"
# El menú no siempre tiene ~/.local/bin en su PATH: se usa la ruta completa.
sed "s|^Exec=ixtli$|Exec=$bin/ixtli|" assets/ixtli.desktop > "$apps/ixtli.desktop"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true

echo "Listo. Busca «Ixtli» en tu menú de aplicaciones, o ejecuta: $bin/ixtli"
