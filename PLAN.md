# Plan de Ixtli

Notas para retomar el proyecto en otra sesión. Actualizado el 10 de noviembre de 2026.

## Para retomar (estado al 10 de noviembre de 2026)

Dónde quedó el trabajo:

- **Fases 0 y 1 hechas.** Versión 0.2.0 publicada en GitHub Releases, con `.deb`.
- **Fase 2 a medias:** existen `ixtli medir`, el **modo ahorro**, la **batería a la
  vista**, **elegir lente** y la **pantalla apagada**, que ahora es automática.
- El 10 de noviembre se probó con el moto g34 por cable y por WiFi (ver «Primera
  prueba con el teléfono» abajo). De ahí salieron tres arreglos, todavía **sin
  probar en la ventana**: búsqueda propia en la red (mDNS), tamaños del modo ahorro
  múltiplos de 8 y pantalla apagada siempre.
- Todo hasta `95368a5` está subido a GitHub; los commits de los arreglos son locales.
  Todavía no hay versión 0.3 publicada.

Lo siguiente, en orden:

1. **Probar los arreglos con el teléfono:** «Vincular con QR» en la red local (hay que
   activar la Depuración inalámbrica: el 10 de noviembre estaba apagada), el modo
   ahorro desde la ventana y girar el teléfono con la pantalla apagada.
2. **Llenar la tabla base** de `docs/mediciones.md` con el teléfono real. Lo hace el
   usuario; por teléfono son cuatro corridas de `ixtli medir 15 --pantalla-prendida`
   (720p y 1080p, por WiFi y por cable), con la cámara «Ixtli» abierta en Meet u OBS.
   La batería solo se ve por WiFi, porque por cable el teléfono se carga. Después,
   las filas del modo ahorro y de la pantalla apagada.
3. **Terminar la fase 2:** ya no queda nada opcional por programar. Los sondeos más
   lentos y la vista previa ligera sí cambian el gasto normal: van después de llenar
   la tabla base, para ver la diferencia.
4. Después, la fase 3 (motor de video propio).

Primera prueba con el teléfono (moto g34, 10 de noviembre de 2026):

- **Funcionó:** cambiar cámara, calidad, modo ahorro y micrófono en vivo (cada cambio
  reabre scrcpy una vez); el giro automático; pasar del cable al WiFi; apagar la
  pantalla (desbloquear la prende, y eso está bien). Ixtli usó como máximo 66 MB y
  cerca de 22 % de un núcleo. Con la vista previa desacoplada llegó a 509 MB: revisar
  en la «vista previa ligera».
- **Modo ahorro:** el moto anuncia 960x540 y 1600x900, pero su cámara los rechaza al
  abrirse (`configureStreamsLocked: Set of requested inputs/outputs not supported by
  HAL`), en las dos cámaras y con cualquier número de cuadros. Los tamaños que sí
  abren tienen ancho y alto múltiplos de 8, así que `saver_size` ahora solo elige
  esos: en el moto queda 720x480 a 15 cuadros/s. Probado por cable desde la terminal:
  llegan 15 cuadros/s a la cámara virtual con la pantalla apagada.
- **Mensaje equivocado:** con ese rechazo scrcpy escribe también «Device
  disconnected», y la ventana decía «revisa el cable». Ahora `explain` busca el
  rechazo de la cámara en todo el registro (`camera_rejected` en `session.rs`) antes
  que la conexión perdida.
- **QR:** no funcionó y hubo que escribir la dirección a mano. Causa: el adb de
  Debian 13 (34.0.5) viene **sin búsqueda mDNS** (`adb mdns check` no dice nada y
  `adb mdns services` siempre sale vacío). Ahora Ixtli busca por su cuenta con la
  biblioteca `mdns-sd` 0.21 (compila con Rust 1.85): `sys::browser()` escucha
  `_adb-tls-connect` y `_adb-tls-pairing` desde la primera vez que hace falta, y
  `mdns_services()` suma lo que vea un adb con mDNS. Las direcciones de la red de
  casa van antes que las de la VPN. Falta probarlo con la Depuración inalámbrica
  encendida.
- **Pantalla apagada siempre:** el usuario no le vio utilidad como opción. Se quitó
  el interruptor; la pantalla se apaga al encender la cámara. `--pantalla-prendida`
  la deja prendida solo esa vez (ya no se guarda `pantalla_apagada` en
  `opciones.conf`). Con la pantalla apagada por Ixtli el sensor de giro sigue activo
  (`mEnabled=true` en el moto), así que se quitó la regla de dejar el giro fijo. Falta
  girar el teléfono con la pantalla apagada y ver que la imagen lo siga.
