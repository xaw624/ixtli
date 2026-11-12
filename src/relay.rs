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
use std::sync::{Arc, Mutex};
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
    let mut last_real: Option<Instant> = None;
    let mut last_black: Option<Instant> = None;

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
                }
                Some(false) => {}
                None => input = None,
            }
        }
        let quiet = last_real.is_none_or(|at| at.elapsed() > BLACK_AFTER);
        if !wrote && quiet && last_black.is_none_or(|at| at.elapsed() >= BLACK_EVERY) {
            black(&mut frame, out.width, out.height);
            if out.write(&frame).is_err() {
                output = None;
                thread::sleep(Duration::from_millis(500));
                continue;
            }
            last_black = Some(Instant::now());
        }
        if input.is_none() {
            thread::sleep(Duration::from_millis(20));
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

/// Acomoda un cuadro YU12 de `sw`x`sh` dentro de otro de `dw`x`dh`, sin
/// deformarlo: lo escala hasta tocar los bordes y rellena con negro lo que
/// sobra (franjas arriba y abajo, o a los lados).
fn fit(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize) {
    if (sw, sh) == (dw, dh) {
        dst.copy_from_slice(&src[..dst.len()]);
        return;
    }
    // Rectángulo de destino, con medidas pares por el color a la mitad.
    let (rw, rh) = if sw * dh > dw * sh {
        (dw, (sh * dw / sw) & !1)
    } else {
        ((sw * dh / sh) & !1, dh)
    };
    let (x0, y0) = (((dw - rw) / 2) & !1, ((dh - rh) / 2) & !1);
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
