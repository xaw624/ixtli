//! Opciones del usuario y su archivo de configuración.

use std::fs;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    Front,
    Back,
}

impl Facing {
    /// Valor que espera `scrcpy --camera-facing`.
    pub fn scrcpy(self) -> &'static str {
        match self {
            Facing::Front => "front",
            Facing::Back => "back",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Hd,
    FullHd,
}

impl Quality {
    /// Valor que espera `scrcpy --camera-size`.
    pub fn size(self) -> &'static str {
        match self {
            Quality::Hd => "1280x720",
            Quality::FullHd => "1920x1080",
        }
    }
}

/// Cómo girar la imagen de la cámara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Sigue la orientación del teléfono (necesita el giro automático activado).
    Auto,
    /// Giro fijo en grados, en el sentido del reloj: 0, 90, 180 o 270.
    Fixed(u16),
}

impl Rotation {
    /// Grados que hay que pedirle a scrcpy.
    ///
    /// `device_quarters` es cuánto está girado el teléfono (0 a 3, en cuartos
    /// de vuelta) o `None` si no se sabe; entonces se asume que está vertical.
    /// El sensor de las cámaras está montado de lado: 90° el trasero y 270°
    /// el frontal, como en casi todos los teléfonos.
    pub fn degrees(self, facing: Facing, device_quarters: Option<u8>) -> u16 {
        match self {
            Rotation::Fixed(degrees) => degrees % 360,
            Rotation::Auto => {
                let device = u16::from(device_quarters.unwrap_or(0) % 4) * 90;
                match facing {
                    Facing::Back => (90 + 360 - device) % 360,
                    Facing::Front => (270 + device) % 360,
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub facing: Facing,
    pub quality: Quality,
    pub rotation: Rotation,
    /// Usar también el micrófono del teléfono.
    pub mic: bool,
    /// Mostrar la imagen de la cámara dentro de la ventana.
    pub preview: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            facing: Facing::Front,
            quality: Quality::Hd,
            rotation: Rotation::Fixed(0),
            mic: true,
            preview: true,
        }
    }
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ixtli").join("opciones.conf"))
}

pub fn load() -> Options {
    path()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Guarda las opciones. Si falla no pasa nada: solo no se recuerdan.
pub fn save(options: &Options) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, serialize(options));
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "si"
    } else {
        "no"
    }
}

fn serialize(o: &Options) -> String {
    format!(
        "camara={}\ncalidad={}\ngiro={}\nmicrofono={}\nvista_previa={}\n",
        match o.facing {
            Facing::Front => "frontal",
            Facing::Back => "trasera",
        },
        match o.quality {
            Quality::Hd => "720",
            Quality::FullHd => "1080",
        },
        match o.rotation {
            Rotation::Auto => "auto".to_string(),
            Rotation::Fixed(degrees) => degrees.to_string(),
        },
        yes_no(o.mic),
        yes_no(o.preview),
    )
}

/// Lee `clave=valor` por línea. Lo que no entiende lo deja en su valor por defecto.
fn parse(text: &str) -> Options {
    let mut o = Options::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match (key.trim(), value.trim()) {
            ("camara", "frontal") => o.facing = Facing::Front,
            ("camara", "trasera") => o.facing = Facing::Back,
            ("calidad", "720") => o.quality = Quality::Hd,
            ("calidad", "1080") => o.quality = Quality::FullHd,
            ("giro", "auto") => o.rotation = Rotation::Auto,
            ("giro", v @ ("0" | "90" | "180" | "270")) => {
                o.rotation = Rotation::Fixed(v.parse().unwrap_or(0))
            }
            ("microfono", v) => o.mic = v == "si",
            ("vista_previa", v) => o.preview = v == "si",
            _ => {}
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let o = Options {
            facing: Facing::Back,
            quality: Quality::FullHd,
            rotation: Rotation::Fixed(270),
            mic: false,
            preview: false,
        };
        assert_eq!(parse(&serialize(&o)), o);
        let auto = Options {
            rotation: Rotation::Auto,
            ..o
        };
        assert_eq!(parse(&serialize(&auto)), auto);
        assert_eq!(parse(&serialize(&Options::default())), Options::default());
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(parse("hola\ncamara=lateral\ngiro=45\n=\n"), Options::default());
    }

    #[test]
    fn auto_rotation_follows_the_phone() {
        let auto = Rotation::Auto;
        // Teléfono vertical (o sin dato): se endereza según el sensor.
        assert_eq!(auto.degrees(Facing::Front, None), 270);
        assert_eq!(auto.degrees(Facing::Back, Some(0)), 90);
        // Teléfono acostado: el sensor ya queda derecho.
        assert_eq!(auto.degrees(Facing::Front, Some(1)), 0);
        assert_eq!(auto.degrees(Facing::Back, Some(1)), 0);
        assert_eq!(auto.degrees(Facing::Front, Some(3)), 180);
        assert_eq!(auto.degrees(Facing::Back, Some(3)), 180);
        assert_eq!(Rotation::Fixed(90).degrees(Facing::Front, Some(2)), 90);
    }
}
