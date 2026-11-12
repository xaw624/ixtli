# Plan de Ixtli

Notas para retomar el proyecto en otra sesión. Actualizado el 12 de noviembre de 2026.

## Para retomar (estado al 12 de noviembre de 2026)

Dónde quedó el trabajo:

- **Fases 0 y 1 hechas.** Versión 0.2.0 publicada en GitHub Releases, con `.deb`.
- **Fase 2 programada:** existen `ixtli medir`, el **modo ahorro**, la **batería a
  la vista**, **elegir lente**, la **pantalla apagada** (automática), los **sondeos
  más lentos** y la **vista previa ligera** (los dos últimos del 11 de noviembre,
  probados solo con simuladores). Solo falta medir con el teléfono real.
- El 10 de noviembre se probó con el moto g34 por cable y por WiFi (ver «Primera
  prueba con el teléfono» abajo). De ahí salieron tres arreglos: búsqueda propia en
  la red (mDNS), tamaños del modo ahorro múltiplos de 8 y pantalla apagada siempre.
  Los tres ya están probados: el QR el 10 de noviembre por la red de casa.
- En GitHub está todo hasta `f8db98a` (pausa automática). El ángulo libre con
  el motor propio va en el commit siguiente. Todavía no hay versión 0.3 publicada.
- **Tamaño fijo hecho** (adelanto del paso 3 de la fase 3): ver «Meet perdía la
  cámara» abajo. El 10 de noviembre el usuario lo probó en una llamada de Meet y
  funcionó bien.

Lo siguiente, en orden:

1. **Seguir la fase 3 (motor de video propio).** El 11 de noviembre el usuario
   decidió seguir programando y dejar para después las mediciones y las pruebas con
   el teléfono. Ya funciona con simuladores (`--motor=propio`, ver «Avance de la
   fase 3»), con pausa automática y ángulo libre; falta probarlo con el teléfono.
2. **Más adelante, con el teléfono:** llenar la tabla base de `docs/mediciones.md`.
   Lo hace el usuario; por teléfono son cuatro corridas de `ixtli medir 15
   --pantalla-prendida` (720p y 1080p, por WiFi y por cable), con la cámara «Ixtli»
   abierta en Meet u OBS. La batería solo se ve por WiFi, porque por cable el
   teléfono se carga. Después, las filas del modo ahorro y de la pantalla apagada.
   Como los sondeos y la vista previa ligera ya están hechos, la tabla base ya no
   muestra el gasto «de antes»: para comparar, medir también con la 0.2.0.
3. Las pruebas pendientes con el teléfono real (lista abajo), y publicar la 0.3.

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
  casa van antes que las de la VPN. Probado el 10 de noviembre en la red de casa: el
  teléfono anunció el servicio de vínculo, en 2 segundos quedó conectado
  (`192.168.1.101:41543`) y la cámara encendió por WiFi.
- **Pantalla apagada siempre:** el usuario no le vio utilidad como opción. Se quitó
  el interruptor; la pantalla se apaga al encender la cámara. `--pantalla-prendida`
  la deja prendida solo esa vez (ya no se guarda `pantalla_apagada` en
  `opciones.conf`). Con la pantalla apagada por Ixtli el sensor de giro sigue activo
  (`mEnabled=true` en el moto), así que se quitó la regla de dejar el giro fijo. El
  usuario lo probó en Meet y los giros funcionan con la pantalla apagada.
- **«Server connection failed» al encender:** los dos scrcpy (cámara y pantalla)
  usaban los mismos puertos de la computadora (desde 27183) y, si arrancaban juntos,
  a veces uno se conectaba con el otro. Ahora el de la pantalla usa 27300 a 27399
  (`SCREEN_PORTS` en `session.rs`) y arranca hasta que la cámara ya está en vivo. Al
  reabrir la cámara por un giro, la pantalla se queda apagada. Probado por cable desde
  la terminal, y después en Meet (ver la siguiente nota).
