//! Salida de tamaño fijo: scrcpy manda el video a una cámara virtual interna y
//! este puente lo copia a la cámara «Ixtli», siempre del mismo tamaño.
//!
//! Así las videollamadas nunca ven un cambio: al girar, al cambiar de cámara o
//! de modo, o con la cámara apagada (manda negro), «Ixtli» sigue ahí con el
//! mismo tamaño. Antes, cada cambio de tamaño hacía que el navegador perdiera la
//! cámara, y la cámara virtual no deja cambiar el tamaño mientras alguien la
//! tiene abierta.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::preview::{self, Frame};
use crate::sys::VirtualCam;

/// Sin video del teléfono durante este tiempo, se manda negro.
const BLACK_AFTER: Duration = Duration::from_millis(400);
/// Cada cuánto se manda el negro (unos 10 cuadros por segundo).
const BLACK_EVERY: Duration = Duration::from_millis(100);
/// Unas 10 imágenes por segundo bastan para la vista previa: es para ver el
/// encuadre, y cada imagen se convierte a color y se vuelve a dibujar.
pub const PREVIEW_EVERY: Duration = Duration::from_millis(100);

// La vista previa ya no lee la cámara «Ixtli»: v4l2loopback solo deja leer a
// un programa a la vez, y si la tomaba Ixtli, la videollamada se quedaba sin
// cámara. El puente le pasa las imágenes directo (en este proceso) o por un
// archivo en memoria (a la ventana suelta, que es otro proceso).

/// Este proceso tiene un puente funcionando.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// La vista previa de este proceso quiere imágenes.
static WANTED: AtomicBool = AtomicBool::new(false);
static LATEST: Mutex<Option<Arc<Frame>>> = Mutex::new(None);
/// Cuándo llegó el último cuadro del teléfono al puente de este proceso.
static LAST_REAL: Mutex<Option<Instant>> = Mutex::new(None);

/// Cuadros que entrega el motor propio (`direct.rs`) sin pasar por la cámara
/// interna. El búfer se intercambia, no se copia: el decodificador y el
/// puente se pasan siempre los mismos dos o tres búferes.
struct Slot {
    fresh: bool,
    width: usize,
    height: usize,
    data: Vec<u8>,
}

static DIRECT: Mutex<Slot> = Mutex::new(Slot { fresh: false, width: 0, height: 0, data: Vec::new() });
static DIRECT_READY: Condvar = Condvar::new();

/// Entrega un cuadro YU12 de `width`x`height` del motor propio. `frame` se
/// queda con un búfer viejo para el siguiente cuadro.
pub fn deliver(width: usize, height: usize, frame: &mut Vec<u8>) {
    let mut slot = DIRECT.lock().unwrap();
    std::mem::swap(&mut slot.data, frame);
    slot.width = width;
    slot.height = height;
    slot.fresh = true;
    DIRECT_READY.notify_one();
}

/// Espera un cuadro del motor propio como mucho `wait`. Si llega, queda en
/// `into` y devuelve su tamaño.
fn take_direct(wait: Duration, into: &mut Vec<u8>) -> Option<(usize, usize)> {
    let slot = DIRECT.lock().unwrap();
    let (mut slot, _) = DIRECT_READY.wait_timeout_while(slot, wait, |slot| !slot.fresh).unwrap();
    if !slot.fresh {
        return None;
    }
    slot.fresh = false;
    std::mem::swap(&mut slot.data, into);
    Some((slot.width, slot.height))
}

/// Hay un puente en este proceso: la vista previa sale de aquí.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// La vista previa de este proceso quiere (o ya no) imágenes.
pub fn want_preview(wanted: bool) {
    WANTED.store(wanted, Ordering::Relaxed);
}

/// El puente de este proceso recibió video del teléfono hace menos de
/// `within`. Mientras llegan cuadros, la conexión con el teléfono está bien y
/// no hace falta preguntarle nada.
pub fn receiving(within: Duration) -> bool {
    active() && LAST_REAL.lock().unwrap().is_some_and(|at| at.elapsed() < within)
}

/// Alguien está viendo la vista previa: la de esta ventana o la ventana suelta.
pub fn preview_watched() -> bool {
    WANTED.load(Ordering::Relaxed) || shared_wanted()
}

