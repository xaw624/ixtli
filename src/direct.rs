//! Motor propio (fase 3): Ixtli habla con el servidor de scrcpy en el
//! teléfono sin el programa scrcpy, decodifica el video con ffmpeg y entrega
//! los cuadros al puente (`relay.rs`), que los gira y los manda a «Ixtli».
//!
//! Así el giro y el espejo cambian sin reabrir la cámara del teléfono.
//!
//! Pasos de cada sesión:
//! 1. Copia al teléfono el servidor que trae el paquete de scrcpy (la misma
//!    versión que el programa scrcpy instalado, porque el servidor exige que
//!    el cliente diga su versión exacta).
//! 2. Abre un túnel (`adb forward tcp:0 localabstract:scrcpy_<scid>`).
//! 3. Arranca el servidor con `raw_stream=true`: manda solo el video H.264,
//!    sin encabezados. Con `send_dummy_byte=true` manda antes un byte, y así
//!    se sabe que la conexión es de verdad con el servidor y no solo con adb.
//! 4. ffmpeg decodifica a YUV 4:2:0 (formato yuv4mpeg, que dice el tamaño) y
//!    cada cuadro va al puente.
//!
//! El micrófono sigue con scrcpy (`Session::audio_only`).

use std::io::{BufRead, BufReader, Read};
use std::net::{Shutdown, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::config::{Capture, Options};
use crate::relay;
use crate::session::{self, Log, Session};
use crate::sys::{self, Link, VirtualCam};

/// Dónde queda el servidor en el teléfono. No es el mismo archivo que usa el
/// programa scrcpy, para no estorbarse si los dos arrancan a la vez.
const REMOTE: &str = "/data/local/tmp/ixtli-server.jar";

/// El servidor de scrcpy que trae el paquete, y su versión.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub path: PathBuf,
    pub version: String,
}

/// Busca el servidor de scrcpy en la computadora. `None` si no está o si no
/// se pudo leer la versión de scrcpy.
pub fn server() -> Option<Server> {
    let candidates = std::env::var_os("SCRCPY_SERVER_PATH")
        .map(PathBuf::from)
        .into_iter()
        .chain(["/usr/share/scrcpy/scrcpy-server", "/usr/local/share/scrcpy/scrcpy-server"].map(PathBuf::from));
    let path = candidates.into_iter().find(|path| path.is_file())?;
    let version = sys::scrcpy_version_text()?;
    Some(Server { path, version })
}

/// Lo que se le pide al servidor. `raw_stream` apaga los encabezados y el
/// byte inicial; `send_dummy_byte` (después) vuelve a pedir el byte.
fn server_args(version: &str, scid: u32, options: &Options, capture: &Capture) -> Vec<String> {
    let mut args = vec![
        version.to_string(),
        format!("scid={scid:08x}"),
        "log_level=info".into(),
        "video_source=camera".into(),
        "audio=false".into(),
        "control=false".into(),
        "tunnel_forward=true".into(),
        "cleanup=false".into(),
        "raw_stream=true".into(),
        "send_dummy_byte=true".into(),
        format!("camera_size={}", capture.size),
        format!("camera_fps={}", capture.fps),
    ];
    // El servidor, como scrcpy, no deja pedir el lente y el lado a la vez.
    args.push(match &capture.camera_id {
        Some(id) => format!("camera_id={id}"),
        None => format!("camera_facing={}", options.facing.scrcpy()),
    });
    if let Some(rate) = capture.bit_rate.and_then(bit_rate) {
        args.push(format!("video_bit_rate={rate}"));
    }
    let angle = server_angle(options);
    if angle != 0 {
        args.push(format!("angle={angle}"));
    }
    args
}

/// El ángulo libre lo gira el teléfono (con su GPU, como con scrcpy), y no
/// Ixtli: girar a cualquier ángulo cada cuadro en la computadora costaría
/// más que todo lo demás junto. El servidor lo aplica antes que el giro y el
/// espejo del puente; como los giros se pueden hacer en cualquier orden, solo
/// el espejo cambia el resultado: con espejo se pide al revés, para que la
/// imagen quede igual que con scrcpy (que gira al final). En grados, de 0 a
/// 359, sentido del reloj.
pub fn server_angle(options: &Options) -> u16 {
    let angle = if options.mirror { -options.angle } else { options.angle };
    angle.rem_euclid(360) as u16
}

