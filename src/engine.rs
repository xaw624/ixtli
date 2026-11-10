//! El motor: un hilo que vigila el sistema y maneja la sesión de scrcpy,
//! para que ni la ventana ni la terminal se queden esperando.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio;
use crate::config::{self, Capture, Options, Remembered, Rotation};
use crate::measure::{self, Battery};
use crate::pairing::{self, Pairing, Progress};
use crate::session::{Session, LOST_WIFI};
use crate::sys::{self, DeviceState, Deps, Link, Phone, VirtualCam};

/// Cada cuánto se intenta reconectar sin cable el teléfono recordado.
const RECONNECT_EVERY: Duration = Duration::from_secs(10);

/// Cada cuánto se lee la batería del teléfono. Cada lectura abre un proceso
/// en el teléfono, y la batería cambia despacio.
const BATTERY_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Off,
    Starting,
    Live,
}

/// Foto del estado actual, para mostrarla en pantalla.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// Falso hasta que termina la primera revisión del sistema.
    pub checked: bool,
    pub deps: Deps,
    pub phone: Phone,
    pub cam: Option<VirtualCam>,
    pub phase: Phase,
    /// El micrófono del teléfono está en uso en esta sesión.
    pub mic: bool,
    /// Lo que se le pidió a la cámara del teléfono en la sesión en vivo.
    pub capture: Option<Capture>,
    /// La batería del teléfono conectado, si se pudo leer.
    pub battery: Option<Battery>,
    /// Último problema, ya explicado para el usuario.
    pub error: Option<String>,
    pub wifi: WifiStatus,
}

/// La conexión sin cable.
#[derive(Clone, Debug, Default)]
pub struct WifiStatus {
    /// El teléfono que se conectó por WiFi alguna vez.
    pub remembered: Option<Remembered>,
    /// Ese teléfono está conectado por la red ahora mismo.
    pub connected: bool,
    /// Se está pasando el teléfono a WiFi.
    pub busy: bool,
    /// Por qué no se pudo conectar la última vez.
    pub problem: Option<String>,
    /// Vinculación con código QR en curso.
    pub pairing: Option<PairingView>,
}

#[derive(Clone, Debug)]
pub struct PairingView {
    /// Texto del código QR; `None` si se vincula con el código de 6 dígitos.
    pub qr: Option<String>,
    pub progress: Progress,
    /// Dirección de un teléfono que espera código, visto en la red.
    pub suggestion: Option<String>,
}

enum Order {
    Start(Options),
    Stop,
    LoadModule,
    RestartAdb,
    WifiConnect,
    WifiForget,
    PairStart,
    PairCode(String, String),
    PairAddress(String),
    PairCancel,
    Quit,
}