- **Meet perdía la cámara (resuelto el 10 de noviembre).** Tres causas, todas
  comprobadas con el moto g34, un Firefox de prueba (oculto, perfil temporal) y
  lectores de prueba:
  1. **v4l2loopback 0.15 solo deja leer a un programa a la vez** (REQBUFS de captura
     da EBUSY: «only exclusive ownership for each stream»). La vista previa de Ixtli
     leía la cámara «Ixtli» y, al reabrirse la cámara, la tomaba antes que Firefox:
     Meet decía «no puede acceder a tu cámara» y Firefox acumulaba una conexión por
     reintento hasta el límite de 10 (`max_openers`). Ahora la vista previa no lee la
     cámara: toma las imágenes del puente (en el mismo proceso) o de
     `$XDG_RUNTIME_DIR/ixtli/vista.rgba` (la ventana suelta, que lo pide tocando
     `vista-quiero`).
  2. **Mientras alguien la lee, la cámara virtual no deja cambiar su tamaño.** Al
     cambiar modo ahorro, calidad o giro, scrcpy fallaba. Ahora hay **tamaño fijo**:
     el módulo crea dos cámaras (`devices=2 video_nr=10,11
     card_label=Ixtli,Ixtli-interna exclusive_caps=1,1`); scrcpy escribe en la
     interna y el puente (`relay.rs`) copia a «Ixtli» con el tamaño de la calidad
     (1280x720 o 1920x1080), escalando con interpolación bilineal y franjas negras.
     El puente se enciende con la primera sesión y sigue mientras Ixtli esté
     abierto: con la cámara apagada manda negro a 10 cuadros/s, así Meet nunca
     pierde la cámara. Escribe con `write()` después de `VIDIOC_S_FMT`. CPU de Ixtli
     con el puente: 2 % en 720p, 4 % en 1080p, 7 % en modo ahorro (agranda).
     La calidad se bloquea en la ventana mientras otro programa tiene abierta
     «Ixtli» (`sys::cam_users` revisa `/proc/*/fd` cada 2 s). Sin la cámara interna
     (módulo de la 0.2) se bloquea todo lo que cambia el tamaño, no hay vista previa
     y la ventana pide «Actualizar» (recarga el módulo con pkexec).
  3. **«Server connection failed» al encender:** dos scrcpy arrancaban a la vez (la
     cámara y la lista de lentes, o la pantalla) y copiaban el mismo
     `/data/local/tmp/scrcpy-server.jar`; el que arrancaba moría (FATAL en
     app_process). Ahora hay un turno (`sys::scrcpy_turn`, un flock en
     `$XDG_RUNTIME_DIR/ixtli/scrcpy.lock`, también entre dos Ixtli) que dura hasta que
     el servidor escribe su primera línea «[server]». Además el scrcpy de la
     pantalla usa los puertos 27300 a 27399 y arranca con la cámara ya en vivo.
  Pruebas hechas: 8 encendidos seguidos alternando modo ahorro, 6 sin pausa, un
  lector abierto mientras se corta el video del teléfono (sigue recibiendo negro),
  Firefox con la ventana y la vista previa abiertas, y dos Ixtli a la vez (el segundo
  avisa y no estorba). Después el usuario lo probó en una llamada real de Meet con
  el moto g34: funcionan la cámara frontal y la trasera, los giros, los cambios de
  calidad y el modo ahorro.
- **Sin explicar:** scrcpy se reinició tres veces con las mismas opciones (19:19:24,
  19:19:45, 19:23:08). Las dos primeras eran con 960x540, así que seguramente eran
  el rechazo de la cámara. La tercera fue por WiFi justo antes de que se cayera la
  conexión. El teléfono apagó la Depuración inalámbrica en algún momento.

Prueba con un Galaxy A54 (SM-A546E, Android 16, 10 de noviembre de 2026), otro
teléfono además del A57 y del moto g34, por WiFi y con la ventana:

- Se conectó por WiFi y la cámara abrió en 1080p sin errores.
- Anuncia cuatro cámaras: 0 trasera, 1 frontal, 2 trasera y 3 frontal. Se probaron
  la frontal, la trasera y los lentes 2 y 3: todos abren en 1920x1080 a 30 cuadros/s.
  Manda la imagen vertical (1080x1920) y el puente la acomoda en «Ixtli».
- La pantalla se apaga sola con la cámara encendida.
- Detalle: en cada cambio de cámara o lente, el scrcpy de la pantalla se cerraba y
  volvía a arrancar, así que la pantalla se prendía un momento. Arreglado el 11 de
  noviembre: al cambiar una opción con la cámara encendida, el motor pasa el scrcpy
  de la pantalla a la sesión nueva (como ya hacía con el giro), si es el mismo
  teléfono. Probado con `adb` y `scrcpy` simulados (el de la pantalla arranca una
  sola vez); falta verlo con el teléfono real.

Pruebas que siguen pendientes con el teléfono real:

- Modo ahorro en el Galaxy A57 y en el A54: qué tamaño elige y que la cámara lo abra.
- Batería: el aviso en ámbar bajo 20 % por WiFi, y cómo se ve en la ventana.
- `ixtli lentes` en el A57 y en el moto g34. Muchos (sobre todo Samsung) esconden el
  gran angular a las apps; entonces el renglón «Lente» no aparece. Que `--lente=N`
  abra ese lente y que el giro automático lo enderece igual. En el A54 ya se probó
  desde la ventana (ver arriba).
