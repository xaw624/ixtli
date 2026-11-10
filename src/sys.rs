//! Consultas al sistema: programas instalados, teléfono y cámara virtual.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

/// Etiqueta que Ixtli le pone a la cámara virtual cuando la crea.
pub const CAM_LABEL: &str = "Ixtli";
const CAM_NUMBER: u32 = 10;

/// Ixtli usa opciones de scrcpy (girar la imagen, no abrir ventana) que
/// existen desde la versión 3.0.
const MIN_SCRCPY: (u32, u32) = (3, 0);

/// Busca un programa en el PATH y en los directorios de administración,
/// que en Debian no están en el PATH de los usuarios normales.
pub fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from("/usr/sbin"), PathBuf::from("/sbin")])
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
}

pub struct Failure {
    pub code: Option<i32>,
    pub message: String,
}

/// Ejecuta un programa y devuelve su salida estándar.
pub fn run(bin: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String, Failure> {
    let bin = bin.as_ref();
    let output = Command::new(bin)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Failure {
            code: None,
            message: format!("{}: {e}", bin.to_string_lossy()),
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("terminó con error")
        .to_string();
    Err(Failure {
        code: output.status.code(),
        message,
    })
}

// ---------------------------------------------------------------------------
// Programas necesarios
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Tool {
    #[default]
    Missing,
    /// Instalado, pero en una versión sin soporte de cámara.
    TooOld(String),
    Ok,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Deps {
    pub scrcpy: Tool,
    pub adb: bool,
    /// Solo hace falta para el micrófono.
    pub pactl: bool,
    /// El módulo v4l2loopback está instalado (cargado o no).
    pub module: bool,
}

impl Deps {
    pub fn check() -> Deps {
        let scrcpy = match which("scrcpy") {
            None => Tool::Missing,
            Some(bin) => match run(bin, &["--version"]).ok().as_deref().and_then(scrcpy_version) {
                Some((major, minor, text)) if (major, minor) < MIN_SCRCPY => Tool::TooOld(text),
                // Si no se pudo leer la versión, se deja intentar.
                _ => Tool::Ok,
            },
        };
        Deps {
            scrcpy,
            adb: which("adb").is_some(),
            pactl: which("pactl").is_some(),
            module: module_installed(),
        }
    }

    /// Lo indispensable para la cámara está instalado.
    pub fn ready(&self) -> bool {
        self.scrcpy == Tool::Ok && self.adb && self.module
    }

    /// Paquetes (con su nombre en Debian/Ubuntu) que faltan.
    pub fn missing_packages(&self) -> Vec<&'static str> {
        let mut packages = Vec::new();
        if self.scrcpy == Tool::Missing {
            packages.push("scrcpy");
        }
        if !self.adb {
            packages.push("adb");
        }
        if !self.module {
            packages.push("v4l2loopback-dkms");
        }
        if !self.pactl {
            packages.push("pulseaudio-utils");
        }
        packages
    }

    pub fn install_command(&self) -> String {
        format!("sudo apt install {}", self.missing_packages().join(" "))
    }

    /// Explica qué falta, o `None` si se puede encender.
    pub fn problem(&self, need_mic: bool) -> Option<String> {
        if let Tool::TooOld(version) = &self.scrcpy {
            return Some(format!(
                "Tu scrcpy ({version}) es muy viejo: se necesita la versión {}.{} o una más nueva.",
                MIN_SCRCPY.0, MIN_SCRCPY.1
            ));
        }
        if !self.ready() {
            return Some(format!("Falta instalar: {}.", self.missing_packages().join(", ")));
        }
        if need_mic && !self.pactl {
            return Some(
                "Para el micrófono falta instalar pulseaudio-utils (o apaga la opción del micrófono)."
                    .to_string(),
            );
        }
        None
    }
}

/// De "scrcpy 3.3.4 <https://...>" saca (3, 3, "3.3.4").
fn scrcpy_version(output: &str) -> Option<(u32, u32, String)> {
    let version = output
        .lines()
        .next()?
        .strip_prefix("scrcpy ")?
        .split_whitespace()
        .next()?
        .trim_start_matches('v');
    let mut numbers = version.split('.').map(|n| n.parse::<u32>().ok());
    let major = numbers.next()??;
    let minor = numbers.next().flatten().unwrap_or(0);
    Some((major, minor, version.to_string()))
}

fn module_installed() -> bool {
    if Path::new("/sys/module/v4l2loopback").exists() {
        return true;
    }
    let Ok(release) = fs::read_to_string("/proc/sys/kernel/osrelease") else {
        return false;
    };
    fs::read_to_string(format!("/lib/modules/{}/modules.dep", release.trim()))
        .map(|deps| deps.contains("v4l2loopback.ko"))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Teléfono
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phone {
    /// No hay ningún teléfono con depuración USB conectado.
    #[default]
    Missing,
    /// Conectado, pero falta aceptar el aviso de depuración USB.
    Unauthorized,
    /// Linux no deja usar el dispositivo USB (reglas udev).
    NoPermission,
    Offline,
    Ready {
        serial: String,
        model: String,
    },
}

impl Phone {
    /// Qué tiene que hacer el usuario para que el teléfono quede listo.
    pub fn advice(&self) -> &'static str {
        match self {
            Phone::Missing => "Conecta tu teléfono por USB y activa la depuración USB.",
            Phone::Unauthorized => {
                "Desbloquea el teléfono y acepta el aviso «¿Permitir depuración USB?»."
            }
            Phone::NoPermission => {
                "Linux no tiene permiso para usar el teléfono por USB. Desconéctalo y vuelve a conectarlo."
            }
            Phone::Offline => "El teléfono no responde. Desconéctalo y vuelve a conectarlo.",
            Phone::Ready { .. } => "Teléfono listo.",
        }
    }
}

pub fn phone() -> Phone {
    match run("adb", &["devices", "-l"]) {
        Ok(output) => parse_devices(&output),
        Err(_) => Phone::Missing,
    }
}

/// Interpreta la salida de `adb devices -l`. Devuelve el primer teléfono
/// listo; si no hay ninguno, el problema del que sí esté conectado.
fn parse_devices(output: &str) -> Phone {
    let mut found = Phone::Missing;
    let devices = output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices"))
        .skip(1);
    for line in devices {
        let mut fields = line.split_whitespace();
        let (Some(serial), Some(state)) = (fields.next(), fields.next()) else {
            continue;
        };
        match state {
            "device" => {
                let model = fields
                    .find_map(|field| field.strip_prefix("model:"))
                    .unwrap_or("Android")
                    .replace('_', "-");
                return Phone::Ready {
                    serial: serial.to_string(),
                    model,
                };
            }
            "unauthorized" => found = Phone::Unauthorized,
            // "no permissions (...)" se parte en "no" y lo demás.
            "no" if found == Phone::Missing => found = Phone::NoPermission,
            "offline" | "authorizing" | "connecting" if found == Phone::Missing => {
                found = Phone::Offline
            }
            _ => {}
        }
    }
    found
}

/// Cuánto está girado el teléfono, en cuartos de vuelta (0 = vertical).
///
/// Usa el sensor de orientación de Android, que solo está activo con el giro
/// automático encendido y la pantalla prendida. Si no, devuelve `None`.
pub fn phone_rotation(serial: &str) -> Option<u8> {
    let output = run(
        "adb",
        &["-s", serial, "shell", "dumpsys window | grep -A12 WindowOrientationListener"],
    )
    .ok()?;
    parse_rotation(&output)
}

fn parse_rotation(dump: &str) -> Option<u8> {
    let value = |key: &str| {
        dump.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
    };
    if value("mEnabled")? != "true" {
        return None;
    }
    // Android escribe "ROTATION_90" o el número de cuartos de vuelta.
    let quarters = |text: &str| -> Option<u8> {
        let text = text.trim();
        match text.strip_prefix("ROTATION_") {
            Some(degrees) => Some((degrees.parse::<u16>().ok()? / 90) as u8),
            None => text.parse::<u8>().ok(),
        }
        .filter(|quarters| *quarters < 4)
    };
    // La propuesta del sensor es lo más reciente; vale -1 mientras no cambia.
    value("mProposedRotation")
        .and_then(quarters)
        .or_else(|| value("mCurrentRotation").and_then(quarters))
}

/// Reinicia el servidor de adb. Suele destrabar un teléfono que sigue
/// "sin autorizar" después de aceptar el aviso.
pub fn restart_adb() {
    let _ = run("adb", &["kill-server"]);
    let _ = run("adb", &["start-server"]);
}

// ---------------------------------------------------------------------------
// Cámara virtual (v4l2loopback)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct VirtualCam {
    /// Por ejemplo `/dev/video10`.
    pub path: PathBuf,
    /// Nombre con el que aparece en las videollamadas.
    pub label: String,
    sysfs: PathBuf,
}

impl VirtualCam {
    /// Tamaño del video que está llegando, si es del formato que Ixtli sabe
    /// mostrar (YU12, el que usa scrcpy).
    pub fn frame_size(&self) -> Option<(usize, usize)> {
        let format = fs::read_to_string(self.sysfs.join("format")).ok()?;
        parse_format(&format)
    }

    /// Algún programa ya está enviando video a la cámara virtual.
    pub fn streaming(&self) -> bool {
        match fs::read_to_string(self.sysfs.join("state")) {
            Ok(state) => state.trim() == "capture",
            // Versiones viejas de v4l2loopback no tienen "state".
            Err(_) if !self.sysfs.join("state").exists() => fs::read_to_string(self.sysfs.join("format"))
                .map(|format| !format.trim().is_empty())
                .unwrap_or(false),
            Err(_) => false,
        }
    }
}

/// De "YU12:1280x720@30" saca (1280, 720).
fn parse_format(format: &str) -> Option<(usize, usize)> {
    let size = format.trim().strip_prefix("YU12:")?;
    let (width, rest) = size.split_once('x')?;
    let height = rest.split('@').next()?;
    let (width, height) = (width.parse().ok()?, height.parse().ok()?);
    (width > 0 && height > 0 && width <= 8192 && height <= 8192).then_some((width, height))
}

/// Busca una cámara virtual ya creada. Prefiere la que creó Ixtli.
pub fn virtual_cam() -> Option<VirtualCam> {
    let mut cams: Vec<(u32, VirtualCam)> = fs::read_dir("/sys/devices/virtual/video4linux")
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let number: u32 = name.strip_prefix("video")?.parse().ok()?;
            let sysfs = entry.path();
            // "max_openers" solo existe en dispositivos de v4l2loopback.
            if !sysfs.join("max_openers").exists() {
                return None;
            }
            let label = fs::read_to_string(sysfs.join("name")).ok()?.trim().to_string();
            let path = PathBuf::from(format!("/dev/{name}"));
            Some((number, VirtualCam { path, label, sysfs }))
        })
        .collect();
    cams.sort_by_key(|(number, cam)| (cam.label != CAM_LABEL, *number));
    cams.into_iter().next().map(|(_, cam)| cam)
}

