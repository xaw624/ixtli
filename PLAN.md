# Plan de Ixtli

Notas para retomar el proyecto en otra sesión. Actualizado el 10 de noviembre de 2026.

## Dónde está hoy

Funciona en Debian 13 con KDE/Wayland, probado con un Galaxy A57 (Android 16) y un
moto g34 5G:

- Cámara frontal o trasera por USB, en 720p o 1080p, hacia una cámara virtual.
- Micrófono del teléfono como micrófono virtual.
- Giro: automático, cuartos de vuelta, espejo y ángulo personalizado.
- Vista previa incrustada en la ventana y desacoplable (`ixtli vista`).
- Modo terminal: `ixtli estado`, `ixtli encender`, `ixtli --encender`.
- **WiFi (sin commit todavía):** «Conectar por WiFi» con el cable una vez
  (`adb tcpip`), probado con el moto g34; y «Vincular con QR» sin cable
  (`adb pair` + mDNS, módulo `pairing.rs`), probado solo con un adb simulado.

Cómo está hecho:

- Rust con iced 0.13 (renderizador tiny-skia). Rust viene de apt en Debian 13
  (cargo 1.85), así que las dependencias deben seguir compilando con esa versión.
- Por debajo usa scrcpy 3.0 o más nuevo, v4l2loopback y `pactl`.
- Módulos: `engine.rs` (hilo que maneja la sesión), `session.rs` (scrcpy),
  `sys.rs` (teléfono, adb y cámara virtual), `pairing.rs` (vincular con QR),
  `audio.rs` (micrófono virtual), `preview.rs` (lee la cámara virtual), `gui.rs`
  y `cli.rs`.
- Se instala con `./instalar.sh` en `~/.local/bin/ixtli`, o con el `.deb` que arma
  `./empaquetar.sh`.

Cosas que conviene saber:

- El widget `svg` de iced dibuja los iconos fuera de lugar con escala fraccionaria
  de pantalla; por eso los iconos se dibujan con resvg y se muestran como imagen.
- Cada cambio de giro u opción reabre scrcpy y la imagen se corta unos 2 segundos.
- El botón «Activar» de la cámara virtual (pkexec + modprobe) no se ha probado,
  porque en las pruebas el módulo ya estaba cargado.
- El código QR de vinculación hay que escanearlo desde «Depuración inalámbrica» del
  teléfono. La app de cámara lo confunde con una red WiFi.

## Decisiones tomadas (10 de noviembre de 2026)

- **Orden:** cerrar WiFi → instalador `.deb` → ahorro → motor de video propio →
  Windows 10 → varias fuentes.
- **Windows de prueba:** una PC con Windows 10. No tiene la cámara virtual oficial
  (esa es de Windows 11), así que se usa una cámara virtual de tipo DirectShow.
- **Funciones nuevas:** varios teléfonos, pantalla del teléfono como fuente, imagen
  combinada y controles de cámara.
- **Ahorro:** pausa automática, modo ahorro, pantalla apagada y vista previa ligera.

## Mapa de fases

| Fase | Entrega | Depende de | Tamaño | Versión |
|------|---------|------------|--------|---------|
| 0 | WiFi cerrado y con commit | nada | chico | 0.2 |
| 1 | `.deb` y GitHub Releases | 0 | chico | 0.2 |
| 2 | Medición y ahorro rápido | 0 | mediano | 0.3 |
| 3 | Motor de video propio (Linux) | 2 | grande | 0.4 |
| 4 | Windows 10 | 1 y 3 | grande | 0.5 |
| 5 | Varias fuentes y controles | 3 | grande | 0.6 |

La idea que une todo: hoy scrcpy escribe directo en la cámara virtual, y eso solo
existe en Linux (`--v4l2-sink`). Para Windows, la pausa automática, el giro sin corte
y el cambio entre teléfonos hace falta lo mismo: que **Ixtli reciba los cuadros de
video** y decida qué manda a la cámara virtual. Esa pieza es la fase 3, y por eso va
antes de Windows y de las funciones nuevas.

La comprobación manual de Windows (paso W0 de la fase 4) no necesita código y se
puede hacer cualquier día, en paralelo con las fases 0 a 3.

## Fase 0. Cerrar el WiFi

Objetivo: dejar el WiFi probado con teléfonos reales y guardado en git.