- Pantalla apagada durante 15 minutos o más, por cable y por WiFi, en los dos
  teléfonos. Que se prenda al apagar la cámara, al cerrar Ixtli y al desconectar el
  cable, y que en Ajustes el tiempo de espera de pantalla vuelva a ser el de antes.

Pruebas con el teléfono que siguen pendientes (el WiFi del usuario estaba inestable):

- WiFi en vivo: desconectar el cable y apagar el WiFi del teléfono con la cámara
  encendida; cerrar y reabrir Ixtli sin cable.
- Instalar el `.deb` en la computadora del usuario y reiniciar, para ver que la cámara
  «Ixtli» aparece sola. Hoy hay una cámara virtual «Celular» cargada a mano.
- El botón «Activar» después de reiniciar la computadora.
- La comprobación manual de Windows 10 (paso W0 de la fase 4), que no necesita código.

Cómo se probó sin teléfono: con un `adb` y un `scrcpy` simulados (scripts de shell
puestos antes en el `PATH`, con `XDG_CONFIG_HOME` apuntando a una carpeta temporal).
Desde el 11 de noviembre el `scrcpy` falso manda video de prueba a `--v4l2-sink`
(`ffmpeg -re -f lavfi -i testsrc2=size=WxH:rate=30 -pix_fmt yuv420p -f v4l2`), así
que el puente, la vista previa y la ventana suelta funcionan como con el teléfono.
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
  (`adb pair` + búsqueda mDNS propia, módulos `pairing.rs` y `sys.rs`); los dos
  probados con el moto g34.

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

- Hecho y probado con el moto g34: «Vincular con QR» escaneando desde Depuración
  inalámbrica. El adb de Debian no busca en la red (mDNS), así que Ixtli busca por su
  cuenta (`mdns-sd`). Si la tarjeta se queda en «Esperando…», puede ser que la red
  bloquee los anuncios entre equipos.
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
- **Vista previa ligera.** Hecho a medias el 11 de noviembre, probado solo con
  simuladores:
  - 10 imágenes por segundo en lugar de 15 (`relay::PREVIEW_EVERY`, la usan el
    puente, el hilo de la vista previa y la ventana).
  - Cada imagen se comparte sin copiarla (`Frame::rgba` es `bytes::Bytes`; la ventana
    suelta usa el mismo búfer que lee de `vista.rgba`).
  - **No se pudo:** detenerla sola con la ventana minimizada o tapada. En Wayland,
    winit 0.30 no avisa (`is_minimized` siempre da `None`, no manda `Occluded` y
    sabe del estado `SUSPENDED` pero no lo pasa), e iced 0.13 dibuja sin esperar al
    compositor (no llama `pre_present_notify`), así que tampoco sirve ver si se dejó
    de dibujar. Revisar al cambiar de iced. Mientras, se puede apagar con el
    interruptor «Vista previa».
  Medido con simuladores (`scrcpy` falso que manda video de prueba con ffmpeg),
  compilación release, ventana con vista previa más ventana suelta, 30 a 60 s: la
  ventana pasó de 6 a 7 % de un núcleo a cerca de 4 %; la ventana suelta, de 0.6 %
  a 0.7 % (igual) y recibe 8 imágenes/s en lugar de 14. Memoria: 40 a 43 MB la
  ventana y 34 a 35 MB la suelta, en las dos versiones. No se repitieron los 509 MB
  que se vieron con el teléfono; hay que ver si vuelven en una sesión larga.
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
- **Sondeos más lentos.** Hecho el 11 de noviembre, probado solo con simuladores.
  Antes se preguntaba el giro cada 1.5 s y, por WiFi, se hacía un «ping» cada 3 s;
  cada pregunta abre un proceso en el teléfono (el giro, un `dumpsys window`
  completo). Ahora (`engine.rs`):
  - **Ping solo si dejan de llegar cuadros.** El puente anota cuándo llegó el último
    cuadro del teléfono (`relay::receiving`). Mientras lleguen, no se pregunta nada;
    si pasan 2 s sin cuadros, se pregunta enseguida y luego cada 3 s, y con dos
    intentos sin respuesta se da por perdida la red. Sin cámara interna (módulo de
    la 0.2) se pregunta cada 3 s como antes.
  - **Giro cada 1.5 s solo si alguien ve la cámara:** otro programa tiene abierta
    «Ixtli» (`cam_users`) o la vista previa está a la vista (`relay::preview_watched`).
    Si nadie la ve, cada 10 s; cuando alguien la abre, se pregunta enseguida.
  Con los simuladores, por WiFi, giro automático y 40 s sin nadie viendo: 4
  preguntas de giro y ningún ping; la 0.2 hacía 20 y 13 (33 procesos en el
  teléfono). Con un lector abierto, el giro se pregunta cada 2 s como antes.
  Cortando el video y la red, Ixtli avisa «Se perdió la conexión WiFi» a los 5 s
  (la 0.2, a los 4 s).

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
   con negro lo que sobre. **Hecho antes** con el puente (`relay.rs`), leyendo de la
   cámara interna donde escribe scrcpy; el motor propio solo cambia de dónde salen
   los cuadros.
