//! El motor: un hilo que vigila el sistema y maneja la sesión de scrcpy,
//! para que ni la ventana ni la terminal se queden esperando.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio;
use crate::config::{Options, Rotation};
use crate::session::Session;
use crate::sys::{self, Deps, Phone, VirtualCam};

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
    /// Último problema, ya explicado para el usuario.
    pub error: Option<String>,
}

enum Order {
    Start(Options),
    Stop,
    LoadModule,
    RestartAdb,
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
        let shared = Arc::new(Mutex::new(Snapshot::default()));
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
                    end(&mut session, false);
                    update(&shared, |state| {
                        state.phase = Phase::Off;
                        state.mic = false;
                        state.error = outcome.err();
                    });
                }
                None if running.session.is_live() => {
                    update(&shared, |state| state.phase = Phase::Live);
                    // Giro automático: si el teléfono cambió de posición, se
                    // reabre la cámara con el giro nuevo (tarda un par de segundos).
                    if running.turned() {
                        let (options, serial) = (running.options, running.serial.clone());
                        end(&mut session, true);
                        match open(&options, &serial, true) {
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
                let phone = if has_adb { sys::phone() } else { Phone::Missing };
                let cam = sys::virtual_cam();
                update(&shared, |state| {
                    state.phone = phone;
                    state.cam = cam;
                });
            }
        }
        update(&shared, |state| state.checked = true);

        // --- Esperar la siguiente orden -----------------------------------
        let phase = shared.lock().unwrap().phase;
        let pause = match (&session, phase) {
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
                match begin(&options, keep_mic) {
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
            Ok(Order::Quit) | Err(RecvTimeoutError::Disconnected) => {
                end(&mut session, false);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Una sesión abierta, con lo necesario para reabrirla si hay que girarla.
struct Running {
    session: Session,
    options: Options,
    serial: String,
    last_turn_check: Instant,
}

impl Running {
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
fn begin(options: &Options, mic_ready: bool) -> Result<Running, String> {
    let checked = (|| {
        let deps = Deps::check();
        if let Some(problem) = deps.problem(options.mic) {
            return Err(problem);
        }
        match sys::phone() {
            Phone::Ready { serial, .. } => Ok(serial),
            other => Err(other.advice().to_string()),
        }
    })();
    match checked {
        Ok(serial) => open(options, &serial, mic_ready),
        Err(problem) => {
            if mic_ready {
                audio::remove();
            }
            Err(problem)
        }
    }
}

/// Abre scrcpy hacia la cámara virtual. Si falla, no deja el micrófono puesto.
fn open(options: &Options, serial: &str, mic_ready: bool) -> Result<Running, String> {
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
        Session::start(serial, &cam, options, degrees)
    })();
    match opened {
        Ok(session) => Ok(Running {
            session,
            options: *options,
            serial: serial.to_string(),
            last_turn_check: Instant::now(),
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