/// «2M» → 2000000, como lo entiende scrcpy.
fn bit_rate(text: &str) -> Option<u32> {
    let text = text.trim();
    let (number, factor) = match text.chars().last()? {
        'M' | 'm' => (&text[..text.len() - 1], 1_000_000),
        'K' | 'k' => (&text[..text.len() - 1], 1_000),
        _ => (text, 1),
    };
    number.parse::<u32>().ok()?.checked_mul(factor)
}

/// Lo que comparten los hilos de la sesión.
#[derive(Default)]
struct State {
    /// Ya llegó el primer cuadro decodificado.
    live: AtomicBool,
    /// El video dejó de llegar (se cerró la conexión o ffmpeg).
    ended: AtomicBool,
}

/// La transmisión de video: el servidor en el teléfono, el túnel y ffmpeg.
/// Se puede cerrar y volver a abrir sin tocar el micrófono (pausa automática).
struct Stream {
    server: Child,
    decoder: Child,
    socket: TcpStream,
    port: u16,
    log: Arc<Mutex<Log>>,
    state: Arc<State>,
}

impl Stream {
    /// Con `push`, primero copia el servidor al teléfono. Al reanudar una
    /// pausa ya está ahí (`cleanup=false` hace que no se borre al cerrar).
    fn open(serial: &str, link: Link, options: &Options, capture: &Capture, push: bool) -> Result<Stream, String> {
        let server = server().ok_or("No encontré el servidor de scrcpy (scrcpy-server) en esta computadora.")?;
        if push {
            sys::run_for(
                "adb",
                &["-s", serial, "push", &server.path.to_string_lossy(), REMOTE],
                Duration::from_secs(20),
            )
            .map_err(|_| "No se pudo copiar el servidor de scrcpy al teléfono.".to_string())?;
        }
        let scid = new_scid();
        let socket_name = format!("localabstract:scrcpy_{scid:08x}");
        let port = sys::run_for("adb", &["-s", serial, "forward", "tcp:0", &socket_name], Duration::from_secs(5))
            .ok()
            .and_then(|reply| reply.trim().parse::<u16>().ok())
            .ok_or("No se pudo abrir el túnel hacia el teléfono (adb forward).")?;

        let log = Arc::new(Mutex::new(Log::default()));
        let mut command = Command::new("adb");
        command
            .args(["-s", serial, "shell"])
            .arg(format!("CLASSPATH={REMOTE}"))
            .args(["app_process", "/", "com.genymobile.scrcpy.Server"])
            .args(server_args(&server.version, scid, options, capture))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        die_with_parent(&mut command);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                remove_forward(serial, port);
                return Err(format!("No se pudo arrancar el servidor en el teléfono: {e}"));
            }
        };
        if let Some(stdout) = child.stdout.take() {
            session::pump(stdout, log.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            session::pump(stderr, log.clone());
        }

        let socket = match connect(port, &mut child) {
            Ok(socket) => socket,
            Err(()) => {
                session::stop_child(&mut child);
                remove_forward(serial, port);
                thread::sleep(Duration::from_millis(150));
                return Err(failure(&log.lock().unwrap(), link, options.saver)
                    .unwrap_or_else(|| "El servidor de scrcpy no respondió en el teléfono.".to_string()));
            }
        };

        let mut decoder = match spawn_decoder(&log) {
            Ok(decoder) => decoder,
            Err(problem) => {
                let _ = socket.shutdown(Shutdown::Both);
                session::stop_child(&mut child);
                remove_forward(serial, port);
                return Err(problem);
            }
        };
        let state = Arc::new(State::default());
        // Del teléfono a ffmpeg.
        if let (Some(mut input), Ok(mut from_phone)) = (decoder.stdin.take(), socket.try_clone()) {
            let state = state.clone();
            thread::spawn(move || {
                let _ = std::io::copy(&mut from_phone, &mut input);
                state.ended.store(true, Ordering::Relaxed);
            });
        }
        // De ffmpeg al puente.
        if let Some(output) = decoder.stdout.take() {
            // Un cuadro mide de 1.4 a 3 MB: con el pipe de 64 KB de siempre
            // hacen falta decenas de lecturas por cuadro.
            // SAFETY: el descriptor es válido mientras exista `output`.
            unsafe {
                libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&output), libc::F_SETPIPE_SZ, 1 << 20);
            }
            let state = state.clone();
            thread::spawn(move || {
                let _ = read_frames(output, &state, relay::deliver);
                state.ended.store(true, Ordering::Relaxed);
            });
        }
        Ok(Stream { server: child, decoder, socket, port, log, state })
    }

    /// Si el video se cortó, el motivo.
    fn finished(&mut self, link: Link, saver: bool) -> Option<String> {
        let server_done = !matches!(self.server.try_wait(), Ok(None));
        let decoder_done = !matches!(self.decoder.try_wait(), Ok(None));
        if !(server_done || decoder_done || self.state.ended.load(Ordering::Relaxed)) {
            return None;
        }
        // Deja que los hilos lectores alcancen las últimas líneas.
        thread::sleep(Duration::from_millis(150));
        let log = self.log.lock().unwrap();
        Some(failure(&log, link, saver).unwrap_or_else(|| session::explain("WARN: Device disconnected", link, saver)))
    }

    /// Cierra la conexión (con eso el servidor termina en el teléfono y apaga
    /// la cámara), adb, ffmpeg y el túnel.
    fn stop(mut self, serial: &str) {
        let _ = self.socket.shutdown(Shutdown::Both);
        session::stop_child(&mut self.server);
        session::stop_child(&mut self.decoder);
        remove_forward(serial, self.port);
    }
}

