#!/bin/sh
# Crea el paquete .deb de Ixtli en target/deb/ (para Debian y Ubuntu).
# Solo necesita cargo y dpkg-deb.
set -eu

cd "$(dirname "$0")"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
arch=$(dpkg --print-architecture)
out="target/deb"
root="$out/ixtli_${version}_${arch}"
deb="$root.deb"

cargo build --release --locked

rm -rf "$root"
install -D -m 755 target/release/ixtli "$root/usr/bin/ixtli"
install -D -m 644 assets/ixtli.desktop "$root/usr/share/applications/ixtli.desktop"
install -D -m 644 assets/ixtli.svg "$root/usr/share/icons/hicolor/scalable/apps/ixtli.svg"
# La cámara virtual se carga al arrancar el sistema, con el nombre «Ixtli».
install -D -m 644 empaquetado/modules-load.conf "$root/usr/lib/modules-load.d/ixtli.conf"
install -D -m 644 empaquetado/modprobe.conf "$root/usr/lib/modprobe.d/ixtli.conf"
install -D -m 644 README.md "$root/usr/share/doc/ixtli/README.md"
install -D -m 644 LICENSE "$root/usr/share/doc/ixtli/copyright"
install -D -m 755 empaquetado/postinst "$root/DEBIAN/postinst"

size=$(du -sk "$root/usr" | cut -f1)
cat > "$root/DEBIAN/control" <<CONTROL
Package: ixtli
Version: $version
Architecture: $arch
Maintainer: Uriel Moguel <xaw624@users.noreply.github.com>
Section: video
Priority: optional
Installed-Size: $size
Depends: libc6, libgcc-s1, scrcpy (>= 3.0), adb, v4l2loopback-dkms, pulseaudio-utils, kmod, libxkbcommon0, libwayland-client0 | libx11-6
Recommends: pkexec, linux-headers-amd64 | linux-headers-generic
Homepage: https://github.com/xaw624/ixtli
Description: Usa tu celular Android como cámara web y micrófono
 Ixtli automatiza scrcpy, v4l2loopback y PipeWire/PulseAudio para que la
 cámara y el micrófono de un teléfono Android aparezcan como una cámara y
 un micrófono normales en Meet, Zoom, Teams, OBS y otros programas.
 Funciona por cable USB o por WiFi.
CONTROL

dpkg-deb --root-owner-group --build "$root" "$deb" >/dev/null
echo "Listo: $deb"
echo "Instálalo con: sudo apt install ./$deb"
