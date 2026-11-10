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
use crate::config::Options;
use crate::sys::VirtualCam;

const LOG_LINES: usize = 200;

#[derive(Default)]
struct Log {
    lines: VecDeque<String>,
    last_error: Option<String>,
    last_warning: Option<String>,
    sink_started: bool,
}

impl Log {
    fn push(&mut self, line: String) {
        if line.contains("v4l2 sink started") {
            self.sink_started = true;
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
    pub with_mic: bool,
    /// Giro con el que se abrió la cámara.
    pub degrees: u16,
}

impl Session {
    pub fn start(
        serial: &str,
        cam: &VirtualCam,
        options: &Options,
        degrees: u16,
    ) -> Result<Session, String> {
        let mut command = Command::new("scrcpy");
        command
            .arg("-s")
            .arg(serial)
            .arg("--video-source=camera")
            .arg(format!("--camera-facing={}", options.facing.scrcpy()))
            .arg(format!("--camera-size={}", options.quality.size()))
            .arg("--camera-fps=30")
            .arg(format!("--v4l2-sink={}", cam.path.display()))
            .arg(format!("--capture-orientation={degrees}"))
            // scrcpy no abre ventana: la vista previa la dibuja Ixtli.
            .arg("--no-window");
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
        Ok(Session {
            child,
            log,
            cam: cam.clone(),
            with_mic: options.mic,
            degrees,
        })
    }

    /// El video del teléfono ya está llegando a la cámara virtual.
    pub fn is_live(&self) -> bool {
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
                let raw = log
                    .last_error
                    .as_deref()
                    .or(log.last_warning.as_deref())
                    .or(log.lines.back().map(String::as_str))
                    .unwrap_or("scrcpy se cerró sin dar detalles");
                Some(Err(explain(raw)))
            }
            Err(e) => Some(Err(format!("No se pudo vigilar scrcpy: {e}"))),
        }
    }

    /// Cierra scrcpy con calma (SIGTERM) y, si no responde, a la fuerza.
    pub fn stop(mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn pump(stream: impl Read + Send + 'static, log: Arc<Mutex<Log>>) {
    thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            log.lock().unwrap().push(line);
        }
    });
}

/// Traduce los errores más comunes de scrcpy a algo que se pueda resolver.
fn explain(raw: &str) -> String {
    let lower = raw.to_lowercase();
    let known = if lower.contains("not supported before android 12") {
        "Tu teléfono necesita Android 12 o más nuevo para compartir la cámara."
    } else if lower.contains("device disconnected") || lower.contains("could not find any adb device")
    {
        "Se perdió la conexión con el teléfono. Revisa el cable USB."
    } else if lower.contains("unauthorized") {
        "Falta aceptar el aviso de depuración USB en el teléfono."
    } else if lower.contains("v4l2") || lower.contains("failed to open output") {
        "No se pudo enviar el video a la cámara virtual. ¿La está usando otro programa?"
    } else if lower.contains("camera") && (lower.contains("size") || lower.contains("no matching")) {
        "La cámara de tu teléfono no acepta esa calidad. Prueba con 720p."
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
        assert!(explain("WARN: Device disconnected").contains("cable USB"));
        assert!(explain("ERROR: Camera mirroring is not supported before Android 12")
            .contains("Android 12"));
        assert!(explain("ERROR: algo raro").contains("algo raro"));
    }
}
