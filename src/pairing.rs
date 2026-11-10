//! Vincular el teléfono sin cable con un código QR, como lo hace Android
//! Studio: Ixtli muestra el código, el teléfono lo escanea desde
//! «Depuración inalámbrica» y se anuncia en la red; Ixtli lo encuentra y
//! completa la vinculación con `adb pair`.

use std::time::{Duration, Instant};

use crate::config::Remembered;
use crate::sys::{self, DeviceState, Link, MdnsKind};

/// Cuánto se espera a que el teléfono escanee el código.
const WAIT_FOR_SCAN: Duration = Duration::from_secs(180);
/// Cuánto se espera a que el teléfono ya vinculado aparezca en la red antes
/// de pedir su dirección.
const WAIT_FOR_CONNECT: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// Esperando a que el teléfono escanee el código.
    Waiting,
    /// Vinculando o ya vinculado; esperando a que se conecte.
    Connecting,
    /// Vinculado, pero no aparece en la red (VPN o red que bloquea los
    /// anuncios): hay que escribir su dirección. Lleva el último error.
    NeedAddress(Option<String>),
    Done(Remembered),
    Failed(String),
}

pub struct Pairing {
    name: String,
    password: String,
    started: Instant,
    /// Vinculación con código: la dirección y el código que muestra el teléfono.
    manual: Option<(String, String)>,
    /// Nombre con el que se anuncia el teléfono ya vinculado, y desde cuándo.
    paired: Option<(String, Instant)>,
    /// Dirección de conexión escrita a mano.
    connect_addr: Option<String>,
    connect_error: Option<String>,
    /// Un teléfono que está esperando código, visto en la red.
    seen: Option<String>,
}

impl Default for Pairing {
    fn default() -> Self {
        Pairing::new()
    }
}

impl Pairing {
    pub fn new() -> Pairing {
        Pairing {
            name: format!("ixtli-{}", sys::random_text(6)),
            password: sys::random_text(10),
            started: Instant::now(),
            manual: None,
            paired: None,
            connect_addr: None,
            connect_error: None,
            seen: None,
        }
    }

    /// Vincular con el código de 6 dígitos en lugar del QR.
    pub fn with_code(addr: String, code: String) -> Pairing {
        let mut pairing = Pairing::new();
        pairing.use_code(addr, code);
        pairing
    }

    pub fn use_code(&mut self, addr: String, code: String) {
        self.manual = Some((addr, code));
    }

    /// La «Dirección IP y puerto» del teléfono, escrita a mano.
    pub fn set_connect_addr(&mut self, addr: String) {
        self.connect_addr = Some(addr);
        self.connect_error = None;
    }

    /// Dirección de un teléfono que espera código, si se vio uno en la red.
    pub fn suggestion(&self) -> Option<String> {
        self.seen.clone()
    }

    /// El texto del código QR, con el formato que entiende Android.
    pub fn qr_text(&self) -> String {
        format!("WIFI:T:ADB;S:{};P:{};;", self.name, self.password)
    }

    /// Da un paso: busca el teléfono, lo vincula o lo conecta.
    pub fn step(&mut self) -> Progress {
        let Some((guid, since)) = self.paired.clone() else {
            if let Some((addr, code)) = self.manual.clone() {
                return match sys::pair(&addr, &code) {
                    Ok(guid) => {
                        self.paired = Some((guid, Instant::now()));
                        Progress::Connecting
                    }
                    Err(problem) => Progress::Failed(problem),
                };
            }
            let services = sys::mdns_services();
            self.seen = services
                .iter()
                .find(|service| service.kind == MdnsKind::Pairing && service.name != self.name)
                .map(|service| service.addr.clone());
            let found = services
                .into_iter()
                .find(|service| service.kind == MdnsKind::Pairing && service.name == self.name);
            return match found {
                Some(service) => match sys::pair(&service.addr, &self.password) {
                    Ok(guid) => {
                        self.paired = Some((guid, Instant::now()));
                        self.step()
                    }
                    Err(problem) => Progress::Failed(problem),
                },
                None if self.started.elapsed() > WAIT_FOR_SCAN => Progress::Failed(
                    "No vi el teléfono. Revisa que esté en la misma red WiFi que esta computadora y genera otro código."
                        .into(),
                ),
                None => Progress::Waiting,
            };
        };

        // Ya vinculado: adb suele conectarlo solo; si no, se le ayuda.
        let remembered = |model: String, last: Option<String>| Remembered {
            addr: guid.clone(),
            model,
            paired: true,
            last,
        };
        let by_name = sys::devices()
            .into_iter()
            .find(|device| {
                device.state == DeviceState::Ready
                    && device.link == Link::Wifi
                    && device.serial.starts_with(&format!("{guid}."))
            })
            .map(|device| device.model);
        if let Some(model) = by_name {
            return Progress::Done(remembered(model, None));
        }
        if let Some(addr) = self.connect_addr.take() {
            match sys::wifi_connect(&addr) {
                Ok(()) => {
                    let model = ready_model(&addr).unwrap_or_else(|| "Android".into());
                    return Progress::Done(remembered(model, Some(addr)));
                }
                Err(_) => {
                    self.connect_error = Some(
                        "No pude conectar con esa dirección. Revisa la IP y el puerto en la pantalla de Depuración inalámbrica."
                            .into(),
                    )
                }
            }
        }
        let announced = sys::mdns_services()
            .into_iter()
            .find(|service| service.kind == MdnsKind::Connect && service.name == guid);
        if let Some(service) = announced {
            if sys::wifi_connect(&service.addr).is_ok() {
                if let Some(model) = ready_model(&service.addr) {
                    return Progress::Done(remembered(model, Some(service.addr)));
                }
            }
        }
        if since.elapsed() > WAIT_FOR_CONNECT {
            return Progress::NeedAddress(self.connect_error.clone());
        }
        Progress::Connecting
    }
}

