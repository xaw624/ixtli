# Plan de Ixtli

Notas para retomar el proyecto en otra sesión.

## Dónde está hoy (v0.1, 9 de noviembre de 2026)

Funciona y está probado con un Galaxy A57 (Android 16) en Debian 13 con KDE/Wayland:

- Cámara frontal o trasera por USB, en 720p o 1080p, hacia una cámara virtual.
- Micrófono del teléfono como micrófono virtual.
- Giro: automático, cuartos de vuelta, espejo y ángulo personalizado.
- Vista previa incrustada en la ventana y desacoplable (`ixtli vista`).
- Modo terminal: `ixtli estado`, `ixtli encender`, `ixtli --encender`.

Cómo está hecho:

- Rust con iced 0.13 (renderizador tiny-skia). Rust viene de apt en Debian 13
  (cargo 1.85), así que las dependencias deben seguir compilando con esa versión.
- Por debajo usa scrcpy 3.0 o más nuevo, v4l2loopback y `pactl`.
- Módulos: `engine.rs` (hilo que maneja la sesión), `session.rs` (scrcpy),
  `sys.rs` (teléfono y cámara virtual), `audio.rs` (micrófono virtual),
  `preview.rs` (lee la cámara virtual), `gui.rs` y `cli.rs`.
- Se instala con `./instalar.sh` en `~/.local/bin/ixtli`.

Cosas que conviene saber:

- El widget `svg` de iced dibuja los iconos fuera de lugar con escala fraccionaria
  de pantalla; por eso los iconos se dibujan con resvg y se muestran como imagen.
- Cada cambio de giro u opción reabre scrcpy y la imagen se corta unos 2 segundos.
- El botón «Activar» de la cámara virtual (pkexec + modprobe) no se ha probado,
  porque en las pruebas el módulo ya estaba cargado.

## Siguientes pasos

### 1. Conexión por WiFi

Objetivo: usar el teléfono sin cable.

- scrcpy ya funciona por red con `adb tcpip 5555` y `adb connect IP:5555`
  (el primer emparejamiento necesita el cable) o con la depuración inalámbrica de
  Android 11 en adelante (`adb pair`).
- En la ventana: un botón «Conectar por WiFi» que, con el teléfono por USB, active
  el modo de red, lea la IP del teléfono y se conecte; y que recuerde el teléfono
  para reconectarse sin cable la siguiente vez.
- `sys::phone()` ya toma el primer dispositivo listo de `adb devices`; hay que
  decidir cuál se prefiere cuando el mismo teléfono aparece por USB y por red.
- Listo cuando: se enciende la cámara sin cable desde la ventana y, si se pierde
  la red, Ixtli lo avisa con un mensaje claro.

### 2. Instalador para que otros no tengan que compilar

Objetivo: instalar Ixtli sin tener Rust.

- Opción A, paquete `.deb` (por ejemplo con `cargo-deb`): puede declarar como
  dependencias `scrcpy`, `adb`, `v4l2loopback-dkms` y `pulseaudio-utils`. Es lo más
  directo para Debian y Ubuntu.
- Opción B, Flatpak: llega a más distribuciones, pero el sandbox complica usar
  `adb`, `pkexec`/`modprobe` y `/dev/video*`. Dejarlo para después del `.deb`.
- Publicar el binario y el `.deb` en GitHub Releases, compilados con GitHub Actions.
- Listo cuando: en un Debian limpio se instala con un solo archivo y abre desde el menú.

### 3. Pendientes menores

- Probar el botón «Activar» de la cámara virtual después de reiniciar la computadora.
- Cargar v4l2loopback al arrancar el sistema (opcional), para no pedir contraseña.
- Probar en otro teléfono y en otra distribución.
- README en inglés.

## Más adelante

- App propia para Android (Kotlin, para Play Store) que reemplace a scrcpy: giro
  instantáneo, sin depuración USB. Las cuentas personales nuevas de Play Store
  necesitan una prueba cerrada con testers antes de publicar.
- Windows: cámara virtual con la API de Windows 11; micrófono con un cable de audio
  virtual ya existente.
