//! Uso desde la terminal. Sin argumentos se abre la ventana.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::audio;
use crate::config::{self, Facing, Quality, Remembered, Rotation};
use crate::engine::{Engine, Phase};
use crate::gui;
use crate::pairing::{self, Pairing, Progress};
use crate::sys::{self, DeviceState, Deps, Link, Phone, Tool};

const HELP: &str = "\
Ixtli: usa tu celular Android como cámara web y micrófono.

Uso:
  ixtli                      Abre la ventana.
  ixtli --encender           Abre la ventana y enciende la cámara de una vez.
  ixtli estado               Revisa si todo está listo.
  ixtli encender [opciones]  Enciende la cámara desde la terminal (Ctrl+C para apagar).
  ixtli vista                Abre una ventana con la imagen de la cámara.
  ixtli wifi                 Pasa a WiFi el teléfono conectado por USB y lo recuerda.
  ixtli wifi olvidar         Olvida el teléfono y lo desconecta de la red.
  ixtli vincular             Vincula el teléfono sin cable con un código QR.
  ixtli vincular IP:PUERTO CÓDIGO
                             Vincula con el código de 6 dígitos que muestra el teléfono
                             (sirve por VPN o si el QR no funciona).

Opciones de «encender» (si no pones ninguna, se usan las de la ventana):
  --frontal | --trasera      Qué cámara del teléfono usar.
  --720 | --1080             Calidad del video.
  --giro=auto|0|90|180|270   Girar la imagen (auto sigue la posición del teléfono).
  --espejo | --sin-espejo    Voltear la imagen como en un espejo.
  --angulo=GRADOS            Giro extra a cualquier ángulo, de -180 a 180.
  --micro | --sin-micro      Usar o no el micrófono del teléfono.

  -h, --help                 Muestra esta ayuda.
  -V, --version              Muestra la versión.";

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("--encender") => match gui::run(args.first().is_some()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("No se pudo abrir la ventana: {e}");
                eprintln!("Puedes usar Ixtli desde la terminal con: ixtli encender");
                ExitCode::FAILURE
            }
        },
        Some("estado" | "status") => status(),
        Some("vista") => match gui::run_viewer() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("No se pudo abrir la ventana: {e}");
                ExitCode::FAILURE
            }
        },
        Some("encender" | "start") => start(&args[1..]),
        Some("vincular" | "pair") => pair(&args[1..]),
        Some("wifi") => match args.get(1).map(String::as_str) {
            None => wifi(),
            Some("olvidar" | "forget") => {
                if let Some(phone) = config::load_phone() {
                    pairing::disconnect(&phone);
                    config::save_phone(None);
                    println!("Listo: olvidé tu {}.", phone.model);
                    if phone.paired {
                        println!("Para quitar también la vinculación, en el teléfono abre Depuración inalámbrica y olvida esta computadora.");
                    }
                } else {
                    println!("No hay ningún teléfono recordado.");
                }
                ExitCode::SUCCESS
            }
            Some(other) => {
                eprintln!("No conozco «wifi {other}».\n\n{HELP}");
                ExitCode::from(2)
            }
        },
        Some("-h" | "--help" | "ayuda" | "help") => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("ixtli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("No conozco «{other}».\n\n{HELP}");
            ExitCode::from(2)
        }
    }
}

fn mark(ok: bool) -> &'static str {
    if ok {
        "[ok]"
    } else {
        "[!!]"
    }
}