/// Comando para activar la cámara virtual a mano.
pub fn module_command() -> String {
    format!("sudo modprobe v4l2loopback video_nr={CAM_NUMBER} card_label={CAM_LABEL} exclusive_caps=1")
}

/// Carga el módulo v4l2loopback pidiendo la contraseña con una ventana (pkexec).
pub fn load_module() -> Result<VirtualCam, String> {
    let manual = format!("Actívala a mano con: {}", module_command());
    let modprobe = which("modprobe").ok_or_else(|| format!("No encontré modprobe. {manual}"))?;
    let pkexec = which("pkexec").ok_or_else(|| format!("No encontré pkexec. {manual}"))?;
    let video_nr = format!("video_nr={CAM_NUMBER}");
    let card_label = format!("card_label={CAM_LABEL}");
    let args = [
        modprobe.to_str().unwrap_or("modprobe"),
        "v4l2loopback",
        &video_nr,
        &card_label,
        "exclusive_caps=1",
    ];
    match run(pkexec, &args) {
        Ok(_) => {}
        // 126: se canceló la ventana. 127: contraseña incorrecta o sin permiso.
        Err(Failure { code: Some(126 | 127), .. }) => {
            return Err("No se dio permiso de administrador para activar la cámara virtual.".into())
        }
        Err(failure) => {
            return Err(format!(
                "No se pudo activar la cámara virtual: {}",
                failure.message
            ))
        }
    }
    // El dispositivo tarda un instante en aparecer con sus permisos.
    for _ in 0..20 {
        if let Some(cam) = virtual_cam() {
            return Ok(cam);
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("El módulo se cargó, pero la cámara virtual no apareció.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version() {
        assert_eq!(
            scrcpy_version("scrcpy 3.3.4 <https://github.com/Genymobile/scrcpy>\n"),
            Some((3, 3, "3.3.4".to_string()))
        );
        assert_eq!(scrcpy_version("scrcpy v2.4\n"), Some((2, 4, "2.4".to_string())));
        assert_eq!(scrcpy_version("scrcpy 1.25"), Some((1, 25, "1.25".to_string())));
        assert_eq!(scrcpy_version("otra cosa"), None);
    }

    #[test]
    fn video_format() {
        assert_eq!(parse_format("YU12:720x1280@30\n"), Some((720, 1280)));
        assert_eq!(parse_format("YUYV:640x480@30"), None);
        assert_eq!(parse_format(""), None);
    }

    #[test]
    fn device_ready() {
        let output = "List of devices attached\n\
            ABC123                 device usb:1-3 product:a57x model:SM_A576B device:a57x transport_id:1\n\n";
        assert_eq!(
            parse_devices(output),
            Phone::Ready {
                serial: "ABC123".into(),
                model: "SM-A576B".into()
            }
        );
    }

    #[test]
    fn device_problems() {
        assert_eq!(parse_devices("List of devices attached\n\n"), Phone::Missing);
        assert_eq!(
            parse_devices("List of devices attached\nABC123  unauthorized usb:1-3 transport_id:1\n"),
            Phone::Unauthorized
        );
        assert_eq!(
            parse_devices("List of devices attached\nABC123  no permissions (user in plugdev group; are your udev rules wrong?); see [http://developer.android.com/tools/device.html]\n"),
            Phone::NoPermission
        );
        assert_eq!(
            parse_devices("List of devices attached\nABC123  offline\n"),
            Phone::Offline
        );
    }

    #[test]
    fn rotation_from_sensor() {
        let off = "    WindowOrientationListener\n      mEnabled=false\n      mCurrentRotation=ROTATION_0\n";
        assert_eq!(parse_rotation(off), None);
        let steady = "    WindowOrientationListener\n      mEnabled=true\n      mCurrentRotation=ROTATION_90\n        mProposedRotation=-1\n";
        assert_eq!(parse_rotation(steady), Some(1));
        let turning = "      mEnabled=true\n      mCurrentRotation=ROTATION_0\n        mProposedRotation=3\n";
        assert_eq!(parse_rotation(turning), Some(3));
        assert_eq!(parse_rotation("nada"), None);
    }

    #[test]
    fn ready_device_wins() {
        let output = "* daemon started successfully\n\
            List of devices attached\n\
            AAA  unauthorized usb:1-1\n\
            BBB  device usb:1-2 model:Pixel_8\n";
        assert_eq!(
            parse_devices(output),
            Phone::Ready {
                serial: "BBB".into(),
                model: "Pixel-8".into()
            }
        );
    }
}