fn ready_model(serial: &str) -> Option<String> {
    sys::devices()
        .into_iter()
        .find(|device| device.serial == serial && device.state == DeviceState::Ready)
        .map(|device| device.model)
}

/// Vuelve a conectar sin cable el teléfono recordado. Al vinculado con
/// código QR se le busca en la red, porque su puerto cambia.
pub fn reconnect(phone: &Remembered) -> Result<(), String> {
    if !phone.paired {
        return sys::wifi_connect(&phone.addr);
    }
    let announced = sys::mdns_services()
        .into_iter()
        .find(|service| service.kind == MdnsKind::Connect && service.name == phone.addr);
    match announced {
        Some(service) => sys::wifi_connect(&service.addr),
        // Sin anuncios (por ejemplo, por VPN) se prueba la última dirección.
        None => match &phone.last {
            Some(addr) => sys::wifi_connect(addr),
            None => Err(
                "No encuentro el teléfono en la red. Revisa que tenga encendida la depuración inalámbrica y que esté en la misma red WiFi."
                    .into(),
            ),
        },
    }
}

/// El código QR como bloques de texto, para la terminal.
pub fn qr_for_terminal(text: &str) -> Option<String> {
    let (width, dark) = qr_modules(text)?;
    let quiet = 2;
    let size = width + quiet * 2;
    let at = |x: usize, y: usize| -> bool {
        x >= quiet && y >= quiet && x < quiet + width && y < quiet + width && dark[(y - quiet) * width + (x - quiet)]
    };
    // Cada carácter dibuja dos renglones del código.
    let mut out = String::new();
    for y in (0..size).step_by(2) {
        for x in 0..size {
            out.push(match (at(x, y), at(x, y + 1)) {
                (false, false) => '█',
                (false, true) => '▀',
                (true, false) => '▄',
                (true, true) => ' ',
            });
        }
        out.push('\n');
    }
    Some(out)
}

/// Los módulos del código QR: su ancho y cuáles son oscuros.
pub fn qr_modules(text: &str) -> Option<(usize, Vec<bool>)> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let dark = code
        .to_colors()
        .into_iter()
        .map(|color| color == qrcode::Color::Dark)
        .collect();
    Some((width, dark))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_text_format() {
        let pairing = Pairing::new();
        let text = pairing.qr_text();
        assert!(text.starts_with("WIFI:T:ADB;S:ixtli-"));
        assert!(text.ends_with(";;"));
        let (width, dark) = qr_modules(&text).unwrap();
        assert_eq!(dark.len(), width * width);
        assert!(qr_for_terminal(&text).unwrap().lines().count() > 10);
    }
}

/// Los renglones de `adb devices` que son el teléfono recordado.
pub fn matching(phone: &Remembered, devices: &[sys::Device]) -> Vec<sys::Device> {
    devices
        .iter()
        .filter(|device| phone.is(&device.serial, &device.model, device.link == Link::Wifi))
        .cloned()
        .collect()
}

/// Desconecta de la red el teléfono recordado.
pub fn disconnect(phone: &Remembered) {
    let mut serials: Vec<String> =
        matching(phone, &sys::devices()).into_iter().map(|device| device.serial).collect();
    if !phone.paired {
        serials.push(phone.addr.clone());
    }
    serials.dedup();
    for serial in serials {
        sys::wifi_disconnect(&serial);
    }
}
