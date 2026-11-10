//! Consultas al sistema: programas instalados, teléfono y cámara virtual.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    /// El programa no terminó a tiempo y se cerró a la fuerza.
    pub timed_out: bool,
}

/// Ejecuta un programa y devuelve su salida estándar.
pub fn run(bin: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String, Failure> {
    let bin = bin.as_ref();
    let output = Command::new(bin)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| launch_failure(bin, e))?;
    finish(output)
}

/// Como [`run`], pero si el programa tarda más de `limit` lo cierra. Sirve
/// para adb: con la red caída, sus comandos se pueden quedar esperando.
pub fn run_for(bin: impl AsRef<std::ffi::OsStr>, args: &[&str], limit: Duration) -> Result<String, Failure> {
    let bin = bin.as_ref();
    let mut child = Command::new(bin)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| launch_failure(bin, e))?;
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure {
                    code: None,
                    message: format!("{} no respondió a tiempo", bin.to_string_lossy()),
                    timed_out: true,
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(launch_failure(bin, e)),
        }
    }
    let output = child.wait_with_output().map_err(|e| launch_failure(bin, e))?;
    finish(output)
}

fn launch_failure(bin: &std::ffi::OsStr, error: std::io::Error) -> Failure {
    Failure {
        code: None,
        message: format!("{}: {error}", bin.to_string_lossy()),
        timed_out: false,
    }
}

fn finish(output: std::process::Output) -> Result<String, Failure> {
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
        timed_out: false,
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

/// Por dónde está conectado el teléfono.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Link {
    #[default]
    Usb,
    Wifi,
}

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
        link: Link,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Ready,
    Unauthorized,
    NoPermission,
    Offline,
    Other,
}

/// Un renglón de `adb devices -l`.
#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub serial: String,
    pub state: DeviceState,
    pub model: String,
    pub link: Link,
}

/// Los dispositivos que ve adb. Vacío si adb no responde.
pub fn devices() -> Vec<Device> {
    run_for("adb", &["devices", "-l"], Duration::from_secs(5))
        .map(|output| parse_devices(&output))
        .unwrap_or_default()
}

pub fn phone() -> Phone {
    pick(&devices())
}

/// Interpreta la salida de `adb devices -l`.
fn parse_devices(output: &str) -> Vec<Device> {
    output
        .lines()
        .skip_while(|line| !line.starts_with("List of devices"))
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?.to_string();
            let state = match fields.next()? {
                "device" => DeviceState::Ready,
                "unauthorized" => DeviceState::Unauthorized,
                // "no permissions (...)" se parte en "no" y lo demás.
                "no" => DeviceState::NoPermission,
                "offline" | "authorizing" | "connecting" => DeviceState::Offline,
                _ => DeviceState::Other,
            };
            let rest: Vec<&str> = fields.collect();
            let model = rest
                .iter()
                .find_map(|field| field.strip_prefix("model:"))
                .unwrap_or("Android")
                .replace('_', "-");
            // Los conectados por cable dicen en qué puerto USB están.
            let link = if rest.iter().any(|field| field.starts_with("usb:")) {
                Link::Usb
            } else {
                Link::Wifi
            };
            Some(Device { serial, state, model, link })
        })
        .collect()
}

/// Elige el teléfono que se va a usar. Si el mismo teléfono aparece por USB y
/// por WiFi se prefiere el cable, que es más rápido y estable. Si no hay
/// ninguno listo, devuelve el problema del que sí esté conectado.
///
/// Un teléfono por WiFi que no responde no cuenta como problema: adb lo
/// sigue intentando y Ixtli lo reconecta solo.
pub fn pick(devices: &[Device]) -> Phone {
    let ready = |link| {
        devices
            .iter()
            .find(|device| device.state == DeviceState::Ready && device.link == link)
    };
    if let Some(device) = ready(Link::Usb).or_else(|| ready(Link::Wifi)) {
        return Phone::Ready {
            serial: device.serial.clone(),
            model: device.model.clone(),
            link: device.link,
        };
    }
    let mut found = Phone::Missing;
    for device in devices {
        match device.state {
            DeviceState::Unauthorized => found = Phone::Unauthorized,
            DeviceState::NoPermission if found == Phone::Missing => found = Phone::NoPermission,
            DeviceState::Offline if found == Phone::Missing && device.link == Link::Usb => {
                found = Phone::Offline
            }
            _ => {}
        }
    }
    found
}

// ---------------------------------------------------------------------------
// Conexión por WiFi
// ---------------------------------------------------------------------------

/// Puerto en el que el teléfono escucha a adb por la red.
const WIFI_PORT: u16 = 5555;