- **«Server connection failed» al encender:** los dos scrcpy (cámara y pantalla)
  usaban los mismos puertos de la computadora (desde 27183) y, si arrancaban juntos,
  a veces uno se conectaba con el otro. Ahora el de la pantalla usa 27300 a 27399
  (`SCREEN_PORTS` en `session.rs`) y arranca hasta que la cámara ya está en vivo. Al
  reabrir la cámara por un giro, la pantalla se queda apagada. Probado por cable desde
  la terminal; falta ver en Meet que la cámara aparezca (el usuario dijo que Meet no
  la reconocía; quizá fue esta falla, o Meet se abrió con la cámara apagada).
- **Sin explicar:** scrcpy se reinició tres veces con las mismas opciones (19:19:24,
  19:19:45, 19:23:08). Las dos primeras eran con 960x540, así que seguramente eran
  el rechazo de la cámara. La tercera fue por WiFi justo antes de que se cayera la
  conexión. El teléfono apagó la Depuración inalámbrica en algún momento.

Pruebas que siguen pendientes con el teléfono real:

- Modo ahorro en el Galaxy A57: qué tamaño elige y que la cámara lo abra.
- Batería: el aviso en ámbar bajo 20 % por WiFi, y cómo se ve en la ventana.
- `ixtli lentes` en los dos teléfonos. Muchos (sobre todo Samsung) esconden el gran
  angular a las apps; entonces el renglón «Lente» no aparece. Que `--lente=N` abra
  ese lente y que el giro automático lo enderece igual.
- Pantalla apagada durante 15 minutos o más, por cable y por WiFi, en los dos
  teléfonos. Que se prenda al apagar la cámara, al cerrar Ixtli y al desconectar el
  cable, y que en Ajustes el tiempo de espera de pantalla vuelva a ser el de antes.

Pruebas con el teléfono que siguen pendientes (el WiFi del usuario estaba inestable):

- WiFi en vivo: desconectar el cable y apagar el WiFi del teléfono con la cámara
  encendida; cerrar y reabrir Ixtli sin cable.
- «Vincular con QR» en la red local, ya con la búsqueda propia (mDNS). El 10 de
  noviembre falló porque el adb de Debian no busca en la red. El código de 6 dígitos
  sí funcionó, por VPN (Tailscale).
- Instalar el `.deb` en la computadora del usuario y reiniciar, para ver que la cámara
  «Ixtli» aparece sola. Hoy hay una cámara virtual «Celular» cargada a mano.
- El botón «Activar» después de reiniciar la computadora.
- La comprobación manual de Windows 10 (paso W0 de la fase 4), que no necesita código.

Cómo se probó sin teléfono: con un `adb` y un `scrcpy` simulados (scripts de shell
puestos antes en el `PATH`, con `XDG_CONFIG_HOME` apuntando a una carpeta temporal).
No están en el repositorio; hay que volver a escribirlos si hacen falta.

Cosas del entorno:

- La ventana instalada es `~/.local/bin/ixtli`. Después de cambiar código hay que
  correr `./instalar.sh` y reabrir Ixtli, o se sigue viendo la versión vieja.
- Para subir cambios a `.github/workflows/`, `gh` necesita el permiso `workflow`
  (ya dado en esta computadora).
- Publicar una versión: subir la etiqueta `vX.Y.Z`; el flujo arma y publica el `.deb`.

## Dónde está hoy

Funciona en Debian 13 con KDE/Wayland, probado con un Galaxy A57 (Android 16) y un
moto g34 5G:

- Cámara frontal o trasera por USB, en 720p o 1080p, hacia una cámara virtual.
- Micrófono del teléfono como micrófono virtual.
- Giro: automático, cuartos de vuelta, espejo y ángulo personalizado.
- Vista previa incrustada en la ventana y desacoplable (`ixtli vista`).
- Modo terminal: `ixtli estado`, `ixtli encender`, `ixtli --encender`.
- Modo ahorro (cerca de 480p, 15 cuadros/s, 2 Mb/s), probado con el moto g34.
- Batería del teléfono en la ventana, con aviso bajo 20 %; probado solo con simuladores.
- Elegir otro lente del mismo lado (`ixtli lentes`, `--lente=N`); probado solo con
  simuladores.
- La pantalla del teléfono se apaga sola mientras la cámara está encendida; probado
  con el moto g34 (`--pantalla-prendida` para dejarla prendida).
- **WiFi:** «Conectar por WiFi» con el cable una vez
  (`adb tcpip`), probado con el moto g34; y «Vincular con QR» sin cable
  (`adb pair` + búsqueda mDNS propia, módulos `pairing.rs` y `sys.rs`); el QR falta
  probarlo en la red local.

Cómo está hecho:

- Rust con iced 0.13 (renderizador tiny-skia). Rust viene de apt en Debian 13
  (cargo 1.85), así que las dependencias deben seguir compilando con esa versión.