- Probar «Vincular con QR» en el moto g34 escaneando desde Depuración inalámbrica.
  Si la tarjeta se queda en «Esperando…», revisar `adb mdns services`: algunas redes
  bloquean los anuncios entre equipos.
- Hecho y probado con el moto g34, también por VPN (Tailscale): vinculación con código de 6 dígitos, en la
  tarjeta de la ventana y con `ixtli vincular IP:PUERTO CÓDIGO`. Si el teléfono no se
  anuncia en la red (VPN), Ixtli pide su «Dirección IP y puerto» y la guarda en
  `telefono.conf` (`direccion=`) para reconectar.
- Probar en vivo: desconectar el cable, apagar el WiFi del teléfono, reabrir Ixtli.
- Probar el botón «Activar» después de reiniciar la computadora.
- Commit.

Listo cuando: los dos teléfonos se conectan sin cable desde la ventana y, si se pierde
la red, Ixtli avisa con un mensaje claro.

## Fase 1. Instalador `.deb` y publicaciones (hecho)

Objetivo: instalar Ixtli sin tener Rust.

Hecho:

- `./empaquetar.sh` arma `target/deb/ixtli_VERSION_amd64.deb` con `dpkg-deb` (no usa
  `cargo-deb`, para no depender de un Rust más nuevo). Depende de `scrcpy (>= 3.0)`,
  `adb`, `v4l2loopback-dkms` y `pulseaudio-utils`.
- El paquete instala `/usr/lib/modules-load.d/ixtli.conf` y
  `/usr/lib/modprobe.d/ixtli.conf` (archivos en `empaquetado/`): la cámara virtual se
  carga al arrancar con la etiqueta «Ixtli» y ya no se pide contraseña.
- `.github/workflows/publicar.yml`: compila y prueba en un contenedor de Debian 13,
  arma el `.deb`, lo instala en otro Debian 13 limpio y, con una etiqueta `v*`, lo
  publica en GitHub Releases junto con el binario suelto.
- README en inglés (`README.en.md`).
- Versión 0.2.0, publicada el 10 de noviembre de 2026 en
  https://github.com/xaw624/ixtli/releases/tag/v0.2.0. El flujo pasó completo,
  incluida la instalación en un Debian 13 limpio.
- Para subir cambios al flujo, la sesión de `gh` necesita el permiso `workflow`
  (`gh auth refresh -h github.com -s workflow`).

Falta:

- Instalar el `.deb` en esta computadora y reiniciar, para ver que la cámara «Ixtli»
  aparece sola. Aquí ya hay una cámara virtual llamada «Celular», cargada a mano.
- Probar en otra distribución (Ubuntu 24.04 trae scrcpy 1.25, que no sirve).

Cosas que conviene saber: en Debian 13, `scrcpy` 3 solo está en `trixie-backports`
(sección `contrib`); el README explica cómo activarlo.

## Fase 2. Medición y ahorro rápido

Objetivo: saber cuánto gasta Ixtli y bajar ese gasto con cambios que no necesitan el
motor propio.

- **Medir primero.** Un comando `ixtli medir` que, con la cámara encendida, anote
  cada minuto: batería y temperatura del teléfono (`adb shell dumpsys battery`), y
  CPU y memoria de Ixtli y de scrcpy en la computadora. Guardar la tabla base (720p y
  1080p, cable y WiFi, los dos teléfonos) en `docs/mediciones.md`.
- **Modo ahorro:** un botón que baja a 480p, 15 o 24 cuadros por segundo y menos tasa
  de bits (`--camera-size`, `--camera-fps`, `--video-bit-rate`). Los tamaños válidos
  se leen de `scrcpy --list-camera-sizes`.
- **Pantalla apagada:** apagar la pantalla del teléfono al encender la cámara. No es
  compatible con el giro automático, que necesita la pantalla prendida: al activarla,
  el giro pasa a fijo.
- **Vista previa ligera:** menos cuadros por segundo y se detiene sola cuando la
  ventana está minimizada o tapada.
- **Batería a la vista:** nivel de batería del teléfono en la ventana y un aviso para
  conectar el cable cuando baje de 20 %.
- **Elegir lente:** lista de cámaras del teléfono (`scrcpy --list-cameras`,
  `--camera-id`) para usar el gran angular u otro lente.
- **Sondeos más lentos:** hoy se pregunta el giro cada 1.5 s y, por WiFi, se hace un
  «ping» cada 3 s; cada pregunta arranca un proceso en el teléfono. Espaciarlos.

