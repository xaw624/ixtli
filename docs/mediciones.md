# Mediciones de Ixtli

Cuánta batería gasta el teléfono y cuánta CPU y memoria usa Ixtli en la computadora.
Sirven para comparar antes y después de cada cambio de ahorro.

## Cómo medir

```bash
ixtli medir 15
```

Enciende la cámara 15 minutos y anota una muestra por minuto. Acepta las mismas
opciones que `ixtli encender` (por ejemplo `ixtli medir 15 --trasera --1080`). Al
terminar imprime una fila lista para pegar en la tabla de abajo.

Para medir el modo ahorro, agrega `--ahorro`. El tamaño depende del teléfono: la fila
dice cuál se usó, por ejemplo `frontal ahorro (854x480, 15 cuadros/s, 2 Mb/s)`.

Para que las mediciones se puedan comparar:

- Mide **por WiFi** si quieres ver el gasto de batería: por cable el teléfono se carga
  y la batería no baja. Por cable sí sirven la temperatura, la CPU y la memoria.
- Mide 15 minutos o más: la batería se reporta en puntos enteros.
- Deja el teléfono quieto, con el mismo brillo de pantalla, y cierra otras apps.
- Abre la cámara «Ixtli» en algún programa (Meet, OBS) durante la medición, como en un
  uso real.

Qué significa cada columna:

- **Batería/h:** puntos de batería por hora (negativo si bajó).
- **Temp.:** cambio de temperatura de la batería, en grados.
- **CPU:** promedio de Ixtli, scrcpy y adb juntos, en porcentaje de un núcleo.
- **Memoria:** máximo de los tres juntos, en MB.

## Tabla base (versión 0.2)

| Fecha | Teléfono | Conexión | Opciones | Min | Batería/h | Temp. | CPU % | Memoria MB |
|-------|----------|----------|----------|-----|-----------|-------|-------|------------|

Faltan por medir, en el Galaxy A57 y en el moto g34: 720p y 1080p, por cable y por WiFi.
Desde la versión 0.3 la pantalla del teléfono se apaga sola; para que la tabla base sea
como la 0.2, mide con `--pantalla-prendida` (por ejemplo `ixtli medir 15 --pantalla-prendida`).

## Modo ahorro (versión 0.3, en desarrollo)

| Fecha | Teléfono | Conexión | Opciones | Min | Batería/h | Temp. | CPU % | Memoria MB |
|-------|----------|----------|----------|-----|-----------|-------|-------|------------|

Faltan por medir, en los dos teléfonos, por WiFi (para la batería) y por cable:

- `ixtli medir 15`: solo la pantalla apagada (la fila no dice «pantalla prendida»).
- `ixtli medir 15 --ahorro --pantalla-prendida`: solo el modo ahorro.
- `ixtli medir 15 --ahorro`: las dos cosas juntas.

Hay que compararlas con la fila de 720p por WiFi de la tabla base.

En el moto g34 el modo ahorro usa **720x480**: la cámara anuncia 960x540, pero no lo
acepta al abrirse (tampoco 1600x900), así que Ixtli solo elige tamaños con ancho y
alto múltiplos de 8.