4. **Giro, espejo y ángulo dentro de Ixtli,** sin reabrir la cámara (el ángulo libre
   terminó girándolo el teléfono; ver «Avance de la fase 3»).
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

### Avance de la fase 3 (11 de noviembre de 2026)

Hechos los pasos 1 a 7, probados solo con simuladores. La pausa automática (paso
5) y el ángulo libre (lo que faltaba del 4) son del 12 de noviembre. **Falta probarlo con un teléfono real.** Por ahora el motor propio
se pide con `--motor=propio` (o `motor=propio` en `opciones.conf`); el de siempre
sigue siendo scrcpy hasta probarlo.

- **`direct.rs`**: copia al teléfono el `scrcpy-server` que trae el paquete de
  scrcpy (`/usr/share/scrcpy/scrcpy-server`, o `SCRCPY_SERVER_PATH`) como
  `/data/local/tmp/ixtli-server.jar`, con la versión del scrcpy instalado (el
  servidor exige la versión exacta). Abre el túnel con `adb forward tcp:0
  localabstract:scrcpy_<scid>` (adb elige el puerto) y arranca el servidor con
  `raw_stream=true send_dummy_byte=true`: el byte inicial confirma que se conectó
  al servidor y no solo a adb. **Cambio al plan:** todavía no se empaqueta una
  versión fija del servidor; se usa la del paquete de scrcpy. Empaquetarla queda
  para Windows (fase 4), donde no hay paquete.
- **Decodificador**: `ffmpeg -f h264 -i pipe:0 ... -f yuv4mpegpipe pipe:1`, con un
  solo hilo (cada hilo de más retrasa un cuadro). yuv4mpeg dice el tamaño en su
  encabezado. Los cuadros pasan al puente (`relay::deliver`) intercambiando búferes,
  sin copiarlos.
- **Giro y espejo en el puente** (`relay::orient`): el servidor manda la imagen como
  la da el sensor y el puente la gira. El giro automático y los cambios de giro o
  espejo en la ventana ya no reabren la cámara (`keeps_open` en `engine.rs`).
  Cambiar la vista previa tampoco reabre, con ningún motor. Ver abajo el ángulo libre.
- **Micrófono**: un scrcpy sin video (`Session::audio_only`).
- La red caída se nota porque deja de llegar video: la conexión se cierra y Ixtli
  avisa enseguida (0.5 a 1.5 s con los simuladores).
- **Pausa automática** (12 de noviembre): con el motor propio, si nadie ve la
  cámara durante 10 segundos (ningún programa la tiene abierta y no se ve la vista
  previa), Ixtli cierra la conexión con el servidor: el teléfono apaga la cámara y
  «Ixtli» muestra una imagen fija de pausa (gris oscuro con dos barras, sin letras).
  Al abrirla alguien, vuelve a arrancar el servidor sin copiarlo otra vez y lee el
  giro antes de que lleguen los cuadros. El micrófono y la pantalla apagada siguen
  durante la pausa. La ventana dice «En pausa» y la terminal avisa al entrar y al
  salir. Se apaga con `--sin-pausa` o `pausa=no` en `opciones.conf`. Con la ventana
  abierta y la vista previa visible no se pausa, porque cuenta como alguien viendo.
- **Quién usa la cámara, sin revisar a cada rato:** revisar todos los procesos
  cuesta decenas de milisegundos (unos 6 600 archivos abiertos en esta computadora).
  Ahora el sistema avisa (inotify, `sys::OpenWatch`) cuando alguien abre o cierra
  «Ixtli» y solo entonces se revisa, más una revisión de respaldo cada 10 s (antes,
  cada 2 s). De paso, un navegador que solo abre y cierra la cámara para listarla no
  despierta al teléfono.