- Por debajo usa scrcpy 3.0 o más nuevo, v4l2loopback y `pactl`.
- Módulos: `engine.rs` (hilo que maneja la sesión), `session.rs` (scrcpy),
  `sys.rs` (teléfono, adb y cámara virtual), `pairing.rs` (vincular con QR),
  `audio.rs` (micrófono virtual), `preview.rs` (lee la cámara virtual),
  `measure.rs` (mediciones), `gui.rs` y `cli.rs`.
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
  El adb de Debian no busca en la red (mDNS), así que Ixtli busca por su cuenta
  (`mdns-sd`). Si la tarjeta se queda en «Esperando…», puede ser que la red bloquee
  los anuncios entre equipos.
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

- **Medir primero.** Hecho: `ixtli medir [MINUTOS] [opciones]` (módulo `measure.rs`)
  enciende la cámara y anota cada minuto la batería y la temperatura del teléfono
  (`adb shell dumpsys battery`) y la CPU y memoria de Ixtli, scrcpy y adb. Probado
  solo con simuladores. **Falta llenar la tabla base** de `docs/mediciones.md` con los
  teléfonos reales (720p y 1080p, cable y WiFi) antes de tocar nada del ahorro.
- **Modo ahorro.** Hecho, probado con el moto g34: interruptor «Modo ahorro» en
  la ventana, `--ahorro` / `--sin-ahorro` en la terminal y `ahorro=si` en
  `opciones.conf`. Ixtli lee una vez por teléfono `scrcpy --list-camera-sizes`
  (`sys::phone_cameras`, guarda la respuesta mientras Ixtli siga abierto) y elige
  (`Options::capture` en `config.rs`):
  - tamaño: el más cercano a 480 de alto, de preferencia 16:9, siempre menos de 720p,
    con ancho y alto múltiplos de 8 (el moto rechaza 960x540);
  - cuadros: 15 si la cámara los acepta, si no 24, si no 30;
  - tasa de bits: 2 Mb/s (scrcpy usa 8 Mb/s).
  Si no puede leer la lista, usa 720p con 2 Mb/s. Con ahorro se ignora la calidad
  elegida. `ixtli medir --ahorro` anota en la fila el tamaño que se usó.
- **Pantalla apagada.** Hecho, probado con el moto g34. Con la cámara como
  fuente scrcpy no controla el teléfono (`Cannot request to turn screen off if
  control is disabled`), así que Ixtli abre un segundo scrcpy sin video ni audio
  (`ScreenOff` en `session.rs`: `--no-video --no-audio --no-window --turn-screen-off
  --screen-off-timeout=86400`). Se abre después de la cámara y se cierra antes; al
  cerrarlo, la pantalla se prende. Si no se puede apagar, la cámara sigue; la
  tarjeta «En vivo» solo dice que se apagó cuando de verdad se apagó. Desde el 10 de noviembre es automática: no hay
  interruptor ni se guarda; `--pantalla-prendida` la deja prendida esa vez. El giro
  automático sigue funcionando (el sensor sigue activo con el panel apagado). Cada
  cambio de opción reabre la cámara, y con eso la pantalla se prende un momento.
- **Vista previa ligera:** menos cuadros por segundo y se detiene sola cuando la
  ventana está minimizada o tapada.
- **Batería a la vista.** Hecho, probado solo con simuladores. El motor lee
  `dumpsys battery` al aparecer el teléfono y luego cada minuto (`BATTERY_EVERY` en
  `engine.rs`), con la cámara encendida o apagada. La ventana muestra el nivel en el
  renglón «Teléfono». Debajo de 20 % y sin carga, el renglón se pone ámbar y aparece
  un aviso bajo el estado: por WiFi sugiere el cable; por USB, un cargador (hay
  cables y puertos que no cargan). `ixtli encender` avisa una vez por sesión.
- **Elegir lente.** Hecho, probado solo con simuladores. Usa la misma lista que el
  modo ahorro (`--list-camera-sizes` trae el número de cada cámara y el tamaño del
  sensor). Al aparecer el teléfono, el motor la pide en un hilo aparte (tarda un par
  de segundos y abre un momento el servidor de scrcpy en el teléfono, una vez por
  teléfono mientras Ixtli siga abierto). Si hay más de una cámara del lado elegido,
  la ventana muestra el renglón «Lente» con «Principal» y los números de las otras,
  y los megapíxeles del sensor; Android no dice cuál es el gran angular. Se guarda
  como `lente=N` y se pide con `--camera-id=N` en lugar de `--camera-facing`. Si el
  teléfono no tiene ese lente de ese lado, se usa el principal. Al cambiar de
  frontal a trasera se vuelve al principal. En la terminal: `ixtli lentes` y
  `--lente=N` / `--lente=principal`.
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
- Que la cámara del teléfono siga transmitiendo con la pantalla apagada en el A57
  (en el moto g34 sí).
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
