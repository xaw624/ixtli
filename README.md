# Ixtli

Usa tu celular Android como **cámara web y micrófono** en Linux, con un clic.

*Ixtli* significa «rostro» u «ojo» en náhuatl.

Ixtli no reinventa nada: automatiza [scrcpy](https://github.com/Genymobile/scrcpy),
[v4l2loopback](https://github.com/umlaeute/v4l2loopback) y PipeWire/PulseAudio, que
normalmente hay que configurar a mano con varios comandos en la terminal. El resultado
es una cámara y un micrófono normales que puedes elegir en Meet, Zoom, Teams, OBS o
cualquier otro programa.

- No hay que instalar ninguna app en el teléfono.
- Cámara frontal o trasera, en 720p o 1080p.
- Giro de la imagen: fijo (0°, 90°, 180°, 270°) o automático según la posición del
  teléfono.
- Vista previa dentro de la ventana, que se puede desacoplar a una ventana aparte.
- Micrófono del teléfono como micrófono virtual.
- Conexión por cable USB.
- Ventana sencilla o uso desde la terminal.

![Ventana de Ixtli](docs/captura.png)

## Requisitos

- Linux con PipeWire o PulseAudio.
- Un teléfono con **Android 12 o más nuevo** y un cable USB.
- `scrcpy` 3.0 o más nuevo, `adb`, el módulo `v4l2loopback` y `pactl`.

En Debian 13 o más nuevo:

```bash
sudo apt install scrcpy adb v4l2loopback-dkms pulseaudio-utils
```

En otras distribuciones los paquetes se llaman parecido (`scrcpy`, `android-tools`,
`v4l2loopback-dkms`). Si falta alguno, Ixtli te dice cuál.

## Preparar el teléfono (solo la primera vez)

1. En **Ajustes → Acerca del teléfono → Información de software**, toca 7 veces
   **Número de compilación**.
2. En **Ajustes → Opciones de desarrollador**, activa **Depuración USB**.
3. Conecta el teléfono por USB y acepta el aviso **«¿Permitir depuración USB?»**.

## Instalación

Necesitas Rust 1.85 o más nuevo (`sudo apt install cargo`).

```bash
git clone https://github.com/xaw624/ixtli.git
cd ixtli
./instalar.sh
```

El script compila Ixtli y lo instala solo para tu usuario, sin sudo: el programa queda
en `~/.local/bin/ixtli` y aparece como **Ixtli** en el menú de aplicaciones.

## Uso

Abre **Ixtli**, conecta el teléfono y presiona **Encender**. Después, en tu
videollamada elige:

- Cámara: **Ixtli**
- Micrófono: **Micrófono del celular (Ixtli)**

La primera vez después de prender la computadora, Ixtli pide tu contraseña para activar
la cámara virtual.

### Girar la imagen

En **Giro** eliges cuánto girar la imagen. Con **Auto**, Ixtli sigue la posición del
teléfono: para eso activa el giro automático de pantalla en el teléfono y deja la
pantalla encendida. Cada vez que cambia el giro, la cámara se reabre y la imagen se
corta un par de segundos.

### Vista previa

Con la cámara encendida, la imagen aparece dentro de la ventana. **Desacoplar** la pasa
a una ventana aparte que puedes mover y cambiar de tamaño; **Acoplar** (o cerrar esa
ventana) la regresa.

### Desde la terminal

```bash
ixtli --encender                   # abre la ventana y enciende de una vez
ixtli estado                       # revisa si todo está listo
ixtli encender                     # enciende; Ctrl+C para apagar
ixtli encender --trasera --1080    # cámara trasera en 1080p
ixtli encender --giro=90           # imagen girada 90°
ixtli encender --sin-micro         # solo la cámara
ixtli vista                        # ventana con la imagen de la cámara
ixtli --help
```

## Cómo funciona

1. `scrcpy` captura la cámara del teléfono por USB y la envía a una cámara virtual
   creada con `v4l2loopback`.
2. El audio del micrófono del teléfono se envía a una salida de audio interna que no
   suena en las bocinas.
3. Un micrófono virtual toma lo que llega a esa salida, y así los demás programas lo
   ven como un micrófono normal.

Al apagar o cerrar Ixtli se cierra `scrcpy` y se quita el micrófono virtual.

## Limitaciones

- Solo Linux y solo por cable USB.
- La cámara virtual se pierde al reiniciar la computadora; Ixtli la vuelve a activar
  cuando hace falta.
- En algunos navegadores hay que recargar la página de la videollamada para que
  aparezcan la cámara y el micrófono nuevos.

## Créditos

Ixtli es una interfaz sobre el trabajo de otros proyectos:
[scrcpy](https://github.com/Genymobile/scrcpy) (Genymobile),
[v4l2loopback](https://github.com/umlaeute/v4l2loopback) y
[iced](https://github.com/iced-rs/iced) para la ventana.

## Licencia

[MIT](LICENSE)