- **Ángulo libre** (12 de noviembre). **Cambio al plan:** no lo gira Ixtli sino el
  teléfono, con el parámetro `angle=` del servidor (el mismo que usa `scrcpy
  --angle`, en la GPU del teléfono). Girar a cualquier ángulo cada cuadro en la
  computadora costaría de 8 a 20 % de un núcleo en 720p, más que todo lo demás
  junto, y el ángulo se ajusta una vez y casi no se cambia. El servidor lo aplica
  antes que el giro y el espejo del puente; los giros se pueden hacer en cualquier
  orden, pero el espejo no, así que con espejo se le pide al revés
  (`direct::server_angle`) y la imagen queda igual que con scrcpy, que lo aplica al
  final. Cambiar el ángulo (o el espejo con ángulo) **reinicia solo el video**
  (`Change::Restream` en `engine.rs`): el micrófono sigue y «Ixtli» repite el
  último cuadro mientras vuelve (`Filler::Hold` en el puente), en lugar de negro.
  El giro y el espejo sin ángulo siguen sin cortar nada.
- **Pendiente:** empaquetar el servidor (en la fase 4).

Cómo se probó: un `adb` simulado que, al arrancar el «servidor», abre el puerto del
túnel, manda el byte inicial y luego H.264 de prueba (`ffmpeg -f lavfi -i testsrc2
... -c:v libx264 -tune zerolatency -f h264 -`). Se comprobó la imagen con el
teléfono vertical y acostado (sacando un cuadro de «Ixtli» con ffmpeg), que el
servidor se arranca una sola vez al girar, el micrófono, el aviso al cortarse el
video y que se quita el túnel.

Gasto medido con los simuladores (compilación release, cámara trasera, 30 s): el
decodificador (ffmpeg) usa 5 % de un núcleo en 720p y 10 a 11 % en 1080p. Ixtli,
3.5 % en 720p sin giro y unos 7 % con el teléfono vertical (90°); en 1080p, 6.7 %
sin giro y 13.6 % vertical. Al principio el giro costaba más (8.7 % en 720p y 19 %
en 1080p): ahora el puente primero achica la imagen y después gira solo la chica
(`fit_turned`). Con el motor de scrcpy, Ixtli usa 2.7 % en 720p, pero ahí no se
cuenta lo que gasta scrcpy al decodificar. **Ojo:** en esta computadora las cifras
cambian hasta 50 % de una corrida a otra (8 GB de memoria con 6 GB en swap): en
otra corrida el decodificador solo marcó 8 % en 720p. Comparar siempre en la misma
tanda. El pipe de ffmpeg se agranda a 1 MB (`F_SETPIPE_SZ`) para leer cada cuadro
en menos pasos; con ese ruido no se pudo medir si ayuda.

Pausa automática con los simuladores (12 de noviembre): se pausa unos 10 s después
de quedar en vivo y quita el túnel; al abrir la cámara con ffmpeg el servidor
vuelve a arrancar 0.7 s después y la imagen real llega a los 3 a 3.5 s (el
simulador tarda en arrancar su codificador; con el teléfono falta medirlo). En
pausa, Ixtli gasta 0.8 a 0.9 % de un núcleo; revisando los procesos cada 700 ms
gastaba 4.8 %, y por eso se cambió a inotify. Con micrófono, este sigue durante la
pausa y al apagar no quedan módulos de audio. Con `--sin-pausa` y con el motor
scrcpy no se pausa. La pantalla «En pausa» de la ventana no se ha visto todavía.

Ángulo libre con los simuladores (12 de noviembre; el servidor simulado gira su
imagen con el filtro `rotate` de ffmpeg cuando recibe `angle=`): con 15° se
reinició solo el video (`angle=15`), con espejo otra vez (`angle=345`) y con giro
de 180° no se reinició nada. En los tres casos la imagen final quedó girada en el
sentido del reloj, como con scrcpy. El micrófono arrancó una sola vez y el
servidor se copió una sola vez. Mientras el video volvía (unos 2.5 s, casi todo el
arranque del simulador), «Ixtli» mostró el último cuadro.

Qué hay que ver con el teléfono real: que el servidor acepte los parámetros
(`raw_stream`, `send_dummy_byte` después, `cleanup=false`), que la imagen sin
`capture_orientation` salga como la da el sensor (así lo supone el giro), el
retraso del decodificador y el gasto contra el motor de scrcpy. Que el servidor acepte
`angle=` con la cámara. De la pausa: cuánto tarda en volver la imagen, que el teléfono de verdad apague la cámara (se apaga el
indicador verde de Android) y que Meet u OBS no pierdan la cámara durante la pausa.

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
  (en el moto g34 y en el Galaxy A54 sí).
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
