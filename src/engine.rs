//! El motor: un hilo que vigila el sistema y maneja la sesión de scrcpy,
//! para que ni la ventana ni la terminal se queden esperando.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio;
use crate::config::Options;
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
    let mut session: Option<Session> = None;
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
            Some(running) => match running.finished() {
                Some(outcome) => {
                    end(&mut session);
                    update(&shared, |state| {
                        state.phase = Phase::Off;
                        state.mic = false;
                        state.error = outcome.err();
                    });
                }
                None if running.is_live() => update(&shared, |state| state.phase = Phase::Live),
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
                end(&mut session);
                match begin(&options) {
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
                end(&mut session);
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
                end(&mut session);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Deja todo listo y abre scrcpy. Cada paso que falla explica qué hacer.
fn begin(options: &Options) -> Result<Session, String> {
    let deps = Deps::check();
    if let Some(problem) = deps.problem(options.mic) {
        return Err(problem);
    }
    let serial = match sys::phone() {
        Phone::Ready { serial, .. } => serial,
        other => return Err(other.advice().to_string()),
    };
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
    if options.mic {
        audio::create()?;
    }
    Session::start(&serial, &cam, options).inspect_err(|_| {
        if options.mic {
            audio::remove();
        }
    })
}

/// Cierra la sesión (si hay) y quita el micrófono virtual que usaba.
fn end(session: &mut Option<Session>) {
    if let Some(running) = session.take() {
        let with_mic = running.with_mic;
        running.stop();
        if with_mic {
            audio::remove();
        }
    }
}