fn remove_forward(serial: &str, port: u16) {
    let _ = sys::run_for("adb", &["-s", serial, "forward", "--remove", &format!("tcp:{port}")], Duration::from_secs(3));
}

pub struct Direct {
    /// `None` mientras la cámara está en pausa.
    stream: Option<Stream>,
    serial: String,
    /// El scrcpy del micrófono, si se pidió. Sigue durante la pausa.
    audio: Option<Session>,
    link: Link,
    saver: bool,
    pub with_mic: bool,
}

impl Direct {
    pub fn start(
        serial: &str,
        link: Link,
        cam: &VirtualCam,
        options: &Options,
        capture: &Capture,
    ) -> Result<Direct, String> {
        let stream = Stream::open(serial, link, options, capture, true)?;
        let audio = if options.mic {
            match Session::audio_only(serial, link, cam) {
                Ok(audio) => Some(audio),
                Err(problem) => {
                    stream.stop(serial);
                    return Err(problem);
                }
            }
        } else {
            None
        };
        Ok(Direct {
            stream: Some(stream),
            serial: serial.to_string(),
            audio,
            link,
            saver: options.saver,
            with_mic: options.mic,
        })
    }

    /// Ya llegó video del teléfono al puente.
    pub fn is_live(&self) -> bool {
        self.stream.as_ref().is_some_and(|stream| stream.state.live.load(Ordering::Relaxed))
    }

    /// La cámara del teléfono está apagada por la pausa automática.
    pub fn is_paused(&self) -> bool {
        self.stream.is_none()
    }

    /// Apaga la cámara del teléfono. El micrófono sigue.
    pub fn pause(&mut self) {
        if let Some(stream) = self.stream.take() {
            stream.stop(&self.serial);
        }
    }

    /// Vuelve a arrancar el video con otras opciones (otro ángulo), sin tocar
    /// el micrófono.
    pub fn restart(&mut self, options: &Options, capture: &Capture) -> Result<(), String> {
        self.pause();
        self.resume(options, capture)
    }

    /// Vuelve a prender la cámara del teléfono después de una pausa.
    pub fn resume(&mut self, options: &Options, capture: &Capture) -> Result<(), String> {
        if self.stream.is_none() {
            self.stream = Some(Stream::open(&self.serial, self.link, options, capture, false)?);
        }
        Ok(())
    }

