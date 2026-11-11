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

/// Lo que se le pide a la cámara del teléfono en una sesión.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// Lente elegido (`--camera-id`); `None` usa la primera cámara del lado
    /// pedido (`--camera-facing`).
    pub camera_id: Option<String>,
    /// Para `scrcpy --camera-size`, por ejemplo `1280x720`.
    pub size: String,
    /// Cuadros por segundo (`--camera-fps`).
    pub fps: u16,
    /// Tasa de bits (`--video-bit-rate`); `None` deja la de scrcpy (8 Mb/s).
    pub bit_rate: Option<&'static str>,
}

impl Capture {
    /// Para mostrarla: «854x480, 15 cuadros/s, 2 Mb/s».
    pub fn describe(&self) -> String {
        let mut text = format!("{}, {} cuadros/s", self.size, self.fps);
        if let Some(rate) = self.bit_rate {
            text.push_str(&format!(", {} Mb/s", rate.trim_end_matches('M')));
        }
        text
    }
}

/// Tasa de bits del modo ahorro. scrcpy usa 8 Mb/s, que sobra para 480p.
const SAVER_BIT_RATE: &str = "2M";

impl Options {
    /// Qué pedirle a la cámara. Sin modo ahorro es la calidad elegida a 30
    /// cuadros/s. Con modo ahorro se buscan en `cameras` (la lista del
    /// teléfono) un tamaño cercano a 480p y 15 o 24 cuadros/s; si no hay
    /// lista, se queda en 720p, que todos los teléfonos aceptan, y solo
    /// baja la tasa de bits.
    ///
    /// Si se eligió otro lente y el teléfono lo tiene de ese lado, se usa;
    /// si no (por ejemplo, con otro teléfono), se usa el principal.
    pub fn capture(&self, cameras: Option<&[crate::sys::PhoneCamera]>) -> Capture {
        let lenses = lenses(cameras.unwrap_or_default(), self.facing);
        let chosen = self.lens.and_then(|lens| lenses.iter().copied().find(|c| c.id == lens.to_string()));
        let camera_id = chosen.map(|camera| camera.id.clone());
        if !self.saver {
            return Capture { camera_id, size: self.quality.size().into(), fps: 30, bit_rate: None };
        }
        // Sin lente elegido, scrcpy usa la primera cámara del lado pedido.
        let camera = chosen.or(lenses.first().copied());
        let size = camera
            .and_then(|camera| saver_size(&camera.sizes))
            .map(|(w, h)| format!("{w}x{h}"))
            .unwrap_or_else(|| Quality::Hd.size().into());
        let fps = camera
            .and_then(|camera| [15, 24].into_iter().find(|fps| camera.fps.contains(fps)))
            .unwrap_or(30);
        Capture { camera_id, size, fps, bit_rate: Some(SAVER_BIT_RATE) }
    }

    /// Hacen falta las cámaras del teléfono para saber qué pedirle.
    pub fn needs_cameras(&self) -> bool {
        self.saver || self.lens.is_some()
    }
}

/// Las cámaras del teléfono que miran hacia `facing`, en el orden de
/// Android: la primera es la principal, la que scrcpy usa si no se elige.
pub fn lenses(cameras: &[crate::sys::PhoneCamera], facing: Facing) -> Vec<&crate::sys::PhoneCamera> {
    cameras.iter().filter(|camera| camera.facing == Some(facing)).collect()
}

/// El tamaño del modo ahorro: el más cercano a 480 de alto, de preferencia
/// panorámico (16:9, como 720p) y siempre más chico que 720p.
///
/// Solo tamaños con ancho y alto múltiplos de 8: el moto g34 anuncia 960x540
/// y 1600x900, pero su cámara los rechaza al abrirla.
fn saver_size(sizes: &[(u32, u32)]) -> Option<(u32, u32)> {
    let short = |&(w, h): &(u32, u32)| w.min(h);
    let wide = |&(w, h): &(u32, u32)| {
        let ratio = f64::from(w.max(h)) / f64::from(w.min(h).max(1));
        (ratio - 16.0 / 9.0).abs() <= 16.0 / 9.0 * 0.1
    };
    let fits = |size: &&(u32, u32)| (360..720).contains(&short(size)) && size.0 % 8 == 0 && size.1 % 8 == 0;
    let closest = |list: Vec<&(u32, u32)>| {
        list.into_iter()
            .min_by_key(|size| (short(size).abs_diff(480), std::cmp::Reverse(size.0 * size.1)))
            .copied()
    };
    closest(sizes.iter().filter(fits).filter(|size| wide(size)).collect())
        .or_else(|| closest(sizes.iter().filter(fits).collect()))
}