/// La imagen más reciente para la vista previa de este proceso.
pub fn latest() -> Option<Arc<Frame>> {
    LATEST.lock().unwrap().clone()
}

/// Carpeta en memoria donde el puente deja la imagen para la ventana suelta.
fn shared_dir() -> PathBuf {
    crate::sys::runtime_dir()
}

/// La ventana suelta avisa que quiere imágenes tocando este archivo.
fn request_path() -> PathBuf {
    shared_dir().join("vista-quiero")
}

fn frame_path() -> PathBuf {
    shared_dir().join("vista.rgba")
}

/// Para la ventana suelta: pide imágenes y lee la más reciente que dejó otro
/// proceso de Ixtli. `None` si no hay una reciente.
pub fn shared_frame() -> Option<Frame> {
    let _ = std::fs::create_dir_all(shared_dir());
    let _ = std::fs::write(request_path(), b"");
    let path = frame_path();
    let fresh = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < Duration::from_secs(1));
    if !fresh {
        return None;
    }
    decode_shared(std::fs::read(path).ok()?)
}

/// La ventana suelta pidió imágenes hace poco.
fn shared_wanted() -> bool {
    std::fs::metadata(request_path())
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|at| SystemTime::now().duration_since(at).ok())
        .is_some_and(|age| age < Duration::from_secs(3))
}

/// Encabezado (ancho, alto y número, en little endian) y luego RGBA.
fn encode_shared(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 + frame.rgba.len());
    bytes.extend_from_slice(&frame.width.to_le_bytes());
    bytes.extend_from_slice(&frame.height.to_le_bytes());
    bytes.extend_from_slice(&frame.number.to_le_bytes());
    bytes.extend_from_slice(&frame.rgba);
    bytes
}

fn decode_shared(bytes: Vec<u8>) -> Option<Frame> {
    let width = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    let height = u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?);
    let number = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    // La imagen se queda en el mismo búfer que se leyó, sin copiarla.
    (bytes.len() == 16 + width as usize * height as usize * 4)
        .then(|| Frame { width, height, rgba: bytes::Bytes::from(bytes).slice(16..), number })
}

/// Pasa el cuadro a la vista previa, si alguien la quiere.
struct PreviewFeed {
    number: u64,
    last: Option<Instant>,
    /// Última vez que se revisó si la ventana suelta quiere imágenes.
    checked: Option<Instant>,
    shared: bool,
}

impl PreviewFeed {
    fn new() -> PreviewFeed {
        PreviewFeed { number: 0, last: None, checked: None, shared: false }
    }

    fn offer(&mut self, frame: &[u8], width: usize, height: usize) {
        if self.checked.is_none_or(|at| at.elapsed() >= Duration::from_secs(1)) {
            self.shared = shared_wanted();
            self.checked = Some(Instant::now());
        }
        let here = WANTED.load(Ordering::Relaxed);
        if !(here || self.shared) || self.last.is_some_and(|at| at.elapsed() < PREVIEW_EVERY) {
            return;
        }
        self.last = Some(Instant::now());
        self.number += 1;
        let picture = preview::convert(frame, width, height, self.number);
        if self.shared {
            let temporary = shared_dir().join("vista.rgba.tmp");
            if std::fs::write(&temporary, encode_shared(&picture)).is_ok() {
                let _ = std::fs::rename(&temporary, frame_path());
            }
        }
        *LATEST.lock().unwrap() = here.then(|| Arc::new(picture));
    }
}

struct Shared {
    /// Tamaño que se quiere en la cámara «Ixtli». `None`: soltarla.
    size: Mutex<Option<(usize, usize)>>,
    /// Giro (en grados, sentido del reloj) y espejo para los cuadros del
    /// motor propio. Los de scrcpy ya llegan girados.
    turn: Mutex<(u16, bool)>,
    /// La cámara del teléfono está en pausa: sin video se manda la imagen de
    /// pausa en lugar de negro.
    paused: AtomicBool,
    closed: AtomicBool,
}

/// El hilo del puente. Al soltarlo, deja de escribir y cierra la cámara.
pub struct Relay {
    shared: Arc<Shared>,
}