Listo cuando: la tabla de mediciones muestra el antes y el después, y el modo ahorro
gasta claramente menos batería que 720p normal.

## Fase 3. Motor de video propio (en Linux)

Objetivo: que Ixtli reciba el video y lo entregue él mismo a la cámara virtual. En
Linux no cambia nada a la vista, salvo que el giro deja de cortar la imagen.

1. **Hablar con el servidor de scrcpy sin el programa scrcpy:** copiar al teléfono el
   `scrcpy-server` (una versión fija, empaquetada con Ixtli; licencia Apache 2.0),
   abrir el túnel con `adb forward` y pedir el video crudo en H.264
   (`raw_stream=true`).
2. **Decodificar a imágenes.** Empezar con `ffmpeg` como proceso aparte, que es lo más
   rápido de probar en los dos sistemas; medir CPU y retraso, y solo si hace falta
   cambiarlo por un decodificador dentro de Ixtli.
3. **Salida:** escribir los cuadros en v4l2loopback con un tamaño fijo, rellenando
   con negro lo que sobre.
4. **Giro, espejo y ángulo dentro de Ixtli,** sin reabrir la cámara.
5. **Pausa automática:** v4l2loopback avisa cuántos programas están usando la cámara.
   Sin ninguno durante unos segundos, Ixtli apaga la cámara del teléfono y manda una
   imagen fija de «en pausa»; al abrirla alguien, la reanuda (tarda unos 2 segundos).
6. **Vista previa desde el motor,** en lugar de leer la cámara virtual. La red caída
   se detecta porque dejan de llegar cuadros, sin «pings».
7. El micrófono sigue como hoy (scrcpy solo con audio y `pactl`).

El camino actual con scrcpy se conserva como respaldo (`--motor=scrcpy`) hasta que el
nuevo iguale todo.

Listo cuando: todo lo de la versión 0.3 funciona con el motor nuevo, cambiar el giro
no corta la imagen, la cámara del teléfono se apaga sola cuando nadie la usa y las
mediciones son iguales o mejores que las de la fase 2.

Riesgos: el protocolo del servidor cambia entre versiones de scrcpy (por eso se fija
y se empaqueta una); el retraso del decodificador externo.

## Fase 4. Windows 10

Objetivo: una versión que se pueda bajar y probar en la PC con Windows 10.

Qué cambia respecto a Linux:

| Pieza | Linux | Windows 10 |
|-------|-------|------------|
| Cámara virtual | v4l2loopback | Cámara DirectShow (biblioteca Softcam, MIT) |
| Micrófono virtual | `pactl` | VB-CABLE, que instala el usuario |
| adb y servidor | paquetes del sistema | incluidos junto a `ixtli.exe` |
| Permisos | `pkexec` al cargar el módulo | administrador una vez, al instalar |
| Procesos hijos | `prctl` | Job Objects, sin ventana de consola |
| Configuración | `~/.config/ixtli` | `%APPDATA%\ixtli` |

Pasos:

- **W0. Comprobación manual, sin programar.** En la PC con Windows 10 (64 bits,
  versión 22H2): bajar scrcpy para Windows y ver la cámara del teléfono con
  `scrcpy --video-source=camera`; instalar OBS y comprobar que «OBS Virtual Camera»
  aparece en Meet y Zoom. Confirma que adb ve el teléfono (por cable puede faltar el
  controlador USB del fabricante) y que las cámaras DirectShow sirven en esa PC.
- **W1. Que abra.** Separar lo que depende del sistema en una capa de plataforma
  (`plat/linux.rs`, `plat/windows.rs`). Compilar en GitHub Actions (`windows-latest`)
  y publicar `ixtli-windows.zip`. La ventana abre, detecta el teléfono y vincula por
  QR.
- **W2. Video.** El motor de la fase 3 entrega los cuadros a la cámara DirectShow
  «Ixtli». Probar en Meet (Chrome y Edge), Zoom, Teams y OBS.
- **W3. Micrófono.** Ixtli detecta si VB-CABLE está instalado y, si falta, explica
  cómo instalarlo. El audio del teléfono se reproduce en «CABLE Input» y en la
  videollamada se elige «CABLE Output».
- **W4. Instalador.** Un instalador que registra la cámara virtual, crea el acceso en
  el menú Inicio y permite desinstalar.

