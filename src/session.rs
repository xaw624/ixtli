//! Una sesión de scrcpy: el proceso que trae la cámara (y el micrófono)
//! del teléfono mientras Ixtli está encendido.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::audio;
use crate::config::{Capture, Options};
use crate::sys::{self, Link, VirtualCam};

const LOG_LINES: usize = 200;

/// Aviso cuando el teléfono deja de responder por la red.
pub const LOST_WIFI: &str =
    "Se perdió la conexión WiFi con el teléfono. Revisa que el teléfono y esta computadora sigan en la misma red.";

#[derive(Default)]
pub(crate) struct Log {
    pub(crate) lines: VecDeque<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_warning: Option<String>,
    sink_started: bool,
    /// El servidor de scrcpy ya arrancó en el teléfono.
    pub(crate) server_started: bool,
}

impl Log {
    pub(crate) fn push(&mut self, line: String) {
        if line.contains("v4l2 sink started") {
            self.sink_started = true;
        }
        if line.contains("[server]") {
            self.server_started = true;
        }
        if line.contains("ERROR") {
            self.last_error = Some(line.clone());
        } else if line.contains("WARN") {
            self.last_warning = Some(line.clone());
        }
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

pub struct Session {
    child: Child,
    log: Arc<Mutex<Log>>,
    cam: VirtualCam,
    link: Link,
    saver: bool,
    /// Sin video: solo trae el micrófono (con el motor propio).
    audio_only: bool,
    pub with_mic: bool,
}

impl Session {
    pub fn start(
        serial: &str,
        link: Link,
        cam: &VirtualCam,
        options: &Options,
        capture: &Capture,
        degrees: u16,
    ) -> Result<Session, String> {
        let mut command = Command::new("scrcpy");
        command
            .arg("-s")
            .arg(serial)
            .arg("--video-source=camera")
            // scrcpy no deja pedir el lente y el lado a la vez.
            .arg(match &capture.camera_id {
                Some(id) => format!("--camera-id={id}"),
                None => format!("--camera-facing={}", options.facing.scrcpy()),
            })
            .arg(format!("--camera-size={}", capture.size))
            .arg(format!("--camera-fps={}", capture.fps))
            .arg(format!("--v4l2-sink={}", cam.path.display()))
            .arg(format!(
                "--capture-orientation={}",
                options.capture_orientation(degrees)
            ))
            // scrcpy no abre ventana: la vista previa la dibuja Ixtli.
            .arg("--no-window");
        if let Some(rate) = capture.bit_rate {
            command.arg(format!("--video-bit-rate={rate}"));
        }
        if options.angle != 0 {
            command.arg(format!("--angle={}", options.angle.rem_euclid(360)));
        }
        if options.mic {
            // El audio del teléfono se "reproduce" en la salida interna de
            // Ixtli; de ahí lo toma el micrófono virtual.
            command
                .arg("--audio-source=mic")
                .env("SDL_AUDIODRIVER", "pulseaudio")
                .env("PULSE_SINK", audio::SINK);
        } else {
            command.arg("--no-audio");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Si Ixtli muere de golpe, el kernel le avisa a scrcpy para que
        // también termine y no se quede usando la cámara.
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let turn = sys::scrcpy_turn();
        let mut child = command
            .spawn()
            .map_err(|e| format!("No se pudo abrir scrcpy: {e}"))?;

        let log = Arc::new(Mutex::new(Log::default()));
        if let Some(stdout) = child.stdout.take() {
            pump(stdout, log.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, log.clone());
        }
        wait_for_server(&mut child, &log);
        drop(turn);
        Ok(Session {
            child,
            log,
            cam: cam.clone(),
            link,
            saver: options.saver,
            audio_only: false,
            with_mic: options.mic,
        })
    }

    /// Un scrcpy sin video que solo trae el micrófono del teléfono, para el
    /// motor propio (que trae el video por su cuenta).
    pub fn audio_only(serial: &str, link: Link, cam: &VirtualCam) -> Result<Session, String> {
        let mut command = Command::new("scrcpy");
        command
            .arg("-s")
            .arg(serial)
            .args(["--no-video", "--no-control", "--no-window", "--audio-source=mic"])
            .env("SDL_AUDIODRIVER", "pulseaudio")
            .env("PULSE_SINK", audio::SINK)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let turn = sys::scrcpy_turn();
        let mut child = command
            .spawn()
            .map_err(|e| format!("No se pudo abrir scrcpy para el micrófono: {e}"))?;
        let log = Arc::new(Mutex::new(Log::default()));
        if let Some(stdout) = child.stdout.take() {
            pump(stdout, log.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, log.clone());
        }
        wait_for_server(&mut child, &log);
        drop(turn);
        Ok(Session {
            child,
            log,
            cam: cam.clone(),
            link,
            saver: false,
            audio_only: true,
            with_mic: true,
        })
    }

    /// El video del teléfono ya está llegando a la cámara virtual.
    pub fn is_live(&self) -> bool {
        if self.audio_only {
            return self.log.lock().unwrap().server_started;
        }
        self.cam.streaming() || self.log.lock().unwrap().sink_started
    }

    /// Si scrcpy ya terminó, devuelve cómo: `Ok` si se cerró de forma normal
    /// (por ejemplo, al cerrar la vista previa) o el motivo del error.
    pub fn finished(&mut self) -> Option<Result<(), String>> {
        match self.child.try_wait() {
            Ok(None) => None,
            Ok(Some(status)) if status.success() => Some(Ok(())),
            Ok(Some(_)) => {
                // Deja que los hilos lectores alcancen las últimas líneas.
                thread::sleep(Duration::from_millis(150));
                let log = self.log.lock().unwrap();
                // El rechazo de la cámara no es el último error que escribe
                // scrcpy («Video encoding error» llega después).
                let raw = log
                    .lines
                    .iter()
                    .rev()
                    .find(|line| camera_rejected(line))
                    .map(String::as_str)
                    .or(log.last_error.as_deref())
                    .or(log.last_warning.as_deref())
                    .or(log.lines.back().map(String::as_str))
                    .unwrap_or("scrcpy se cerró sin dar detalles");
                Some(Err(explain(raw, self.link, self.saver)))
            }
            Err(e) => Some(Err(format!("No se pudo vigilar scrcpy: {e}"))),
        }
    }

    /// Cierra scrcpy con calma (SIGTERM) y, si no responde, a la fuerza.
    pub fn stop(mut self) {
        stop_child(&mut self.child);
    }
}

/// Mientras la cámara está encendida, el teléfono no se duerme solo: scrcpy
/// cambia el tiempo de espera de la pantalla a un día y lo regresa al cerrar.
const AWAKE_SECONDS: u32 = 24 * 60 * 60;
/// Puertos de la computadora para el scrcpy de la pantalla. Si usara los de
/// siempre (desde 27183) y arrancara junto con la cámara, a veces uno de los
/// dos se conectaba con el otro y fallaba con «Server connection failed».
const SCREEN_PORTS: &str = "27300:27399";

/// Un segundo scrcpy, sin video ni audio, que solo apaga la pantalla del
/// teléfono. Hace falta aparte porque con la cámara como fuente scrcpy no
/// controla el teléfono. Apaga solo el panel: el teléfono sigue despierto y
/// sin bloquearse, y al cerrar este scrcpy la pantalla se vuelve a prender.
pub struct ScreenOff {
    child: Child,
}

impl ScreenOff {
    pub fn start(serial: &str) -> Result<ScreenOff, String> {
        let mut command = Command::new("scrcpy");
        command
            .arg("-s")
            .arg(serial)
            .args(["--no-video", "--no-audio", "--no-window", "--turn-screen-off"])
            .arg(format!("--screen-off-timeout={AWAKE_SECONDS}"))
            .arg(format!("--port={SCREEN_PORTS}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
        let turn = sys::scrcpy_turn();
        let mut child = command
            .spawn()
            .map_err(|e| format!("No se pudo apagar la pantalla: {e}"))?;
        let log = Arc::new(Mutex::new(Log::default()));
        if let Some(stdout) = child.stdout.take() {
            pump(stdout, log.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            pump(stderr, log.clone());
        }
        wait_for_server(&mut child, &log);
        drop(turn);
        Ok(ScreenOff { child })
    }

    /// Sigue abierto (y la pantalla, apagada).
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Lo cierra, y con eso la pantalla se vuelve a prender.
    pub fn stop(mut self) {
        stop_child(&mut self.child);
    }
}

/// Espera a que el servidor de scrcpy arranque en el teléfono (o a que scrcpy
/// termine), como mucho 10 segundos. Mientras tanto ningún otro scrcpy debe
/// arrancar (ver `sys::scrcpy_turn`).
fn wait_for_server(child: &mut Child, log: &Mutex<Log>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if log.lock().unwrap().server_started || !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Cierra un proceso con calma (SIGTERM) y, si no responde, a la fuerza.
pub(crate) fn stop_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn pump(stream: impl Read + Send + 'static, log: Arc<Mutex<Log>>) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            log.lock().unwrap().push(line);
        }
    });
}

/// La cámara del teléfono no aceptó el tamaño o los cuadros pedidos.
pub(crate) fn camera_rejected(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("camera device is currently in the error state") || lower.contains("not supported by hal")
}

/// Traduce los errores más comunes de scrcpy a algo que se pueda resolver.
pub(crate) fn explain(raw: &str, link: Link, saver: bool) -> String {
    let lower = raw.to_lowercase();
    // La cámara rechazó el tamaño al abrirse. scrcpy también dice «Device
    // disconnected», así que esto va antes que la conexión perdida.
    let rejected = camera_rejected(raw);
    let known = if lower.contains("not supported before android 12") {
        "Tu teléfono necesita Android 12 o más nuevo para compartir la cámara."
    } else if rejected || (lower.contains("camera") && (lower.contains("size") || lower.contains("no matching") || lower.contains("fps"))) {
        if saver {
            "La cámara de tu teléfono no acepta el modo ahorro. Apágalo e intenta otra vez."
        } else {
            "La cámara de tu teléfono no acepta esa calidad. Prueba con 720p."
        }
    } else if lower.contains("device disconnected") || lower.contains("could not find any adb device")
    {
        match link {
            Link::Usb => "Se perdió la conexión con el teléfono. Revisa el cable USB.",
            Link::Wifi => LOST_WIFI,
        }
    } else if lower.contains("unauthorized") {
        "Falta aceptar el aviso de depuración USB en el teléfono."
    } else if lower.contains("v4l2") || lower.contains("failed to open output") {
        "No se pudo enviar el video a la cámara virtual. ¿La está usando otro programa?"
    } else {
        return format!("scrcpy se cerró con un error: {}", raw.trim());
    };
    known.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_tracks_state() {
        let mut log = Log::default();
        log.push("INFO: Renderer: opengl".into());
        assert!(!log.sink_started);
        log.push("INFO: v4l2 sink started to device: /dev/video10".into());
        log.push("WARN: Device disconnected".into());
        assert!(log.sink_started);
        assert_eq!(log.last_warning.as_deref(), Some("WARN: Device disconnected"));
        assert!(log.last_error.is_none());
    }

    #[test]
    fn log_is_bounded() {
        let mut log = Log::default();
        for n in 0..(LOG_LINES + 50) {
            log.push(format!("línea {n}"));
        }
        assert_eq!(log.lines.len(), LOG_LINES);
    }

    #[test]
    fn explains_known_errors() {
        assert!(explain("WARN: Device disconnected", Link::Usb, false).contains("cable USB"));
        assert!(explain("WARN: Device disconnected", Link::Wifi, false).contains("WiFi"));
        assert!(explain("ERROR: Camera mirroring is not supported before Android 12", Link::Usb, false)
            .contains("Android 12"));
        assert!(explain("ERROR: algo raro", Link::Usb, false).contains("algo raro"));
        let size = "ERROR: Could not select camera size";
        assert!(explain(size, Link::Usb, false).contains("720p"));
        assert!(explain(size, Link::Usb, true).contains("modo ahorro"));
        // Lo que dice scrcpy cuando la cámara del moto g34 rechaza 960x540.
        let rejected = "[server] ERROR: Capture/encoding error: java.io.IOException: android.hardware.camera2.CameraAccessException: The camera device is currently in the error state; no further calls to it will succeed.\nWARN: Device disconnected";
        assert!(explain(rejected, Link::Usb, true).contains("modo ahorro"));
        assert!(explain(rejected, Link::Usb, false).contains("720p"));
    }
}