/// Cómo girar la imagen de la cámara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// Sigue la orientación del teléfono (necesita el giro automático activado).
    Auto,
    /// Giro fijo en grados, en el sentido del reloj: 0, 90, 180 o 270.
    Fixed(u16),
}

impl Options {
    /// Valor de `scrcpy --capture-orientation` para un giro de `degrees`.
    ///
    /// scrcpy aplica el espejo antes de girar; para que el espejo quede
    /// sobre la imagen ya girada hay que pedirle el giro contrario.
    pub fn capture_orientation(&self, degrees: u16) -> String {
        if self.mirror {
            format!("flip{}", (360 - degrees % 360) % 360)
        } else {
            (degrees % 360).to_string()
        }
    }
}

impl Rotation {
    /// El giro fijo que resulta de dar un cuarto de vuelta más.
    pub fn turned(self, clockwise: bool) -> Rotation {
        let current = match self {
            Rotation::Auto => 0,
            Rotation::Fixed(degrees) => degrees % 360,
        };
        Rotation::Fixed((current + if clockwise { 90 } else { 270 }) % 360)
    }

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
    /// Efecto espejo: voltea la imagen de izquierda a derecha.
    pub mirror: bool,
    /// Giro extra a cualquier ángulo, en grados (sentido del reloj).
    pub angle: i16,
    /// Usar también el micrófono del teléfono.
    pub mic: bool,
    /// Mostrar la imagen de la cámara dentro de la ventana.
    pub preview: bool,
    /// Modo ahorro: menos resolución, cuadros y tasa de bits que la calidad
    /// elegida, para que el teléfono gaste menos batería.
    pub saver: bool,
    /// Otro lente del mismo lado (por ejemplo, el gran angular), por su
    /// número de cámara. `None` es el principal.
    pub lens: Option<u16>,
    /// Apagar la pantalla del teléfono mientras la cámara está encendida.
    /// Siempre se apaga; solo `--pantalla-prendida` la deja prendida, y eso no
    /// se guarda.
    pub screen_off: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            facing: Facing::Front,
            quality: Quality::Hd,
            rotation: Rotation::Fixed(0),
            mirror: false,
            angle: 0,
            mic: true,
            preview: true,
            saver: false,
            lens: None,
            screen_off: true,
        }
    }
}

fn path(file: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("ixtli").join(file))
}

/// Escribe un archivo de configuración. Si falla no pasa nada: solo no se recuerda.
fn write(file: &str, text: &str) {
    let Some(path) = path(file) else { return };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, text);
}

pub fn load() -> Options {
    path("opciones.conf")
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(options: &Options) {
    write("opciones.conf", &serialize(options));
}

/// El teléfono que se conectó por WiFi, para reconectarlo sin cable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remembered {
    /// Por cable (`adb tcpip`): `IP:puerto`. Vinculado con código QR: el
    /// nombre con el que el teléfono se anuncia en la red (`adb-…`).
    pub addr: String,
    pub model: String,
    /// Se vinculó con la depuración inalámbrica (código QR o de 6 dígitos).
    pub paired: bool,
    /// Vinculado: la última `IP:puerto` con la que se conectó, por si no se
    /// anuncia en la red (pasa por VPN).
    pub last: Option<String>,
}

impl Remembered {
    /// Este renglón de `adb devices` es el teléfono recordado.
    pub fn is(&self, serial: &str, model: &str, wifi: bool) -> bool {
        if !self.paired {
            return serial == self.addr;
        }
        // adb lo conecta solo con el nombre `adb-…._adb-tls-connect._tcp`;
        // si Ixtli lo conectó por su IP, se reconoce por el modelo.
        wifi && (serial.starts_with(&format!("{}.", self.addr)) || model == self.model)
    }
}

pub fn load_phone() -> Option<Remembered> {
    let text = fs::read_to_string(path("telefono.conf")?).ok()?;
    parse_phone(&text)
}