fn status() -> ExitCode {
    let deps = Deps::check();
    let scrcpy = match &deps.scrcpy {
        Tool::Ok => "instalado".to_string(),
        Tool::TooOld(version) => format!("versión {version}, muy vieja (se necesita 3.0 o más nueva)"),
        Tool::Missing => "falta instalarlo".to_string(),
    };
    println!("{} scrcpy: {scrcpy}", mark(deps.scrcpy == Tool::Ok));
    println!("{} adb: {}", mark(deps.adb), if deps.adb { "instalado" } else { "falta instalarlo" });
    println!(
        "{} v4l2loopback: {}",
        mark(deps.module),
        if deps.module { "instalado" } else { "falta instalarlo" }
    );
    println!(
        "{} pactl (para el micrófono): {}",
        mark(deps.pactl),
        if deps.pactl { "instalado" } else { "falta instalarlo" }
    );
    if !deps.missing_packages().is_empty() {
        println!("     Instala lo que falta con: {}", deps.install_command());
    }

    let remembered = config::load_phone();
    let mut phone = if deps.adb { sys::phone() } else { Phone::Missing };
    // Sin cable, se intenta con el teléfono recordado.
    let mut wifi_problem = None;
    if let (true, false, Some(wanted)) = (deps.adb, matches!(phone, Phone::Ready { .. }), &remembered) {
        match pairing::reconnect(wanted) {
            Ok(()) => phone = sys::phone(),
            Err(problem) => wifi_problem = Some(problem),
        }
    }
    match &phone {
        Phone::Ready { model, link: Link::Usb, .. } => {
            println!("{} Teléfono: {model} conectado por USB", mark(true))
        }
        Phone::Ready { model, link: Link::Wifi, .. } => {
            println!("{} Teléfono: {model} conectado por WiFi", mark(true))
        }
        other => println!("{} Teléfono: {}", mark(false), other.advice()),
    }
    if let Some(wanted) = &remembered {
        let connected = pairing::matching(wanted, &sys::devices())
            .iter()
            .any(|device| device.state == DeviceState::Ready);
        println!(
            "     WiFi: recuerdo tu {}{} ({})",
            wanted.model,
            if wanted.paired { ", vinculado sin cable".to_string() } else { format!(" en {}", wanted.addr) },
            if connected { "conectado" } else { "sin conexión" }
        );
        if let Some(problem) = wifi_problem {
            println!("     {problem}");
        }
    }

    let cam = sys::virtual_cam();
    match &cam {
        Some(cam) => println!(
            "{} Cámara virtual: «{}» en {}",
            mark(true),
            cam.label,
            cam.path.display()
        ),
        None if deps.module => println!(
            "{} Cámara virtual: apagada. Se activa sola al encender, o con: {}",
            mark(false),
            sys::module_command()
        ),
        None => println!("{} Cámara virtual: falta instalar v4l2loopback", mark(false)),
    }

    let ready = deps.ready() && matches!(phone, Phone::Ready { .. });
    if ready {
        println!("\nTodo listo. Enciende con: ixtli encender");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// `ixtli wifi`: pasa a WiFi el teléfono conectado por USB.
fn wifi() -> ExitCode {
    if !Deps::check().adb {
        eprintln!("Falta instalar adb: sudo apt install adb");
        return ExitCode::FAILURE;
    }
    let (serial, model) = match sys::phone() {
        Phone::Ready { serial, model, link: Link::Usb } => (serial, model),
        Phone::Ready { serial, .. } => {
            println!("El teléfono ya está conectado por WiFi ({serial}).");
            return ExitCode::SUCCESS;
        }
        Phone::Missing => {
            eprintln!("Conecta el teléfono por USB para pasarlo a WiFi (solo hace falta una vez).");
            return ExitCode::FAILURE;
        }
        other => {
            eprintln!("{}", other.advice());
            return ExitCode::FAILURE;
        }
    };
    println!("Pasando el teléfono a WiFi...");
    match sys::wifi_enable(&serial) {
        Ok(addr) => {
            if let Some(old) = config::load_phone().filter(|old| old.addr != addr) {
                pairing::disconnect(&old);
            }
            config::save_phone(Some(&Remembered { addr: addr.clone(), model, paired: false, last: None }));
            println!("Listo: conectado en {addr}. Ya puedes desconectar el cable.");
            println!("Ixtli recordará el teléfono y se reconectará solo mientras esté en la misma red.");
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// `ixtli vincular`: muestra un código QR y espera a que el teléfono lo escanee.
///
/// Con una dirección y un código, vincula con el código de 6 dígitos.
fn pair(args: &[String]) -> ExitCode {
    if !Deps::check().adb {
        eprintln!("Falta instalar adb: sudo apt install adb");
        return ExitCode::FAILURE;
    }
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    }
    let mut job = match args {
        [] => {
            let job = Pairing::new();
            match pairing::qr_for_terminal(&job.qr_text()) {
                Some(qr) => println!("{qr}"),
                None => {
                    eprintln!("No se pudo crear el código QR.");
                    return ExitCode::FAILURE;
                }
            }
            println!("En el teléfono: Opciones de desarrollador → Depuración inalámbrica →");
            println!("«Vincular dispositivo con código QR», y escanea este código con el lector que se abre.");
            println!("No uses la app de cámara: la cámara lo confunde con una red WiFi.");
            println!("El teléfono y esta computadora deben estar en la misma red WiFi.\n");
            println!("Esperando al teléfono... (Ctrl+C para cancelar)");
            job
        }
        [addr, code] if sys::valid_addr(addr) && code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) => {
            println!("Vinculando con {addr}...");
            Pairing::with_code(addr.clone(), code.clone())
        }
        _ => {
            eprintln!("Uso: ixtli vincular            (código QR)");
            eprintln!("     ixtli vincular IP:PUERTO CÓDIGO   (código de 6 dígitos)");
            return ExitCode::from(2);
        }
    };
    let mut announced = false;
    loop {
        if interrupted.load(Ordering::SeqCst) {
            println!("Cancelado.");
            return ExitCode::FAILURE;
        }
        match job.step() {
            Progress::Waiting => {}
            Progress::Connecting if !announced => {
                announced = true;
                println!("Vinculado. Conectando...");
            }
            Progress::Connecting => {}
            Progress::NeedAddress(problem) => {
                if let Some(problem) = problem {
                    eprintln!("{problem}");
                }
                println!("El teléfono ya quedó vinculado, pero no lo encuentro en la red.");
                println!("Escribe la «Dirección IP y puerto» que muestra Depuración inalámbrica (Enter vacío para cancelar):");
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
                let addr = line.trim();
                if addr.is_empty() {
                    println!("Cancelado.");
                    return ExitCode::FAILURE;
                }
                if sys::valid_addr(addr) {
                    job.set_connect_addr(addr.to_string());
                } else {
                    eprintln!("Eso no tiene forma de IP:puerto, por ejemplo 192.168.1.50:37211.");
                }
                continue;
            }
            Progress::Done(phone) => {
                if let Some(old) = config::load_phone().filter(|old| old.addr != phone.addr) {
                    pairing::disconnect(&old);
                }
                config::save_phone(Some(&phone));
                println!("Listo: {} conectado sin cable.", phone.model);
                println!("Ixtli lo encontrará solo mientras tenga encendida la depuración inalámbrica.");
                return ExitCode::SUCCESS;
            }
            Progress::Failed(problem) => {
                eprintln!("{problem}");
                return ExitCode::FAILURE;
            }
        }
        thread::sleep(Duration::from_millis(800));
    }
}

fn start(flags: &[String]) -> ExitCode {
    let mut options = config::load();
    for flag in flags {
        match flag.as_str() {
            "--frontal" => options.facing = Facing::Front,
            "--trasera" => options.facing = Facing::Back,
            "--720" => options.quality = Quality::Hd,
            "--1080" => options.quality = Quality::FullHd,
            "--espejo" => options.mirror = true,
            "--sin-espejo" => options.mirror = false,
            angle if angle.starts_with("--angulo=") => match angle["--angulo=".len()..].parse::<i16>() {
                Ok(degrees) if (-180..=180).contains(&degrees) => options.angle = degrees,
                _ => {
                    eprintln!("El ángulo debe ser un número entre -180 y 180.");
                    return ExitCode::from(2);
                }
            },
            "--micro" => options.mic = true,
            "--sin-micro" => options.mic = false,
            "--giro=auto" => options.rotation = Rotation::Auto,
            "--giro=0" => options.rotation = Rotation::Fixed(0),
            "--giro=90" => options.rotation = Rotation::Fixed(90),
            "--giro=180" => options.rotation = Rotation::Fixed(180),
            "--giro=270" => options.rotation = Rotation::Fixed(270),
            other => {
                eprintln!("No conozco la opción «{other}».\n\n{HELP}");
                return ExitCode::from(2);
            }
        }
    }

    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    }

    let engine = Engine::spawn();
    engine.start(options);
    println!("Conectando con el teléfono...");

    let mut announced = false;
    let code = loop {
        thread::sleep(Duration::from_millis(150));
        if interrupted.load(Ordering::SeqCst) {
            println!("\nApagando...");
            break ExitCode::SUCCESS;
        }
        let state = engine.snapshot();
        match state.phase {
            Phase::Live if !announced => {
                announced = true;
                let cam = state.cam.map(|cam| cam.label).unwrap_or_default();
                println!("En vivo. En tu videollamada elige la cámara «{cam}».");
                if state.mic {
                    println!("Micrófono: «{}».", audio::MIC_LABEL);
                }
                println!("Presiona Ctrl+C para apagar.");
            }
            Phase::Off => {
                break match state.error {
                    Some(problem) => {
                        eprintln!("{problem}");
                        ExitCode::FAILURE
                    }
                    // scrcpy se cerró solo (por ejemplo, al cerrar la vista previa).
                    None => ExitCode::SUCCESS,
                };
            }
            _ => {}
        }
    };
    engine.shutdown();
    code
}