Listo cuando: en la PC con Windows 10 se instala con un archivo, el teléfono se
vincula por QR y la cámara y el micrófono funcionan en una videollamada de Meet.

Limitaciones conocidas:

- Las cámaras DirectShow no aparecen en la app «Cámara» de Windows ni en algunas apps
  de la tienda. En navegadores, Zoom, Teams, Discord y OBS sí.
- El `.exe` no está firmado: Windows mostrará «Windows protegió su PC» y hay que
  elegir «Ejecutar de todas formas». Firmarlo cuesta dinero; queda para después.
- La primera vez, el Firewall de Windows preguntará si adb puede usar la red privada.

## Fase 5. Varias fuentes y controles

Objetivo: más de un teléfono, la pantalla como fuente e imagen combinada.

- **5a. Varios teléfonos.** Recordar varios teléfonos, lista para elegir, miniatura
  de cada uno y cambio con un clic o una tecla. Dos modos:
  - *Ahorro* (por defecto): solo transmite el teléfono que está al aire; cambiar
    tarda unos 2 segundos.
  - *Cambio instantáneo*: todos transmiten; gasta la batería de todos.
- **5b. Pantalla del teléfono** como fuente (`video_source=display`). Avisar que las
  notificaciones del teléfono se verán en la videollamada.
- **5c. Imagen combinada:** cuadro chico sobre otro, o lado a lado. Salida limitada a
  720p porque es lo que más CPU usa.
- **5d. Controles de cámara:** zoom digital (recortar y ampliar en la computadora).
  La linterna y el zoom óptico no existen en scrcpy 3.3.4; revisar versiones nuevas
  o dejarlo para la app propia.

Listo cuando: con dos teléfonos conectados se cambia entre ellos desde la ventana, y
la tabla de mediciones incluye el gasto con dos fuentes.

## Presupuesto de batería, CPU y memoria

Reglas de diseño para todas las fases:

1. **Nada trabaja si nadie lo ve:** sin programas usando la cámara virtual, el
   teléfono no graba; con la ventana oculta, no hay vista previa.
2. **Un solo formato de imagen** (YUV 4:2:0) de principio a fin. Solo la vista previa
   se convierte a color RGBA, y en tamaño chico.
3. **Una decodificación por teléfono al aire.** Los que están en espera no transmiten.
4. **Sin sondeos rápidos al teléfono:** usar avisos (llegan cuadros o no, `adb
   track-devices`) en lugar de preguntar cada segundo.
5. **Por WiFi el teléfono gasta más** y no se carga: mostrar la batería y sugerir el
   cable cuando esté baja.

Metas provisionales, a confirmar con las mediciones de la fase 2:

- Teléfono: con la pausa automática, gasto parecido al reposo cuando nadie usa la
  cámara; el modo ahorro gasta menos que 720p normal.
- Computadora, un teléfono en 720p a 30 cuadros: Ixtli y el decodificador juntos por
  debajo de 10 % de un núcleo y de 150 MB de memoria, sin vista previa.
- Cada función nueva se mide antes de darla por terminada.

## Por comprobar antes de programar

- Que la biblioteca Softcam siga funcionando en Windows 10 22H2 y qué formato de
  imagen pide. Alternativa: usar la cámara virtual de OBS.
- Que la cámara del teléfono siga transmitiendo con la pantalla apagada, en el A57 y
  en el moto g34.
- Que el adb para Windows encuentre los teléfonos por mDNS sin instalar nada más.
- Cómo mandar el audio a un dispositivo concreto en Windows (¿scrcpy lo permite o lo
  reproduce Ixtli?).
- Qué versión de v4l2loopback trae el aviso de «hay programas usando la cámara».
- Licencia para incluir `adb.exe` junto a Ixtli (scrcpy para Windows ya lo hace).
- Tamaño de un `ffmpeg.exe` mínimo para incluirlo en Windows.

## Más adelante

- Windows 11: cámara virtual oficial, que sí aparece en todas las apps.
- App propia para Android (Kotlin, para Play Store) que reemplace a scrcpy: sin
  depuración USB, con linterna y zoom óptico. Las cuentas personales nuevas de Play
  Store necesitan una prueba cerrada con testers antes de publicar.
- Flatpak u otros paquetes para más distribuciones de Linux.
- macOS: necesita una extensión de cámara firmada con cuenta de desarrollador de
  Apple.
- Firmar el instalador de Windows.