    /// Si el video (o el micrófono) se cortó, el motivo.
    pub fn finished(&mut self) -> Option<Result<(), String>> {
        if let Some(problem) = self.stream.as_mut().and_then(|stream| stream.finished(self.link, self.saver)) {
            return Some(Err(problem));
        }
        if let Some(audio) = &mut self.audio {
            if let Some(outcome) = audio.finished() {
                return Some(outcome.and(Err("Se cerró el micrófono del teléfono.".to_string())));
            }
        }
        None
    }

    /// Cierra todo: el video y el micrófono.
    pub fn stop(mut self) {
        self.pause();
        if let Some(audio) = self.audio {
            audio.stop();
        }
    }
}

/// Un número de sesión para el nombre del túnel (`scrcpy_<scid>`), distinto
/// en cada sesión y entre dos Ixtli. El servidor solo acepta 31 bits.
fn new_scid() -> u32 {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|at| at.subsec_nanos() ^ at.as_secs() as u32)
        .unwrap_or(0);
    (nanos ^ std::process::id().rotate_left(16)) & 0x7fff_ffff
}

/// Se conecta al túnel hasta que el servidor responde con su byte inicial,
/// como mucho 10 segundos. Mientras el servidor no escucha, adb acepta la
/// conexión y la cierra enseguida.
fn connect(port: u16, server: &mut Child) -> Result<TcpStream, ()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !matches!(server.try_wait(), Ok(None)) {
            return Err(());
        }
        if let Ok(mut socket) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
            let mut byte = [0u8; 1];
            if matches!(socket.read(&mut byte), Ok(1)) {
                let _ = socket.set_read_timeout(None);
                let _ = socket.set_nodelay(true);
                return Ok(socket);
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(())
}

/// ffmpeg lee H.264 por su entrada y escribe yuv4mpeg por su salida. Con un
/// solo hilo de decodificación no junta cuadros (cada hilo más los retrasa).
fn spawn_decoder(log: &Arc<Mutex<Log>>) -> Result<Child, String> {
    let mut command = Command::new("ffmpeg");
    command
        .args(["-hide_banner", "-loglevel", "error"])
        .args(["-fflags", "nobuffer", "-flags", "low_delay", "-probesize", "32", "-analyzeduration", "0"])
        .args(["-threads", "1", "-f", "h264", "-i", "pipe:0"])
        .args(["-map", "0:v:0", "-fps_mode", "passthrough", "-pix_fmt", "yuv420p", "-f", "yuv4mpegpipe", "pipe:1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    die_with_parent(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("No se pudo abrir ffmpeg para decodificar el video: {e}"))?;
    if let Some(stderr) = child.stderr.take() {
        let log = log.clone();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                log.lock().unwrap().push(format!("ERROR: ffmpeg: {line}"));
            }
        });
    }
    Ok(child)
}

/// Lee los cuadros de ffmpeg y los entrega (al puente, `relay::deliver`).
fn read_frames(
    output: impl Read,
    state: &State,
    mut deliver: impl FnMut(usize, usize, &mut Vec<u8>),
) -> std::io::Result<()> {
    let mut reader = BufReader::with_capacity(1 << 20, output);
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line)?;
    let (width, height) = y4m_size(&line).ok_or(std::io::ErrorKind::InvalidData)?;
    let size = width * height + 2 * (width.div_ceil(2) * height.div_ceil(2));
    let mut frame = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        if !line.starts_with(b"FRAME") {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        frame.resize(size, 0);
        reader.read_exact(&mut frame)?;
        deliver(width, height, &mut frame);
        state.live.store(true, Ordering::Relaxed);
    }
}

/// Del encabezado `YUV4MPEG2 W1280 H720 F30:1 ...` saca el ancho y el alto.
fn y4m_size(header: &[u8]) -> Option<(usize, usize)> {
    let text = std::str::from_utf8(header).ok()?;
    let mut fields = text.split_whitespace();
    if fields.next()? != "YUV4MPEG2" {
        return None;
    }
    let (mut width, mut height) = (None, None);
    for field in fields {
        if let Some(value) = field.strip_prefix('W') {
            width = value.parse().ok();
        } else if let Some(value) = field.strip_prefix('H') {
            height = value.parse().ok();
        }
    }
    Some((width?, height?)).filter(|&(w, h): &(usize, usize)| w > 0 && h > 0)
}

