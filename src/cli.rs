//! Uso desde la terminal. Sin argumentos se abre la ventana.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::audio;
use crate::config::{self, Facing, Quality, Rotation};
use crate::engine::{Engine, Phase};
use crate::gui;
use crate::sys::{self, Deps, Phone, Tool};

const HELP: &str = "\
Ixtli: usa tu celular Android como cámara web y micrófono.

Uso:
  ixtli                      Abre la ventana.
  ixtli --encender           Abre la ventana y enciende la cámara de una vez.
  ixtli estado               Revisa si todo está listo.
  ixtli encender [opciones]  Enciende la cámara desde la terminal (Ctrl+C para apagar).
  ixtli vista                Abre una ventana con la imagen de la cámara.

Opciones de «encender» (si no pones ninguna, se usan las de la ventana):
  --frontal | --trasera      Qué cámara del teléfono usar.
  --720 | --1080             Calidad del video.
  --giro=auto|0|90|180|270   Girar la imagen (auto sigue la posición del teléfono).
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

    let phone = if deps.adb { sys::phone() } else { Phone::Missing };
    match &phone {
        Phone::Ready { model, .. } => println!("{} Teléfono: {model} conectado", mark(true)),
        other => println!("{} Teléfono: {}", mark(false), other.advice()),
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

fn start(flags: &[String]) -> ExitCode {
    let mut options = config::load();
    for flag in flags {
        match flag.as_str() {
            "--frontal" => options.facing = Facing::Front,
            "--trasera" => options.facing = Facing::Back,
            "--720" => options.quality = Quality::Hd,
            "--1080" => options.quality = Quality::FullHd,
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
