//! Vista previa: la imagen que el puente (`relay.rs`) manda a la cámara
//! «Ixtli», así que lo que se ve aquí es lo mismo que verán los demás.
//!
//! No lee la cámara virtual: v4l2loopback solo deja leer a un programa a la
//! vez, y si la tomaba Ixtli, la videollamada se quedaba sin cámara.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bytes::Bytes;

use crate::{relay, sys};

/// Lado más largo de la imagen de la vista previa, en pixeles.
const MAX_SIDE: usize = 640;

pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Se comparte sin copiar con la ventana (`image::Handle::from_rgba`).
    pub rgba: Bytes,
    /// Cambia con cada imagen nueva.
    pub number: u64,
}

struct Shared {
    wanted: AtomicBool,
    closed: AtomicBool,
    latest: Mutex<Option<Arc<Frame>>>,
}

/// Un hilo que mantiene la imagen más reciente mientras alguien la quiera ver.
pub struct Preview {
    shared: Arc<Shared>,
}

impl Preview {
    pub fn spawn() -> Preview {
        let shared = Arc::new(Shared {
            wanted: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            latest: Mutex::new(None),
        });
        {
            let shared = shared.clone();
            thread::spawn(move || watch(&shared));
        }
        Preview { shared }
    }

    /// Prende o apaga la lectura. Apagada no gasta nada.
    pub fn set_wanted(&self, wanted: bool) {
        self.shared.wanted.store(wanted, Ordering::Relaxed);
        if !wanted {
            *self.shared.latest.lock().unwrap() = None;
        }
    }

    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.shared.latest.lock().unwrap().clone()
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

fn watch(shared: &Shared) {
    while !shared.closed.load(Ordering::Relaxed) {
        let wanted = shared.wanted.load(Ordering::Relaxed);
        // Con el puente en este proceso, la imagen sale de él.
        if relay::active() {
            relay::want_preview(wanted);
            *shared.latest.lock().unwrap() = if wanted { relay::latest() } else { None };
            thread::sleep(relay::PREVIEW_EVERY);
            continue;
        }
        relay::want_preview(false);
        // Si el puente es de otro proceso de Ixtli (por ejemplo, esta es la
        // ventana suelta), deja la imagen en memoria. Sin cámara interna
        // (módulo viejo) no hay puente ni vista previa: hay que actualizarla.
        let frame = if wanted && sys::internal_cam().is_some() { relay::shared_frame() } else { None };
        *shared.latest.lock().unwrap() = frame.map(Arc::new);
        thread::sleep(if wanted { relay::PREVIEW_EVERY } else { Duration::from_millis(300) });
    }
}

/// Convierte YU12 (YUV 4:2:0 por planos) a RGBA, reduciendo el tamaño.
pub fn convert(raw: &[u8], width: usize, height: usize, number: u64) -> Frame {
    let step = width.max(height).div_ceil(MAX_SIDE).max(1);
    let (out_width, out_height) = (width / step, height / step);
    let (u_plane, v_plane) = (width * height, width * height + (width / 2) * (height / 2));
    let mut rgba = Vec::with_capacity(out_width * out_height * 4);
    for row in 0..out_height {
        let y_row = row * step;
        for column in 0..out_width {
            let x = column * step;
            let chroma = (y_row / 2) * (width / 2) + x / 2;
            let y = i32::from(raw[y_row * width + x]) - 16;
            let u = i32::from(raw[u_plane + chroma]) - 128;
            let v = i32::from(raw[v_plane + chroma]) - 128;
            // BT.601, rango de video.
            let red = (298 * y + 409 * v + 128) >> 8;
            let green = (298 * y - 100 * u - 208 * v + 128) >> 8;
            let blue = (298 * y + 516 * u + 128) >> 8;
            rgba.extend_from_slice(&[
                red.clamp(0, 255) as u8,
                green.clamp(0, 255) as u8,
                blue.clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    Frame {
        width: out_width as u32,
        height: out_height as u32,
        rgba: rgba.into(),
        number,
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
    fn converts_colors() {
        let white = convert(&solid(4, 2, 235, 128, 128), 4, 2, 1);
        assert_eq!((white.width, white.height), (4, 2));
        assert_eq!(&white.rgba[..4], &[255, 255, 255, 255]);
        let black = convert(&solid(4, 2, 16, 128, 128), 4, 2, 2);
        assert_eq!(&black.rgba[..4], &[0, 0, 0, 255]);
        let red = convert(&solid(4, 2, 81, 90, 240), 4, 2, 3);
        assert!(red.rgba[0] > 240 && red.rgba[1] < 10 && red.rgba[2] < 10);
    }

    #[test]
    fn shrinks_large_frames() {
        let frame = convert(&solid(1280, 720, 100, 128, 128), 1280, 720, 1);
        assert_eq!((frame.width, frame.height), (640, 360));
        assert_eq!(frame.rgba.len(), 640 * 360 * 4);
        let tall = convert(&solid(1080, 1920, 100, 128, 128), 1080, 1920, 1);
        assert_eq!((tall.width, tall.height), (360, 640));
    }
}