/// El motivo más claro que dejó el servidor (o ffmpeg) en el registro.
fn failure(log: &Log, link: Link, saver: bool) -> Option<String> {
    let raw = log
        .lines
        .iter()
        .rev()
        .find(|line| session::camera_rejected(line))
        .or(log.last_error.as_ref())
        .or(log.last_warning.as_ref())?;
    Some(session::explain(raw, link, saver))
}

/// Si Ixtli muere de golpe, el kernel cierra también este proceso.
fn die_with_parent(command: &mut Command) {
    // SAFETY: `prctl` es seguro de llamar después de `fork`.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Facing;

    #[test]
    fn reads_y4m_header() {
        assert_eq!(y4m_size(b"YUV4MPEG2 W1280 H720 F30:1 Ip A1:1 C420jpeg XYSCSS=420JPEG\n"), Some((1280, 720)));
        assert_eq!(y4m_size(b"YUV4MPEG2 H1920 W1080\n"), Some((1080, 1920)));
        assert_eq!(y4m_size(b"FRAME\n"), None);
        assert_eq!(y4m_size(b"YUV4MPEG2 W0 H720\n"), None);
    }

    #[test]
    fn bit_rates() {
        assert_eq!(bit_rate("2M"), Some(2_000_000));
        assert_eq!(bit_rate("800K"), Some(800_000));
        assert_eq!(bit_rate("8000000"), Some(8_000_000));
        assert_eq!(bit_rate("x"), None);
    }

    #[test]
    fn asks_the_server_for_a_raw_stream() {
        let options = Options { facing: Facing::Back, ..Options::default() };
        let capture = Capture { camera_id: None, size: "1280x720".into(), fps: 30, bit_rate: Some("2M") };
        let args = server_args("3.3.4", 0x1234, &options, &capture);
        assert_eq!(args[0], "3.3.4");
        assert!(args.contains(&"scid=00001234".to_string()));
        assert!(args.contains(&"camera_facing=back".to_string()));
        assert!(args.contains(&"video_bit_rate=2000000".to_string()));
        // El byte inicial se vuelve a pedir después de `raw_stream`.
        let raw = args.iter().position(|a| a == "raw_stream=true").unwrap();
        let dummy = args.iter().position(|a| a == "send_dummy_byte=true").unwrap();
        assert!(raw < dummy);
        let lens = Capture { camera_id: Some("2".into()), ..capture };
        let args = server_args("3.3.4", 1, &options, &lens);
        assert!(args.contains(&"camera_id=2".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("camera_facing")));
    }

    #[test]
    fn mirror_reverses_the_angle() {
        let tilted = Options { angle: 15, ..Options::default() };
        assert_eq!(server_angle(&tilted), 15);
        assert_eq!(server_angle(&Options { mirror: true, ..tilted }), 345);
        assert_eq!(server_angle(&Options { angle: -180, ..tilted }), 180);
        assert_eq!(server_angle(&Options::default()), 0);
        let capture = Capture { camera_id: None, size: "1280x720".into(), fps: 30, bit_rate: None };
        assert!(server_args("3.3.4", 1, &tilted, &capture).contains(&"angle=15".to_string()));
        assert!(!server_args("3.3.4", 1, &Options::default(), &capture).iter().any(|a| a.starts_with("angle")));
    }

    #[test]
    fn scid_fits_in_31_bits() {
        assert!(new_scid() <= 0x7fff_ffff);
    }

    #[test]
    fn reads_frames_from_ffmpeg() {
        let mut stream = b"YUV4MPEG2 W4 H2 F30:1 C420jpeg\n".to_vec();
        for value in [10u8, 20] {
            stream.extend_from_slice(b"FRAME\n");
            stream.extend(std::iter::repeat_n(value, 4 * 2 + 2 * 2));
        }
        let state = State::default();
        let mut seen = Vec::new();
        read_frames(&stream[..], &state, |w, h, frame| seen.push((w, h, frame[0], frame.len()))).unwrap();
        assert!(state.live.load(Ordering::Relaxed));
        assert_eq!(seen, [(4, 2, 10, 12), (4, 2, 20, 12)]);
    }
}