/// Pasa a WiFi un teléfono conectado por USB: lee su IP, le pide a adb que
/// escuche por la red y se conecta. Devuelve la dirección (`IP:puerto`).
pub fn wifi_enable(serial: &str) -> Result<String, String> {
    let addresses = run_for(
        "adb",
        &["-s", serial, "shell", "ip -o -f inet addr show"],
        Duration::from_secs(5),
    )
    .map_err(|failure| format!("No pude leer la red del teléfono: {}", failure.message))?;
    let ip = parse_wifi_ip(&addresses)
        .ok_or("El teléfono no está conectado a una red WiFi. Conéctalo a la misma red que esta computadora.")?;
    let port = WIFI_PORT.to_string();
    run_for("adb", &["-s", serial, "tcpip", &port], Duration::from_secs(10))
        .map_err(|failure| format!("El teléfono no aceptó la conexión por WiFi: {}", failure.message))?;
    let addr = format!("{ip}:{WIFI_PORT}");
    // adb se reinicia en el teléfono para escuchar por la red: tarda un poco.
    let mut last = String::new();
    for _ in 0..5 {
        thread::sleep(Duration::from_millis(800));
        match wifi_connect(&addr) {
            Ok(()) => return Ok(addr),
            Err(problem) => last = problem,
        }
    }
    Err(last)
}

/// Se conecta a un teléfono que ya estuvo por WiFi.
pub fn wifi_connect(addr: &str) -> Result<(), String> {
    // Si adb lo tiene como "sin respuesta", hay que soltarlo para reintentar.
    if devices()
        .iter()
        .any(|device| device.serial == addr && device.state != DeviceState::Ready)
    {
        wifi_disconnect(addr);
    }
    let reply = run_for("adb", &["connect", addr], Duration::from_secs(5)).map_err(|failure| {
        if failure.timed_out {
            explain_connect("timed out")
        } else {
            explain_connect(&failure.message)
        }
    })?;
    let lower = reply.to_lowercase();
    if !lower.contains("connected to") || lower.contains("failed") || lower.contains("cannot") {
        return Err(explain_connect(&reply));
    }
    // Ya conectado: espera a que adb lo marque como listo.
    for _ in 0..10 {
        match devices().iter().find(|device| device.serial == addr).map(|device| device.state) {
            Some(DeviceState::Ready) => return Ok(()),
            Some(DeviceState::Unauthorized) => {
                return Err("Desbloquea el teléfono y acepta el aviso de depuración.".into())
            }
            _ => thread::sleep(Duration::from_millis(200)),
        }
    }
    Err(explain_connect("timed out"))
}

pub fn wifi_disconnect(addr: &str) {
    let _ = run_for("adb", &["disconnect", addr], Duration::from_secs(3));
}

/// El teléfono sigue respondiendo (sirve para notar que se cayó la red).
pub fn reachable(serial: &str) -> bool {
    run_for("adb", &["-s", serial, "shell", "echo", "ok"], Duration::from_secs(3))
        .is_ok_and(|output| output.trim() == "ok")
}

/// De `ip -o -f inet addr show` saca la IP de la red WiFi. Prefiere `wlan*`;
/// si el teléfono comparte su propia red (zona WiFi), usa esa.
fn parse_wifi_ip(output: &str) -> Option<String> {
    let mut found: Vec<(u8, String)> = output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let interface = fields.nth(1)?;
            fields.find(|field| *field == "inet")?;
            let ip = fields.next()?.split('/').next()?.to_string();
            let rank = if interface.starts_with("wlan") {
                0
            } else if interface.contains("wlan") || interface.starts_with("ap") {
                1
            } else {
                return None;
            };
            Some((rank, ip))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().next().map(|(_, ip)| ip)
}

// ---------------------------------------------------------------------------
// Depuración inalámbrica (Android 11 o más nuevo)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MdnsKind {
    /// El teléfono está esperando que lo vinculen.
    Pairing,
    /// Un teléfono con la depuración inalámbrica encendida.
    Connect,
    Other,
}

/// Un servicio que adb encontró en la red.
#[derive(Clone, Debug, PartialEq)]
pub struct MdnsService {
    pub name: String,
    pub kind: MdnsKind,
    /// `IP:puerto`.
    pub addr: String,
}

/// Lo que los teléfonos anuncian en la red (mDNS), según adb.
pub fn mdns_services() -> Vec<MdnsService> {
    run_for("adb", &["mdns", "services"], Duration::from_secs(4))
        .map(|output| parse_mdns(&output))
        .unwrap_or_default()
}