#[derive(Clone)]
pub struct Engine {
    orders: Sender<Order>,
    shared: Arc<Mutex<Snapshot>>,
    worker: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Engine {
    pub fn spawn() -> Engine {
        let (orders, inbox) = mpsc::channel();
        let mut first = Snapshot::default();
        first.wifi.remembered = config::load_phone();
        let shared = Arc::new(Mutex::new(first));
        let worker = {
            let shared = shared.clone();
            thread::spawn(move || work(inbox, shared))
        };
        Engine {
            orders,
            shared,
            worker: Arc::new(Mutex::new(Some(worker))),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap().clone()
    }

    /// Enciende (o reinicia con otras opciones) la cámara.
    pub fn start(&self, options: Options) {
        {
            let mut state = self.shared.lock().unwrap();
            state.phase = Phase::Starting;
            state.error = None;
        }
        let _ = self.orders.send(Order::Start(options));
    }

    pub fn stop(&self) {
        self.shared.lock().unwrap().phase = Phase::Off;
        let _ = self.orders.send(Order::Stop);
    }

    pub fn load_module(&self) {
        let _ = self.orders.send(Order::LoadModule);
    }

    pub fn restart_adb(&self) {
        let _ = self.orders.send(Order::RestartAdb);
    }

    /// Pasa a WiFi el teléfono conectado por USB y lo recuerda.
    pub fn wifi_connect(&self) {
        {
            let mut state = self.shared.lock().unwrap();
            state.wifi.busy = true;
            state.wifi.problem = None;
        }
        let _ = self.orders.send(Order::WifiConnect);
    }

    /// Olvida el teléfono recordado y lo desconecta de la red.
    pub fn wifi_forget(&self) {
        let _ = self.orders.send(Order::WifiForget);
    }

    /// Empieza a vincular un teléfono con código QR (o genera otro código).
    pub fn pair_start(&self) {
        let _ = self.orders.send(Order::PairStart);
    }

    /// Vincula con la dirección y el código de 6 dígitos que muestra el teléfono.
    pub fn pair_code(&self, addr: String, code: String) {
        let _ = self.orders.send(Order::PairCode(addr, code));
    }

    /// Conecta el teléfono ya vinculado por su «Dirección IP y puerto».
    pub fn pair_address(&self, addr: String) {
        let _ = self.orders.send(Order::PairAddress(addr));
    }

    pub fn pair_cancel(&self) {
        self.shared.lock().unwrap().wifi.pairing = None;
        let _ = self.orders.send(Order::PairCancel);
    }

    /// Apaga todo y espera a que el hilo termine de limpiar.
    pub fn shutdown(&self) {
        let _ = self.orders.send(Order::Quit);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

fn update(shared: &Mutex<Snapshot>, change: impl FnOnce(&mut Snapshot)) {
    change(&mut shared.lock().unwrap());
}

fn work(inbox: Receiver<Order>, shared: Arc<Mutex<Snapshot>>) {
    let mut session: Option<Running> = None;
    let mut deps_checked: Option<Instant> = None;
    let mut remembered = shared.lock().unwrap().wifi.remembered.clone();
    let mut last_reconnect: Option<Instant> = None;
    let mut pairing: Option<Pairing> = None;
    // A qué teléfono y cuándo se le leyó la batería por última vez.
    let mut battery_read: Option<(String, Instant)> = None;

    loop {
        // --- Revisar el sistema -------------------------------------------
        let deps_ready = shared.lock().unwrap().deps.ready();
        let deps_stale = deps_checked.is_none_or(|at| at.elapsed() > Duration::from_secs(10));
        if deps_stale || !deps_ready {
            let deps = Deps::check();
            update(&shared, |state| state.deps = deps);
            deps_checked = Some(Instant::now());
        }

        match &mut session {
            // Con la cámara encendida solo se vigila a scrcpy.
            Some(running) => match running.session.finished() {
                Some(outcome) => {
                    let (options, link, with_mic) =
                        (running.options, running.link, running.session.with_mic);
                    end(&mut session, true);
                    // Si se desconectó el cable pero el teléfono sigue por
                    // WiFi, la cámara continúa sin cable.
                    let wifi = remembered
                        .as_ref()
                        .filter(|_| outcome.is_err() && link == Link::Usb)
                        .and_then(|phone| {
                            pairing::matching(phone, &sys::devices())
                                .into_iter()
                                .find(|device| device.state == DeviceState::Ready)
                        });
                    let reopened = match wifi {
                        Some(device) => Some(open(&options, &device.serial, Link::Wifi, with_mic)),
                        None => {
                            if with_mic {
                                audio::remove();
                            }
                            None
                        }
                    };
                    match reopened {
                        Some(Ok(running)) => {
                            session = Some(running);
                            update(&shared, |state| state.phase = Phase::Starting);
                        }
                        Some(Err(problem)) => update(&shared, |state| {
                            state.phase = Phase::Off;
                            state.mic = false;
                            state.error = Some(problem);
                        }),
                        None => update(&shared, |state| {
                            state.phase = Phase::Off;
                            state.mic = false;
                            state.error = outcome.err();
                        }),
                    }
                }
                // Por WiFi la red se puede caer sin que scrcpy se entere.
                None if running.lost() => {
                    end(&mut session, false);
                    update(&shared, |state| {
                        state.phase = Phase::Off;
                        state.mic = false;
                        state.wifi.connected = false;
                        state.error = Some(LOST_WIFI.to_string());
                    });
                }
                None if running.session.is_live() => {
                    let capture = running.capture.clone();
                    update(&shared, |state| {
                        state.phase = Phase::Live;
                        state.capture = Some(capture);
                    });
                    // Giro automático: si el teléfono cambió de posición, se
                    // reabre la cámara con el giro nuevo (tarda un par de segundos).
                    if running.turned() {
                        let (options, serial, link) =
                            (running.options, running.serial.clone(), running.link);
                        end(&mut session, true);
                        match open(&options, &serial, link, true) {
                            Ok(reopened) => {
                                session = Some(reopened);
                                update(&shared, |state| state.phase = Phase::Starting);
                            }
                            Err(problem) => update(&shared, |state| {
                                state.phase = Phase::Off;
                                state.mic = false;
                                state.error = Some(problem);
                            }),
                        }
                    }
                }
                None => {}
            },
            None => {
                let has_adb = shared.lock().unwrap().deps.adb;
                look_for_phone(&shared, has_adb, remembered.as_ref());
                update(&shared, |state| state.checked = true);
                if let Some(job) = &mut pairing {
                    let progress = job.step();
                    let suggestion = job.suggestion();
                    match &progress {
                        Progress::Done(phone) => {
                            if let Some(old) = remembered.as_ref().filter(|old| old.addr != phone.addr) {
                                pairing::disconnect(old);
                            }
                            remember(&shared, &mut remembered, phone.clone());
                            pairing = None;
                            look_for_phone(&shared, has_adb, remembered.as_ref());
                        }
                        Progress::Failed(_) => pairing = None,
                        Progress::Waiting | Progress::Connecting | Progress::NeedAddress(_) => {}
                    }
                    update(&shared, |state| {
                        if let Some(view) = &mut state.wifi.pairing {
                            view.progress = progress;
                            view.suggestion = suggestion;
                        }
                    });
                }
                // Sin teléfono listo, se busca sin cable el que se recuerda.
                let missing = !matches!(shared.lock().unwrap().phone, Phone::Ready { .. });
                let due = last_reconnect.is_none_or(|at| at.elapsed() >= RECONNECT_EVERY);
                if let (true, true, true, Some(phone)) = (has_adb, missing, due, &remembered) {
                    last_reconnect = Some(Instant::now());
                    let result = pairing::reconnect(phone);
                    update(&shared, |state| state.wifi.problem = result.err());
                    look_for_phone(&shared, has_adb, remembered.as_ref());
                }
            }
        }
        update(&shared, |state| state.checked = true);

        // --- Batería del teléfono -----------------------------------------
        let serial = match &shared.lock().unwrap().phone {
            Phone::Ready { serial, .. } => Some(serial.clone()),
            _ => None,
        };
        match serial {
            Some(serial) => {
                // Se lee al aparecer el teléfono (o al cambiar de cable a WiFi)
                // y después cada minuto.
                let due = battery_read
                    .as_ref()
                    .is_none_or(|(read, at)| *read != serial || at.elapsed() >= BATTERY_EVERY);
                if due {
                    let battery = measure::battery(&serial);
                    update(&shared, |state| state.battery = battery);
                    battery_read = Some((serial, Instant::now()));
                }
            }
            None => {
                battery_read = None;
                update(&shared, |state| state.battery = None);
            }
        }

        // --- Esperar la siguiente orden -----------------------------------
        let phase = shared.lock().unwrap().phase;
        let pause = match (&session, phase) {
            (None, _) if pairing.is_some() => Duration::from_millis(800),
            (Some(_), Phase::Live) => Duration::from_millis(1000),
            (Some(_), _) => Duration::from_millis(200),
            (None, _) => Duration::from_millis(1500),
        };
        match inbox.recv_timeout(pause) {
            Ok(Order::Start(options)) => {
                // Si el micrófono sigue en uso se conserva, para que la
                // videollamada no lo pierda al cambiar una opción.
                let keep_mic = options.mic && session.as_ref().is_some_and(|s| s.session.with_mic);
                end(&mut session, keep_mic);
                match begin(&options, keep_mic, remembered.as_ref()) {
                    Ok(started) => {
                        session = Some(started);
                        update(&shared, |state| {
                            state.phase = Phase::Starting;
                            state.mic = options.mic;
                            state.error = None;
                        });
                    }
                    Err(problem) => update(&shared, |state| {
                        state.phase = Phase::Off;
                        state.mic = false;
                        state.error = Some(problem);
                    }),
                }
            }
            Ok(Order::Stop) => {
                end(&mut session, false);
                update(&shared, |state| {
                    state.phase = Phase::Off;
                    state.mic = false;
                    state.error = None;
                });
            }
            Ok(Order::LoadModule) => {
                let result = if sys::virtual_cam().is_some() {
                    Ok(())
                } else {
                    sys::load_module().map(|_| ())
                };
                update(&shared, |state| state.error = result.err());
            }
            Ok(Order::RestartAdb) => sys::restart_adb(),
            Ok(Order::WifiConnect) => {
                let result = if session.is_some() {
                    Err("Apaga la cámara antes de pasar a WiFi.".to_string())
                } else {
                    match sys::phone() {
                        Phone::Ready { serial, model, link: Link::Usb } => {
                            sys::wifi_enable(&serial)
                                .map(|addr| Remembered { addr, model, paired: false, last: None })
                        }
                        Phone::Ready { link: Link::Wifi, .. } => {
                            Err("El teléfono ya está conectado por WiFi.".to_string())
                        }
                        _ => Err("Conecta el teléfono por USB para pasarlo a WiFi.".to_string()),
                    }
                };
                match result {
                    Ok(phone) => {
                        // Si cambió de IP, se suelta la dirección vieja.
                        if let Some(old) = remembered.as_ref().filter(|old| old.addr != phone.addr) {
                            pairing::disconnect(old);
                        }
                        remember(&shared, &mut remembered, phone);
                    }
                    Err(problem) => update(&shared, |state| {
                        state.wifi.busy = false;
                        state.wifi.problem = Some(problem);
                    }),
                }
                let has_adb = shared.lock().unwrap().deps.adb;
                if session.is_none() {
                    look_for_phone(&shared, has_adb, remembered.as_ref());
                }
            }
            Ok(Order::WifiForget) => {
                if let Some(phone) = remembered.take() {
                    if session.as_ref().is_some_and(|running| running.link == Link::Wifi) {
                        end(&mut session, false);
                        update(&shared, |state| {
                            state.phase = Phase::Off;
                            state.mic = false;
                        });
                    }
                    pairing::disconnect(&phone);
                }
                config::save_phone(None);
                update(&shared, |state| state.wifi = WifiStatus::default());
            }
            Ok(Order::PairStart) => {
                let job = Pairing::new();
                let view = PairingView {
                    qr: Some(job.qr_text()),
                    progress: Progress::Waiting,
                    suggestion: None,
                };
                pairing = Some(job);
                update(&shared, |state| {
                    state.wifi.pairing = Some(view);
                    state.wifi.problem = None;
                });
            }
            Ok(Order::PairCode(addr, code)) => {
                match &mut pairing {
                    Some(job) => job.use_code(addr, code),
                    None => pairing = Some(Pairing::with_code(addr, code)),
                }
                update(&shared, |state| {
                    state.wifi.pairing = Some(PairingView {
                        qr: None,
                        progress: Progress::Connecting,
                        suggestion: None,
                    });
                    state.wifi.problem = None;
                });
            }
            Ok(Order::PairAddress(addr)) => {
                if let Some(job) = &mut pairing {
                    job.set_connect_addr(addr);
                    update(&shared, |state| {
                        if let Some(view) = &mut state.wifi.pairing {
                            view.progress = Progress::Connecting;
                        }
                    });
                }
            }
            Ok(Order::PairCancel) => pairing = None,
            Ok(Order::Quit) | Err(RecvTimeoutError::Disconnected) => {
                end(&mut session, false);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Guarda el teléfono que se acaba de conectar sin cable.
fn remember(shared: &Mutex<Snapshot>, remembered: &mut Option<Remembered>, phone: Remembered) {
    config::save_phone(Some(&phone));
    *remembered = Some(phone.clone());
    update(shared, |state| {
        let pairing = state.wifi.pairing.take();
        state.wifi = WifiStatus {
            remembered: Some(phone),
            connected: true,
            busy: false,
            problem: None,
            pairing,
        }
    });
}

/// Revisa qué teléfono hay y si el recordado está conectado por WiFi.
fn look_for_phone(shared: &Mutex<Snapshot>, has_adb: bool, remembered: Option<&Remembered>) {
    let devices = if has_adb { sys::devices() } else { Vec::new() };
    let phone = sys::pick(&devices);
    let connected = remembered.is_some_and(|wanted| {
        pairing::matching(wanted, &devices)
            .iter()
            .any(|device| device.state == DeviceState::Ready)
    });
    let cam = sys::virtual_cam();
    update(shared, |state| {
        state.phone = phone;
        state.cam = cam;
        state.wifi.connected = connected;
        if connected {
            state.wifi.problem = None;
        }
    });
}

/// Una sesión abierta, con lo necesario para reabrirla si hay que girarla.
struct Running {
    session: Session,
    options: Options,
    serial: String,
    link: Link,
    capture: Capture,
    last_turn_check: Instant,
    last_ping: Instant,
    missed_pings: u8,
}

impl Running {
    /// Por WiFi: el teléfono dejó de responder. Pregunta cada 3 segundos y
    /// lo da por perdido después de dos intentos sin respuesta.
    fn lost(&mut self) -> bool {
        if self.link != Link::Wifi || self.last_ping.elapsed() < Duration::from_secs(3) {
            return false;
        }
        self.missed_pings = if sys::reachable(&self.serial) { 0 } else { self.missed_pings + 1 };
        self.last_ping = Instant::now();
        self.missed_pings >= 2
    }

    /// Con giro automático: el teléfono ya no está en la posición con la que
    /// se abrió la cámara. Pregunta al teléfono como mucho cada 1.5 segundos.
    fn turned(&mut self) -> bool {
        if self.options.rotation != Rotation::Auto
            || self.last_turn_check.elapsed() < Duration::from_millis(1500)
        {
            return false;
        }
        self.last_turn_check = Instant::now();
        match sys::phone_rotation(&self.serial) {
            // Sin dato del sensor no se toca nada.
            None => false,
            quarters => self.options.rotation.degrees(self.options.facing, quarters) != self.session.degrees,
        }
    }
}

/// Deja todo listo y abre scrcpy. Cada paso que falla explica qué hacer.
fn begin(options: &Options, mic_ready: bool, remembered: Option<&Remembered>) -> Result<Running, String> {
    let checked = (|| {
        let deps = Deps::check();
        if let Some(problem) = deps.problem(options.mic) {
            return Err(problem);
        }
        let mut phone = sys::phone();
        // Sin cable, se intenta con el teléfono recordado.
        if let (false, Some(wanted)) = (matches!(phone, Phone::Ready { .. }), remembered) {
            match pairing::reconnect(wanted) {
                Ok(()) => phone = sys::phone(),
                Err(problem) if phone == Phone::Missing => {
                    return Err(format!("No encontré el teléfono por USB ni por WiFi. {problem}"))
                }
                Err(_) => {}
            }
        }
        match phone {
            Phone::Ready { serial, link, .. } => Ok((serial, link)),
            other => Err(other.advice().to_string()),
        }
    })();
    match checked {
        Ok((serial, link)) => open(options, &serial, link, mic_ready),
        Err(problem) => {
            if mic_ready {
                audio::remove();
            }
            Err(problem)
        }
    }
}

/// Abre scrcpy hacia la cámara virtual. Si falla, no deja el micrófono puesto.
fn open(options: &Options, serial: &str, link: Link, mic_ready: bool) -> Result<Running, String> {
    let opened = (|| {
        let cam = match sys::virtual_cam() {
            Some(cam) => cam,
            None => sys::load_module()?,
        };
        if cam.streaming() {
            return Err(format!(
                "La cámara virtual «{}» ya está recibiendo video de otro programa (¿scrcpy abierto en una terminal?). Ciérralo e intenta otra vez.",
                cam.label
            ));
        }
        if options.mic && !mic_ready {
            audio::create()?;
        }
        let quarters = match options.rotation {
            Rotation::Auto => sys::phone_rotation(serial),
            Rotation::Fixed(_) => None,
        };
        let degrees = options.rotation.degrees(options.facing, quarters);
        // Solo el modo ahorro necesita saber qué tamaños acepta la cámara.
        let cameras = options.saver.then(|| sys::phone_cameras(serial)).flatten();
        let capture = options.capture(cameras.as_deref());
        Session::start(serial, link, &cam, options, &capture, degrees).map(|session| (session, capture))
    })();
    match opened {
        Ok((session, capture)) => Ok(Running {
            session,
            options: *options,
            serial: serial.to_string(),
            link,
            capture,
            last_turn_check: Instant::now(),
            last_ping: Instant::now(),
            missed_pings: 0,
        }),
        Err(problem) => {
            if options.mic {
                audio::remove();
            }
            Err(problem)
        }
    }
}

/// Cierra la sesión (si hay). El micrófono virtual se quita salvo que la
/// siguiente sesión lo vaya a seguir usando.
fn end(session: &mut Option<Running>, keep_mic: bool) {
    if let Some(running) = session.take() {
        let with_mic = running.session.with_mic;
        running.session.stop();
        if with_mic && !keep_mic {
            audio::remove();
        }
    }
}
