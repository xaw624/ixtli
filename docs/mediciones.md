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
