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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub facing: Facing,
    pub quality: Quality,
    /// Usar también el micrófono del teléfono.
    pub mic: bool,
    /// Abrir una ventana con la imagen de la cámara.
    pub preview: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            facing: Facing::Front,
            quality: Quality::Hd,
            mic: true,
            preview: false,
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
        "camara={}\ncalidad={}\nmicrofono={}\nvista_previa={}\n",
        match o.facing {
            Facing::Front => "frontal",
            Facing::Back => "trasera",
        },
        match o.quality {
            Quality::Hd => "720",
            Quality::FullHd => "1080",
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
            mic: false,
            preview: true,
        };
        assert_eq!(parse(&serialize(&o)), o);
        assert_eq!(parse(&serialize(&Options::default())), Options::default());
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(parse("hola\ncamara=lateral\n=\n"), Options::default());
    }
}