impl Relay {
    pub fn start(public: VirtualCam, internal: VirtualCam) -> Relay {
        let shared = Arc::new(Shared {
            size: Mutex::new(None),
            turn: Mutex::new((0, false)),
            paused: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        });
        {
            let shared = shared.clone();
            thread::spawn(move || run(&shared, &public, &internal));
        }
        Relay { shared }
    }

    /// Tamaño de la cámara «Ixtli». Si alguien la está usando, la cámara
    /// virtual no deja cambiarlo y se queda el que tenía.
    pub fn set_size(&self, size: Option<(usize, usize)>) {
        *self.shared.size.lock().unwrap() = size;
    }

    /// Giro y espejo de los cuadros del motor propio. Cambia en el siguiente
    /// cuadro, sin cortar la imagen.
    pub fn set_turn(&self, degrees: u16, mirror: bool) {
        *self.shared.turn.lock().unwrap() = (degrees % 360, mirror);
    }

    /// Mientras la cámara del teléfono está en pausa, «Ixtli» muestra la
    /// imagen de pausa.
    pub fn set_paused(&self, paused: bool) {
        self.shared.paused.store(paused, Ordering::Relaxed);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

fn run(shared: &Shared, public: &VirtualCam, internal: &VirtualCam) {
    ACTIVE.store(true, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(shared_dir());
    let mut feed = PreviewFeed::new();
    let mut output: Option<Output> = None;
    let mut input: Option<Input> = None;
    let mut frame = Vec::new();
    // Para los cuadros del motor propio: el que llega y el ya girado.
    let mut direct = Vec::new();
    let mut turned = Vec::new();
    let mut last_real: Option<Instant> = None;
    let mut last_black: Option<Instant> = None;
    // Qué tiene ya dibujado `frame` sin video: negro (false) o la imagen de
    // pausa (true). Así no se vuelve a dibujar 10 veces por segundo.
    let mut filler: Option<bool> = None;

    while !shared.closed.load(Ordering::Relaxed) {
        let wanted = *shared.size.lock().unwrap();
        let Some(wanted) = wanted else {
            output = None;
            input = None;
            thread::sleep(Duration::from_millis(200));
            continue;
        };
        if output.as_ref().map(|out| out.wanted) != Some(wanted) {
            output = None;
            match Output::open(public, wanted) {
                Ok(opened) => {
                    frame = vec![0; opened.width * opened.height * 3 / 2];
                    filler = None;
                    output = Some(opened);
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
            }
        }
        let Some(out) = output.as_mut() else { continue };

        // La entrada se cierra en cuanto scrcpy se va, para que el siguiente
        // pueda abrirla con otro tamaño.
        if input.as_ref().is_some_and(|inp| !internal.streaming() || internal.frame_size() != Some(inp.size)) {
            input = None;
        }
        if input.is_none() && internal.streaming() {
            input = internal.frame_size().and_then(|size| Input::open(internal, size));
        }

        let mut wrote = false;
        // Del motor propio: se gira aquí y se acomoda igual que lo de scrcpy.
        let wait = if input.is_none() { Duration::from_millis(20) } else { Duration::ZERO };
        if let Some((width, height)) = take_direct(wait, &mut direct) {
            if direct.len() >= width * height * 3 / 2 {
                let (degrees, mirror) = *shared.turn.lock().unwrap();
                fit_turned(&direct, width, height, (degrees, mirror), &mut frame, out.width, out.height, &mut turned);
                if out.write(&frame).is_err() {
                    output = None;
                    thread::sleep(Duration::from_millis(500));
                    continue;
                }
                feed.offer(&frame, out.width, out.height);
                last_real = Some(Instant::now());
                *LAST_REAL.lock().unwrap() = last_real;
                wrote = true;
                filler = None;
            }
        }
        if let Some(inp) = input.as_mut() {
            match inp.next() {
                Some(true) => {
                    let (width, height) = inp.size;
                    fit(&inp.raw, width, height, &mut frame, out.width, out.height);
                    if out.write(&frame).is_err() {
                        output = None;
                        thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                    feed.offer(&frame, out.width, out.height);
                    last_real = Some(Instant::now());
                    *LAST_REAL.lock().unwrap() = last_real;
                    wrote = true;
                    filler = None;
                }
                Some(false) => {}
                None => input = None,
            }
        }
        let quiet = last_real.is_none_or(|at| at.elapsed() > BLACK_AFTER);
        if !wrote && quiet && last_black.is_none_or(|at| at.elapsed() >= BLACK_EVERY) {
            let paused = shared.paused.load(Ordering::Relaxed);
            if filler != Some(paused) {
                if paused {
                    pause_image(&mut frame, out.width, out.height);
                } else {
                    black(&mut frame, out.width, out.height);
                }
                filler = Some(paused);
            }
            if out.write(&frame).is_err() {
                output = None;
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            last_black = Some(Instant::now());
        }
    }
    ACTIVE.store(false, Ordering::Relaxed);
    *LATEST.lock().unwrap() = None;
    *LAST_REAL.lock().unwrap() = None;
}

/// La cámara «Ixtli», abierta para escribir.
struct Output {
    file: File,
    /// El tamaño pedido.
    wanted: (usize, usize),
    /// El que dejó la cámara virtual (puede ser otro si alguien la usa).
    width: usize,
    height: usize,
}

impl Output {
    fn open(cam: &VirtualCam, wanted: (usize, usize)) -> std::io::Result<Output> {
        let file = OpenOptions::new().write(true).open(&cam.path)?;
        let (width, height) = set_format(&file, wanted)?;
        Ok(Output { file, wanted, width, height })
    }

    fn write(&mut self, frame: &[u8]) -> std::io::Result<()> {
        self.file.write_all(frame)
    }
}

/// La cámara interna, abierta para leer lo que manda scrcpy.
struct Input {
    file: File,
    size: (usize, usize),
    raw: Vec<u8>,
}

impl Input {
    fn open(cam: &VirtualCam, size: (usize, usize)) -> Option<Input> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&cam.path)
            .ok()?;
        Some(Input { file, size, raw: vec![0; size.0 * size.1 * 3 / 2] })
    }

    /// Espera un cuadro nuevo (como mucho 50 ms). `Some(true)`: llegó uno;
    /// `Some(false)`: todavía no; `None`: la cámara interna dejó de servir.
    fn next(&mut self) -> Option<bool> {
        // Primero se lee: v4l2loopback no avisa de cuadros nuevos (poll)
        // hasta que el lector hizo su primera lectura.
        match self.file.read(&mut self.raw) {
            Ok(read) if read == self.raw.len() => return Some(true),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => return None,
        }
        let mut poll = libc::pollfd { fd: self.file.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: `poll` apunta a una estructura válida durante la llamada.
        unsafe { libc::poll(&mut poll, 1, 50) };
        Some(false)
    }
}

// ---------------------------------------------------------------------------
// Formato de la cámara virtual (ioctl de V4L2)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Default)]
struct PixFormat {
    width: u32,
    height: u32,
    pixelformat: u32,
    field: u32,
    bytesperline: u32,
    sizeimage: u32,
    colorspace: u32,
    private: u32,
    flags: u32,
    ycbcr_enc: u32,
    quantization: u32,
    xfer_func: u32,
}

/// `struct v4l2_format`: 208 bytes; la unión empieza en el byte 8.
#[repr(C)]
struct Format {
    kind: u32,
    _align: u32,
    pix: PixFormat,
    _rest: [u8; 152],
}

const VIDIOC_S_FMT: libc::c_ulong = 0xC0D0_5605;
const BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const FIELD_NONE: u32 = 1;
/// YUV 4:2:0 por planos, el que usa scrcpy.
const PIX_FMT_YU12: u32 = u32::from_le_bytes(*b"YU12");

/// Pide el formato YU12 del tamaño dado y devuelve el que quedó.
fn set_format(file: &File, (width, height): (usize, usize)) -> std::io::Result<(usize, usize)> {
    let mut format = Format {
        kind: BUF_TYPE_VIDEO_OUTPUT,
        _align: 0,
        pix: PixFormat {
            width: width as u32,
            height: height as u32,
            pixelformat: PIX_FMT_YU12,
            field: FIELD_NONE,
            bytesperline: width as u32,
            sizeimage: (width * height * 3 / 2) as u32,
            ..PixFormat::default()
        },
        _rest: [0; 152],
    };
    // SAFETY: `format` tiene el tamaño y la forma de `struct v4l2_format`.
    let result = unsafe { libc::ioctl(file.as_raw_fd(), VIDIOC_S_FMT, &mut format) };
    if result < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if format.pix.pixelformat != PIX_FMT_YU12 || format.pix.width == 0 || format.pix.height == 0 {
        return Err(std::io::Error::other("la cámara virtual tiene otro formato"));
    }
    Ok((format.pix.width as usize, format.pix.height as usize))
}

// ---------------------------------------------------------------------------
// Imagen
// ---------------------------------------------------------------------------

const BLACK_Y: u8 = 16;
const BLACK_UV: u8 = 128;

/// Un cuadro negro en YU12.
fn black(frame: &mut [u8], width: usize, height: usize) {
    let luma = width * height;
    frame[..luma].fill(BLACK_Y);
    frame[luma..].fill(BLACK_UV);
}

/// La imagen de pausa en YU12: fondo gris oscuro y el símbolo de pausa (dos
/// barras claras) en el centro. Sin letras, para no depender de un idioma ni
/// de una fuente.
fn pause_image(frame: &mut [u8], width: usize, height: usize) {
    const BACK_Y: u8 = 40;
    const BAR_Y: u8 = 200;
    black(frame, width, height);
    frame[..width * height].fill(BACK_Y);
    let bar_h = height / 4;
    let bar_w = (height / 16).max(1);
    let gap = bar_w;
    let left = (width / 2).saturating_sub(gap / 2 + bar_w);
    let top = (height - bar_h) / 2;
    for row in top..top + bar_h {
        let line = &mut frame[row * width..(row + 1) * width];
        for x0 in [left, left + bar_w + gap] {
            let x1 = (x0 + bar_w).min(width);
            line[x0.min(width)..x1].fill(BAR_Y);
        }
    }
}

/// Como `fit`, pero girando la imagen `degrees` (0, 90, 180 o 270, sentido
/// del reloj) y, con `mirror`, volteándola de izquierda a derecha después de
/// girarla. Primero la escala al tamaño final sin girar (en `scratch`) y luego
/// gira solo esa imagen chica: girar el cuadro completo del sensor costaba el
/// triple.
#[allow(clippy::too_many_arguments)]
fn fit_turned(
    src: &[u8],
    sw: usize,
    sh: usize,
    (degrees, mirror): (u16, bool),
    dst: &mut [u8],
    dw: usize,
    dh: usize,
    scratch: &mut Vec<u8>,
) {
    let quarters = (degrees / 90) % 4;
    if quarters == 0 && !mirror {
        fit(src, sw, sh, dst, dw, dh);
        return;
    }
    // Tamaño de la imagen ya girada, y el rectángulo que ocupa en el destino.
    let (tw, th) = if quarters % 2 == 1 { (sh, sw) } else { (sw, sh) };
    let (rw, rh, x0, y0) = placement(tw, th, dw, dh);
    // El mismo rectángulo antes de girar.
    let (uw, uh) = if quarters % 2 == 1 { (rh, rw) } else { (rw, rh) };
    black(dst, dw, dh);
    if uw == 0 || uh == 0 {
        return;
    }
    scratch.resize(uw * uh * 3 / 2, 0);
    let (src_y, src_rest) = src.split_at(sw * sh);
    let (src_u, src_v) = src_rest.split_at((sw / 2) * (sh / 2));
    let (small_y, small_rest) = scratch.split_at_mut(uw * uh);
    let (small_u, small_v) = small_rest.split_at_mut((uw / 2) * (uh / 2));
    scale_plane(src_y, sw, sh, small_y, uw, (0, 0, uw, uh));
    let chroma = (sw / 2) * (sh / 2);
    scale_plane(&src_u[..chroma], sw / 2, sh / 2, small_u, uw / 2, (0, 0, uw / 2, uh / 2));
    scale_plane(&src_v[..chroma], sw / 2, sh / 2, small_v, uw / 2, (0, 0, uw / 2, uh / 2));

    let (dst_y, dst_rest) = dst.split_at_mut(dw * dh);
    let (dst_u, dst_v) = dst_rest.split_at_mut((dw / 2) * (dh / 2));
    orient_plane(small_y, uw, uh, dst_y, dw, (x0, y0), quarters, mirror);
    let half = (x0 / 2, y0 / 2);
    orient_plane(small_u, uw / 2, uh / 2, dst_u, dw / 2, half, quarters, mirror);
    orient_plane(small_v, uw / 2, uh / 2, dst_v, dw / 2, half, quarters, mirror);
}

/// Gira un plano de `sw`x`sh` y lo escribe en `dst` (de ancho `stride`) a
/// partir de `(x0, y0)`. Recorre el destino en orden, para escribir seguido.
#[allow(clippy::too_many_arguments)]
fn orient_plane(
    src: &[u8],
    sw: usize,
    sh: usize,
    dst: &mut [u8],
    stride: usize,
    (x0, y0): (usize, usize),
    quarters: u16,
    mirror: bool,
) {
    let (dw, dh) = if quarters % 2 == 1 { (sh, sw) } else { (sw, sh) };
    if dw == 0 || dh == 0 {
        return;
    }
    let (sw_i, sh_i) = (sw as isize, sh as isize);
    for y in 0..dh {
        let yi = y as isize;
        // Índice en el original del punto (0, y) del girado, y cuánto avanza
        // con cada punto a la derecha.
        let (mut start, mut step) = match quarters {
            1 => ((sh_i - 1) * sw_i + yi, -sw_i),
            2 => ((sh_i - 1 - yi) * sw_i + sw_i - 1, -1),
            3 => (sw_i - 1 - yi, sw_i),
            _ => (yi * sw_i, 1),
        };
        if mirror {
            start += (dw as isize - 1) * step;
            step = -step;
        }
        let row = &mut dst[(y0 + y) * stride + x0..(y0 + y) * stride + x0 + dw];
        let mut at = start;
        for pixel in row.iter_mut() {
            *pixel = src[at as usize];
            at += step;
        }
    }
}

/// Acomoda un cuadro YU12 de `sw`x`sh` dentro de otro de `dw`x`dh`, sin
/// deformarlo: lo escala hasta tocar los bordes y rellena con negro lo que
/// sobra (franjas arriba y abajo, o a los lados).
fn fit(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize) {
    if (sw, sh) == (dw, dh) {
        dst.copy_from_slice(&src[..dst.len()]);
        return;
    }
    let (rw, rh, x0, y0) = placement(sw, sh, dw, dh);
    black(dst, dw, dh);

    let (src_y, src_rest) = src.split_at(sw * sh);
    let (src_u, src_v) = src_rest.split_at((sw / 2) * (sh / 2));
    let (dst_y, dst_rest) = dst.split_at_mut(dw * dh);
    let (dst_u, dst_v) = dst_rest.split_at_mut((dw / 2) * (dh / 2));
    scale_plane(src_y, sw, sh, dst_y, dw, (x0, y0, rw, rh));
    let half = (x0 / 2, y0 / 2, rw / 2, rh / 2);
    scale_plane(&src_u[..(sw / 2) * (sh / 2)], sw / 2, sh / 2, dst_u, dw / 2, half);
    scale_plane(&src_v[..(sw / 2) * (sh / 2)], sw / 2, sh / 2, dst_v, dw / 2, half);
}

/// Dónde va una imagen de `sw`x`sh` dentro de una de `dw`x`dh` sin
/// deformarla: su tamaño y su esquina, con medidas pares por el color a la
/// mitad.
fn placement(sw: usize, sh: usize, dw: usize, dh: usize) -> (usize, usize, usize, usize) {
    let (rw, rh) = if sw * dh > dw * sh {
        (dw, (sh * dw / sw) & !1)
    } else {
        ((sw * dh / sh) & !1, dh)
    };
    (rw, rh, ((dw - rw) / 2) & !1, ((dh - rh) / 2) & !1)
}

/// Escala un plano con interpolación bilineal (en punto fijo) dentro del
/// rectángulo `(x0, y0, rw, rh)` de un plano de ancho `dw`.
fn scale_plane(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, rect: (usize, usize, usize, usize)) {
    let (x0, y0, rw, rh) = rect;
    if rw == 0 || rh == 0 || sw == 0 || sh == 0 {
        return;
    }
    // Posición en el origen de cada columna, en 1/256 de pixel.
    let columns: Vec<(usize, usize, u32)> = (0..rw)
        .map(|x| {
            let pos = ((x * 2 + 1) * sw * 256 / (rw * 2)).saturating_sub(128);
            let left = (pos >> 8).min(sw - 1);
            (left, (left + 1).min(sw - 1), (pos & 255) as u32)
        })
        .collect();
    for y in 0..rh {
        let pos = ((y * 2 + 1) * sh * 256 / (rh * 2)).saturating_sub(128);
        let top = (pos >> 8).min(sh - 1);
        let bottom = (top + 1).min(sh - 1);
        let fy = (pos & 255) as u32;
        let (row_a, row_b) = (&src[top * sw..top * sw + sw], &src[bottom * sw..bottom * sw + sw]);
        let out = &mut dst[(y0 + y) * dw + x0..(y0 + y) * dw + x0 + rw];
        for (pixel, &(left, right, fx)) in out.iter_mut().zip(&columns) {
            let upper = u32::from(row_a[left]) * (256 - fx) + u32::from(row_a[right]) * fx;
            let lower = u32::from(row_b[left]) * (256 - fx) + u32::from(row_b[right]) * fx;
            *pixel = ((upper * (256 - fy) + lower * fy + (1 << 15)) >> 16) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: usize, height: usize, y: u8, u: u8, v: u8) -> Vec<u8> {
        let mut raw = vec![y; width * height];
        raw.extend(vec![u; (width / 2) * (height / 2)]);
        raw.extend(vec![v; (width / 2) * (height / 2)]);
        raw
    }

    #[test]
    fn format_struct_matches_v4l2() {
        assert_eq!(std::mem::size_of::<Format>(), 208);
        assert_eq!(std::mem::offset_of!(Format, pix), 8);
        assert_eq!(PIX_FMT_YU12, 0x3231_5559);
    }

    #[test]
    fn shared_frame_roundtrip() {
        let frame = Frame { width: 2, height: 1, rgba: vec![1, 2, 3, 255, 4, 5, 6, 255].into(), number: 7 };
        let back = decode_shared(encode_shared(&frame)).unwrap();
        assert_eq!((back.width, back.height, back.number), (2, 1, 7));
        assert_eq!(back.rgba, frame.rgba);
        assert!(decode_shared(vec![0; 10]).is_none());
        assert!(decode_shared(vec![0; 17]).is_none());
    }

    #[test]
    fn orient_turns_and_mirrors() {
        // Plano de 3x2:  a b c
        //                d e f
        let src = [1, 2, 3, 4, 5, 6];
        let turn = |quarters, mirror| {
            let (w, h) = if quarters % 2 == 1 { (2, 3) } else { (3, 2) };
            let mut dst = vec![0; w * h];
            orient_plane(&src, 3, 2, &mut dst, w, (0, 0), quarters, mirror);
            dst
        };
        // Un cuarto de vuelta a la derecha:  d a / e b / f c
        assert_eq!(turn(1, false), [4, 1, 5, 2, 6, 3]);
        assert_eq!(turn(2, false), [6, 5, 4, 3, 2, 1]);
        assert_eq!(turn(3, false), [3, 6, 2, 5, 1, 4]);
        // Espejo después de girar.
        assert_eq!(turn(0, true), [3, 2, 1, 6, 5, 4]);
        assert_eq!(turn(1, true), [1, 4, 2, 5, 3, 6]);
        assert_eq!(turn(3, true), [6, 3, 5, 2, 4, 1]);
        // Dentro de un plano más grande, a partir de (1, 1).
        let mut big = vec![0; 4 * 4];
        orient_plane(&src, 3, 2, &mut big, 4, (1, 1), 1, false);
        assert_eq!(&big[5..7], &[4, 1]);
        assert_eq!(&big[13..15], &[6, 3]);
    }

    #[test]
    fn turned_frame_fits_like_a_turned_image() {
        // Un cuadro horizontal del sensor (1280x720) girado 90°: queda
        // vertical dentro de 1280x720, con franjas a los lados.
        let mut src = solid(1280, 720, 0, 128, 128);
        // La fila de arriba, clara: al girar a la derecha queda a la derecha.
        src[..1280].fill(235);
        let mut dst = vec![0; 1280 * 720 * 3 / 2];
        let mut scratch = Vec::new();
        fit_turned(&src, 1280, 720, (90, false), &mut dst, 1280, 720, &mut scratch);
        let y = |x: usize, row: usize| dst[row * 1280 + x];
        // 720 de alto → 404 de ancho, desde la columna 438.
        assert_eq!(y(437, 360), BLACK_Y);
        assert_eq!(y(440, 360), 0);
        assert!(y(841, 360) > 100);
        assert_eq!(y(842, 360), BLACK_Y);
        // Con espejo, la fila clara queda a la izquierda.
        fit_turned(&src, 1280, 720, (90, true), &mut dst, 1280, 720, &mut scratch);
        assert!(dst[360 * 1280 + 438] > 100);
        assert_eq!(dst[360 * 1280 + 841], 0);
        // 180°: del mismo tamaño, la fila clara abajo.
        fit_turned(&src, 1280, 720, (180, false), &mut dst, 1280, 720, &mut scratch);
        assert!(dst[719 * 1280 + 640] > 100);
        assert_eq!(dst[640], 0);
    }

    #[test]
    fn direct_frames_swap_buffers() {
        let mut frame = vec![7; 6];
        deliver(2, 2, &mut frame);
        let mut into = vec![1; 3];
        assert_eq!(take_direct(Duration::ZERO, &mut into), Some((2, 2)));
        assert_eq!(into, vec![7; 6]);
        assert_eq!(take_direct(Duration::ZERO, &mut into), None);
    }

    #[test]
    fn pause_image_has_two_centered_bars() {
        let (w, h) = (64, 32);
        let mut frame = vec![0; w * h * 3 / 2];
        pause_image(&mut frame, w, h);
        let middle = &frame[(h / 2) * w..(h / 2 + 1) * w];
        let lit: Vec<usize> = (0..w).filter(|&x| middle[x] == 200).collect();
        // Dos barras de 2 píxeles separadas por 2, alrededor del centro.
        assert_eq!(lit, vec![29, 30, 33, 34]);
        // Arriba no hay barras y el color es neutro.
        assert!(frame[..w].iter().all(|&y| y == 40));
        assert!(frame[w * h..].iter().all(|&c| c == BLACK_UV));
    }

    #[test]
    fn same_size_is_a_copy() {
        let src = solid(4, 2, 200, 90, 60);
        let mut dst = vec![0; src.len()];
        fit(&src, 4, 2, &mut dst, 4, 2);
        assert_eq!(dst, src);
    }

    #[test]
    fn vertical_image_gets_side_bars() {
        // Teléfono vertical (720x1280) dentro de 1280x720: franjas a los lados.
        let src = solid(720, 1280, 200, 90, 60);
        let mut dst = vec![0; 1280 * 720 * 3 / 2];
        fit(&src, 720, 1280, &mut dst, 1280, 720);
        let y = |x: usize, row: usize| dst[row * 1280 + x];
        // 720 de alto → 404 de ancho, centrado desde la columna 438.
        assert_eq!(y(0, 360), BLACK_Y);
        assert_eq!(y(437, 360), BLACK_Y);
        assert_eq!(y(640, 360), 200);
        assert_eq!(y(1279, 360), BLACK_Y);
        let u = &dst[1280 * 720..1280 * 720 + 640 * 360];
        assert_eq!(u[180 * 640], BLACK_UV);
        assert_eq!(u[180 * 640 + 320], 90);
    }

    #[test]
    fn saver_image_is_enlarged() {
        // Modo ahorro (720x480, 3:2) dentro de 1280x720: 1080x720 al centro.
        let src = solid(720, 480, 120, 128, 128);
        let mut dst = vec![0; 1280 * 720 * 3 / 2];
        fit(&src, 720, 480, &mut dst, 1280, 720);
        assert_eq!(dst[100 * 1280 + 99], BLACK_Y);
        assert_eq!(dst[100 * 1280 + 100], 120);
        assert_eq!(dst[100 * 1280 + 1179], 120);
        assert_eq!(dst[100 * 1280 + 1180], BLACK_Y);
    }

    #[test]
    fn scaling_keeps_gradients_smooth() {
        // Una rampa horizontal escalada al doble sigue creciendo.
        let width = 8;
        let src: Vec<u8> = (0..width * 2).map(|i| ((i % width) * 30) as u8).collect();
        let mut dst = vec![0; 16 * 4];
        scale_plane(&src, width, 2, &mut dst, 16, (0, 0, 16, 4));
        let row = &dst[16..32];
        assert!(row.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(row[0], 0);
        assert!(row[15] >= 200);
    }
}