/// Interpreta `adb mdns services`: «nombre  tipo  IP:puerto» por renglón.
fn parse_mdns(output: &str) -> Vec<MdnsService> {
    let mut services: Vec<MdnsService> = output
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, kind, addr] = fields[..] else { return None };
            let (ip, port) = addr.rsplit_once(':')?;
            port.parse::<u16>().ok()?;
            ip.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok()?;
            let kind = if kind.contains("_adb-tls-pairing") {
                MdnsKind::Pairing
            } else if kind.contains("_adb-tls-connect") {
                MdnsKind::Connect
            } else {
                MdnsKind::Other
            };
            Some(MdnsService { name: name.to_string(), kind, addr: addr.to_string() })
        })
        .collect();
    // Las direcciones IPv4 primero: son las que adb maneja sin problema.
    services.sort_by_key(|service| service.addr.matches(':').count() > 1);
    services
}

/// Vincula el teléfono con la contraseña del código QR. Devuelve el nombre
/// con el que el teléfono se anuncia en la red de ahí en adelante.
pub fn pair(addr: &str, password: &str) -> Result<String, String> {
    let reply = run_for("adb", &["pair", addr, password], Duration::from_secs(20)).map_err(|failure| {
        if failure.timed_out {
            "El teléfono no terminó de vincularse. Intenta otra vez.".to_string()
        } else {
            explain_pair(&failure.message)
        }
    })?;
    parse_pair(&reply).ok_or_else(|| explain_pair(&reply))
}

/// De «Successfully paired to 192.168.1.5:37123 [guid=adb-ZY32-AbCd]» saca el guid.
fn parse_pair(output: &str) -> Option<String> {
    let line = output.lines().find(|line| line.contains("Successfully paired"))?;
    let guid = line.split("guid=").nth(1)?.trim_end_matches(|c: char| c == ']' || c.is_whitespace());
    Some(guid.to_string()).filter(|guid| !guid.is_empty())
}

fn explain_pair(raw: &str) -> String {
    if raw.to_lowercase().contains("password") {
        "El teléfono rechazó el código. Genera otro e intenta de nuevo.".into()
    } else {
        format!("No se pudo vincular el teléfono: {}", raw.trim())
    }
}

/// Texto al azar para el código QR (sin letras que se confundan).
pub fn random_text(length: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = vec![0u8; length];
    let filled = fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
        .is_ok();
    if !filled {
        // Sin /dev/urandom: algo de azar del reloj y del proceso.
        use std::hash::{BuildHasher, Hasher};
        for (index, byte) in bytes.iter_mut().enumerate() {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_usize(index);
            *byte = hasher.finish() as u8;
        }
    }
    bytes
        .iter()
        .map(|byte| ALPHABET[*byte as usize % ALPHABET.len()] as char)
        .collect()
}

/// El texto tiene forma de `IP:puerto` (lo único que se le pasa a adb).
pub fn valid_addr(text: &str) -> bool {
    text.rsplit_once(':').is_some_and(|(ip, port)| {
        port.parse::<u16>().is_ok() && ip.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok()
    })
}

/// Traduce lo que dice `adb connect` cuando no se pudo conectar.
fn explain_connect(raw: &str) -> String {
    let lower = raw.to_lowercase();
    if lower.contains("refused") {
        "El teléfono no está escuchando por WiFi (pasa al reiniciarlo). Conéctalo por USB una vez y presiona «Conectar por WiFi».".into()
    } else if lower.contains("timed out")
        || lower.contains("no route")
        || lower.contains("unreachable")
        || lower.contains("host is down")
    {
        "No encuentro el teléfono en la red. Revisa que el teléfono y esta computadora estén en la misma red WiFi.".into()
    } else if lower.contains("authenticate") {
        "Desbloquea el teléfono y acepta el aviso de depuración.".into()
    } else {
        format!("No se pudo conectar por WiFi: {}", raw.trim())
    }
}