/// Recuerda el teléfono, o lo olvida con `None`.
pub fn save_phone(phone: Option<&Remembered>) {
    match phone {
        Some(phone) => write(
            "telefono.conf",
            &format!(
                "{}={}\nmodelo={}\n{}",
                if phone.paired { "vinculado" } else { "wifi" },
                phone.addr,
                phone.model,
                phone.last.as_ref().map(|addr| format!("direccion={addr}\n")).unwrap_or_default()
            ),
        ),
        None => {
            if let Some(path) = path("telefono.conf") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn parse_phone(text: &str) -> Option<Remembered> {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.split_once('=').filter(|(k, _)| k.trim() == key))
            .map(|(_, v)| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let model = value("modelo").unwrap_or_else(|| "Android".into());
    // Solo se acepta lo que tiene la forma esperada, porque se le pasa a adb.
    if let Some(name) = value("vinculado") {
        let valid = name.starts_with("adb-")
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        let last = value("direccion").filter(|addr| crate::sys::valid_addr(addr));
        return valid.then_some(Remembered { addr: name, model, paired: true, last });
    }
    let addr = value("wifi").filter(|addr| crate::sys::valid_addr(addr))?;
    Some(Remembered { addr, model, paired: false, last: None })
}

/// Deja el ángulo extra entre -180 y 180 grados.
pub fn clamp_angle(angle: i16) -> i16 {
    angle.clamp(-180, 180)
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
        "camara={}\ncalidad={}\ngiro={}\nespejo={}\nangulo={}\nmicrofono={}\nvista_previa={}\nahorro={}\nlente={}\n",
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
        yes_no(o.mirror),
        o.angle,
        yes_no(o.mic),
        yes_no(o.preview),
        yes_no(o.saver),
        o.lens.map(|id| id.to_string()).unwrap_or_else(|| "principal".into()),
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
            ("espejo", v) => o.mirror = v == "si",
            ("angulo", v) => o.angle = v.parse().map(clamp_angle).unwrap_or(0),
            ("microfono", v) => o.mic = v == "si",
            ("vista_previa", v) => o.preview = v == "si",
            ("ahorro", v) => o.saver = v == "si",
            ("lente", v) => o.lens = v.parse().ok(),
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
            mirror: true,
            angle: -15,
            mic: false,
            preview: false,
            saver: true,
            lens: Some(2),
            screen_off: true,
        };
        assert_eq!(parse(&serialize(&o)), o);
        let auto = Options {
            rotation: Rotation::Auto,
            ..o
        };
        assert_eq!(parse(&serialize(&auto)), auto);
        assert_eq!(parse(&serialize(&Options::default())), Options::default());
        // La pantalla se apaga siempre, aunque un archivo viejo diga que no.
        assert!(parse("pantalla_apagada=no\n").screen_off);
    }

    #[test]
    fn remembered_phone() {
        let phone = parse_phone("wifi=192.168.1.75:5555\nmodelo=SM-A576B\n").unwrap();
        assert_eq!(phone.addr, "192.168.1.75:5555");
        assert_eq!(phone.model, "SM-A576B");
        assert_eq!(parse_phone("wifi=192.168.1.75:5555\n").unwrap().model, "Android");
        assert!(!phone.paired);
        assert_eq!(parse_phone("wifi=-s otro\n"), None);
        let paired = parse_phone("vinculado=adb-ZY32JG58TH-AbCdEf\nmodelo=moto-g34-5G\n").unwrap();
        assert!(paired.paired);
        assert_eq!(parse_phone(&format!("vinculado={}\nmodelo={}\n", paired.addr, paired.model)), Some(paired.clone()));
        assert!(paired.is("adb-ZY32JG58TH-AbCdEf._adb-tls-connect._tcp", "moto-g34-5G", true));
        assert!(paired.is("192.168.1.101:37211", "moto-g34-5G", true));
        assert!(!paired.is("ZY32JG58TH", "moto-g34-5G", false));
        assert_eq!(parse_phone("vinculado=-s rm\n"), None);
        let vpn = parse_phone("vinculado=adb-ZY32-AbCd\nmodelo=moto\ndireccion=100.64.0.7:37211\n").unwrap();
        assert_eq!(vpn.last.as_deref(), Some("100.64.0.7:37211"));
        assert_eq!(parse_phone("vinculado=adb-ZY32-AbCd\ndireccion=-s x\n").unwrap().last, None);
        assert_eq!(parse_phone("wifi=192.168.1.75\n"), None);
        assert_eq!(parse_phone(""), None);
    }

    fn camera(facing: Facing, fps: &[u16], sizes: &[(u32, u32)]) -> crate::sys::PhoneCamera {
        crate::sys::PhoneCamera {
            id: if facing == Facing::Back { "0" } else { "1" }.into(),
            facing: Some(facing),
            sensor: None,
            fps: fps.to_vec(),
            sizes: sizes.to_vec(),
        }
    }

    #[test]
    fn choosing_a_lens() {
        let wide = crate::sys::PhoneCamera { id: "2".into(), ..camera(Facing::Back, &[30], &[(1280, 720), (640, 360)]) };
        let cameras = [camera(Facing::Back, &[15, 30], &[(1280, 720), (854, 480)]), camera(Facing::Front, &[30], &[]), wide];
        assert_eq!(lenses(&cameras, Facing::Back).len(), 2);
        assert_eq!(lenses(&cameras, Facing::Front).len(), 1);
        let back = Options { facing: Facing::Back, ..Options::default() };
        assert!(!back.needs_cameras());
        assert_eq!(back.capture(Some(&cameras)).camera_id, None);
        let on_wide = Options { lens: Some(2), ..back };
        assert!(on_wide.needs_cameras());
        assert_eq!(on_wide.capture(Some(&cameras)).camera_id.as_deref(), Some("2"));
        // El modo ahorro usa los tamaños del lente elegido.
        let saver = Options { saver: true, ..on_wide }.capture(Some(&cameras));
        assert_eq!((saver.size.as_str(), saver.fps), ("640x360", 30));
        // Un lente que este teléfono no tiene, o de otro lado: el principal.
        assert_eq!(Options { lens: Some(7), ..back }.capture(Some(&cameras)).camera_id, None);
        assert_eq!(Options { facing: Facing::Front, ..on_wide }.capture(Some(&cameras)).camera_id, None);
        assert_eq!(on_wide.capture(None).camera_id, None);
        assert_eq!(parse("lente=principal\n").lens, None);
        assert_eq!(parse("lente=3\n").lens, Some(3));
    }

    #[test]
    fn saver_capture() {
        let normal = Options::default();
        assert_eq!(
            normal.capture(None),
            Capture { camera_id: None, size: "1280x720".into(), fps: 30, bit_rate: None }
        );
        assert_eq!(Options { quality: Quality::FullHd, ..normal }.capture(None).size, "1920x1080");

        let saver = Options { saver: true, quality: Quality::FullHd, ..normal };
        // Sin lista del teléfono: 720p, que siempre funciona, con menos tasa de bits.
        assert_eq!(
            saver.capture(None),
            Capture { camera_id: None, size: "1280x720".into(), fps: 30, bit_rate: Some("2M") }
        );
        let cameras = [
            camera(Facing::Back, &[15, 24, 30], &[(1920, 1080), (1280, 720), (864, 480), (640, 480), (320, 240)]),
            camera(Facing::Front, &[24, 30], &[(1920, 1080), (1280, 720), (720, 480), (640, 480), (960, 544)]),
        ];
        let back = Options { facing: Facing::Back, ..saver }.capture(Some(&cameras));
        assert_eq!(back, Capture { camera_id: None, size: "864x480".into(), fps: 15, bit_rate: Some("2M") });
        assert_eq!(back.describe(), "864x480, 15 cuadros/s, 2 Mb/s");
        // La frontal no tiene 16:9 de 480 de alto: el panorámico más cercano es 960x544.
        let front = saver.capture(Some(&cameras));
        assert_eq!((front.size.as_str(), front.fps), ("960x544", 24));
        // Como el moto g34: 960x540 no es múltiplo de 8 y su cámara no lo acepta.
        let moto = [(1280, 720), (960, 720), (960, 540), (960, 432), (720, 480), (640, 480)];
        assert_eq!(saver_size(&moto), Some((720, 480)));
        // Sin panorámicos chicos, cualquier tamaño cercano a 480p.
        assert_eq!(saver_size(&[(1280, 720), (640, 480), (352, 288)]), Some((640, 480)));
        // Nada más chico que 720p: se queda en 720p.
        let only_big = [camera(Facing::Front, &[30], &[(1920, 1080), (1280, 720)])];
        let fallback = saver.capture(Some(&only_big));
        assert_eq!((fallback.size.as_str(), fallback.fps), ("1280x720", 30));
        assert_eq!(normal.capture(None).describe(), "1280x720, 30 cuadros/s");
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(parse("hola\ncamara=lateral\ngiro=45\n=\n"), Options::default());
    }

    #[test]
    fn turning_and_mirror() {
        assert_eq!(Rotation::Fixed(270).turned(true), Rotation::Fixed(0));
        assert_eq!(Rotation::Fixed(0).turned(false), Rotation::Fixed(270));
        assert_eq!(Rotation::Auto.turned(true), Rotation::Fixed(90));
        let plain = Options::default();
        let mirrored = Options { mirror: true, ..plain };
        assert_eq!(plain.capture_orientation(90), "90");
        assert_eq!(mirrored.capture_orientation(0), "flip0");
        assert_eq!(mirrored.capture_orientation(90), "flip270");
        assert_eq!(parse("angulo=999\n").angle, 180);
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