/// Cuánto está girado el teléfono, en cuartos de vuelta (0 = vertical).
///
/// Usa el sensor de orientación de Android, que solo está activo con el giro
/// automático encendido y la pantalla prendida. Si no, devuelve `None`.
pub fn phone_rotation(serial: &str) -> Option<u8> {
    let output = run_for(
        "adb",
        &["-s", serial, "shell", "dumpsys window | grep -A12 WindowOrientationListener"],
        Duration::from_secs(3),
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

    fn phone_from(output: &str) -> Phone {
        pick(&parse_devices(output))
    }

    #[test]
    fn device_ready() {
        let output = "List of devices attached\n\
            ABC123                 device usb:1-3 product:a57x model:SM_A576B device:a57x transport_id:1\n\n";
        assert_eq!(
            phone_from(output),
            Phone::Ready {
                serial: "ABC123".into(),
                model: "SM-A576B".into(),
                link: Link::Usb,
            }
        );
    }

    #[test]
    fn device_problems() {
        assert_eq!(phone_from("List of devices attached\n\n"), Phone::Missing);
        assert_eq!(
            phone_from("List of devices attached\nABC123  unauthorized usb:1-3 transport_id:1\n"),
            Phone::Unauthorized
        );
        assert_eq!(
            phone_from("List of devices attached\nABC123  no permissions (user in plugdev group; are your udev rules wrong?); see [http://developer.android.com/tools/device.html]\n"),
            Phone::NoPermission
        );
        assert_eq!(
            phone_from("List of devices attached\nABC123  offline usb:1-3\n"),
            Phone::Offline
        );
    }

    #[test]
    fn usb_wins_over_wifi() {
        let output = "List of devices attached\n\
            192.168.1.75:5555      device product:a57x model:SM_A576B device:a57x transport_id:4\n\
            ABC123                 device usb:1-3 product:a57x model:SM_A576B device:a57x transport_id:3\n";
        assert_eq!(
            phone_from(output),
            Phone::Ready {
                serial: "ABC123".into(),
                model: "SM-A576B".into(),
                link: Link::Usb,
            }
        );
        let wifi_only = "List of devices attached\n\
            192.168.1.75:5555      device product:a57x model:SM_A576B device:a57x transport_id:4\n";
        assert_eq!(
            phone_from(wifi_only),
            Phone::Ready {
                serial: "192.168.1.75:5555".into(),
                model: "SM-A576B".into(),
                link: Link::Wifi,
            }
        );
        // Un teléfono por WiFi sin respuesta no es un problema que mostrar.
        assert_eq!(
            phone_from("List of devices attached\n192.168.1.75:5555  offline transport_id:4\n"),
            Phone::Missing
        );
    }

    #[test]
    fn wifi_ip() {
        let output = "1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever\n\
            12: rmnet_data0    inet 10.45.2.9/30 scope global rmnet_data0\\       valid_lft forever\n\
            30: wlan0    inet 192.168.1.75/24 brd 192.168.1.255 scope global wlan0\\       valid_lft forever\n";
        assert_eq!(parse_wifi_ip(output).as_deref(), Some("192.168.1.75"));
        let hotspot = "31: swlan0    inet 10.180.33.1/24 scope global swlan0\n";
        assert_eq!(parse_wifi_ip(hotspot).as_deref(), Some("10.180.33.1"));
        assert_eq!(parse_wifi_ip("1: lo    inet 127.0.0.1/8 scope host lo\n"), None);
    }

    #[test]
    fn mdns_list() {
        let output = "List of discovered mdns services\n\
            adb-ZY32JG58TH-AbCdEf\t_adb-tls-connect._tcp\t192.168.1.101:37211\n\
            ixtli-k3m9px\t_adb-tls-pairing._tcp\t192.168.1.101:41875\n\
            adb-ZY32JG58TH-AbCdEf\t_adb-tls-connect._tcp\tfe80::1:37211\n\
            basura\n";
        let services = parse_mdns(output);
        assert_eq!(services.len(), 3);
        assert_eq!(services[0].kind, MdnsKind::Connect);
        assert_eq!(services[0].addr, "192.168.1.101:37211");
        assert_eq!(services[1].name, "ixtli-k3m9px");
        assert_eq!(services[1].kind, MdnsKind::Pairing);
        assert_eq!(services[2].addr, "fe80::1:37211");
    }

    #[test]
    fn pair_reply() {
        assert_eq!(
            parse_pair("Successfully paired to 192.168.1.101:41875 [guid=adb-ZY32JG58TH-AbCdEf]\n").as_deref(),
            Some("adb-ZY32JG58TH-AbCdEf")
        );
        assert_eq!(parse_pair("Failed: Wrong password or connection was dropped.\n"), None);
        assert!(explain_pair("Failed: Wrong password or connection was dropped.").contains("código"));
        let text = random_text(10);
        assert_eq!(text.len(), 10);
        assert!(text.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn connect_errors() {
        assert!(explain_connect("failed to connect to '192.168.1.75:5555': Connection refused").contains("USB"));
        assert!(explain_connect("cannot connect to 192.168.1.75:5555: No route to host (113)").contains("misma red"));
        assert!(explain_connect("timed out").contains("misma red"));
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
            phone_from(output),
            Phone::Ready {
                serial: "BBB".into(),
                model: "Pixel-8".into(),
                link: Link::Usb,
            }
        );
    }
}
