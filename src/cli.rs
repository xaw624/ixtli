//! Uso desde la terminal. Sin argumentos se abre la ventana.

use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::audio;
use crate::config::{self, Facing, Options, Quality, Remembered, Rotation};
use crate::engine::{Engine, Phase};
use crate::gui;
use crate::measure;
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
  ixtli medir [MINUTOS] [opciones]
                             Enciende la cámara y mide cada minuto la batería del teléfono
                             y la CPU y memoria en la computadora (10 minutos si no dices).
  ixtli lentes               Muestra las cámaras del teléfono, para elegir con --lente.
  ixtli vincular             Vincula el teléfono sin cable con un código QR.
  ixtli vincular IP:PUERTO CÓDIGO
                             Vincula con el código de 6 dígitos que muestra el teléfono
                             (sirve por VPN o si el QR no funciona).

Opciones de «encender» (si no pones ninguna, se usan las de la ventana):
  --frontal | --trasera      Qué cámara del teléfono usar.
  --lente=N | --lente=principal
                             Otro lente del mismo lado, por ejemplo el gran angular
                             (los números salen en «ixtli lentes»).
  --720 | --1080             Calidad del video.
  --ahorro | --sin-ahorro    Modo ahorro: cerca de 480p, 15 cuadros/s y menos tasa de bits,
                             para que el teléfono gaste menos batería.
  --giro=auto|0|90|180|270   Girar la imagen (auto sigue la posición del teléfono).
  --espejo | --sin-espejo    Voltear la imagen como en un espejo.
  --angulo=GRADOS            Giro extra a cualquier ángulo, de -180 a 180.
  --micro | --sin-micro      Usar o no el micrófono del teléfono.
  --pantalla-prendida        No apagar la pantalla del teléfono. Normalmente se apaga
                             mientras la cámara está encendida, para gastar menos
                             batería (desbloquearlo la prende).

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
        Some("medir" | "measure") => measure_run(&args[1..]),
        Some("vincular" | "pair") => pair(&args[1..]),
        Some("lentes" | "lenses") => lenses(),
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

/// `ixtli lentes`: las cámaras del teléfono, agrupadas por lado.
fn lenses() -> ExitCode {
    let serial = match sys::phone() {
        Phone::Ready { serial, .. } => serial,
        other => {
            eprintln!("{}", other.advice());
            return ExitCode::FAILURE;
        }
    };
    println!("Preguntando al teléfono...");
    let Some(cameras) = sys::phone_cameras(&serial) else {
        eprintln!("El teléfono no respondió con su lista de cámaras.");
        return ExitCode::FAILURE;
    };
    for (facing, name, flag) in [(Facing::Front, "Frontal", "--frontal"), (Facing::Back, "Trasera", "--trasera")] {
        let lenses = config::lenses(&cameras, facing);
        if lenses.is_empty() {
            continue;
        }
        println!("\n{name}:");
        for (position, camera) in lenses.iter().enumerate() {
            let mp = camera.megapixels().map(|mp| format!(", sensor de {mp} MP")).unwrap_or_default();
            if position == 0 {
                println!("  principal{mp}: ixtli encender {flag}");
            } else {
                println!("  lente {}{mp}: ixtli encender {flag} --lente={}", camera.id, camera.id);
            }
        }
    }
    println!("\nAndroid no dice cuál es el gran angular: prueba cada uno.");
    ExitCode::SUCCESS
}

/// Lee las opciones de «encender» sobre las guardadas en la ventana.
fn read_options(flags: &[String]) -> Result<Options, ExitCode> {
    let mut options = config::load();
    for flag in flags {
        match flag.as_str() {
            "--frontal" => options.facing = Facing::Front,
            "--trasera" => options.facing = Facing::Back,
            "--720" => options.quality = Quality::Hd,
            "--1080" => options.quality = Quality::FullHd,
            "--lente=principal" => options.lens = None,
            lens if lens.starts_with("--lente=") => match lens["--lente=".len()..].parse::<u16>() {
                Ok(id) => options.lens = Some(id),
                Err(_) => {
                    eprintln!("El lente es un número de «ixtli lentes», o «principal».");
                    return Err(ExitCode::from(2));
                }
            },
            "--ahorro" => options.saver = true,
            "--sin-ahorro" => options.saver = false,
            "--espejo" => options.mirror = true,
            "--sin-espejo" => options.mirror = false,
            angle if angle.starts_with("--angulo=") => match angle["--angulo=".len()..].parse::<i16>() {
                Ok(degrees) if (-180..=180).contains(&degrees) => options.angle = degrees,
                _ => {
                    eprintln!("El ángulo debe ser un número entre -180 y 180.");
                    return Err(ExitCode::from(2));
                }
            },
            "--pantalla-apagada" => options.screen_off = true,
            "--pantalla-prendida" => options.screen_off = false,
            "--micro" => options.mic = true,
            "--sin-micro" => options.mic = false,
            "--giro=auto" => options.rotation = Rotation::Auto,
            "--giro=0" => options.rotation = Rotation::Fixed(0),
            "--giro=90" => options.rotation = Rotation::Fixed(90),
            "--giro=180" => options.rotation = Rotation::Fixed(180),
            "--giro=270" => options.rotation = Rotation::Fixed(270),
            other => {
                eprintln!("No conozco la opción «{other}».\n\n{HELP}");
                return Err(ExitCode::from(2));
            }
        }
    }
    Ok(options)
}

/// `ixtli medir`: enciende la cámara y anota cada minuto la batería y la
/// temperatura del teléfono, y la CPU y la memoria que usa Ixtli.
fn measure_run(args: &[String]) -> ExitCode {
    let mut minutes = 10.0_f64;
    let mut every = Duration::from_secs(60);
    let mut flags = Vec::new();
    for arg in args {
        if let Ok(value) = arg.parse::<f64>() {
            minutes = value;
        } else if let Some(seconds) = arg.strip_prefix("--cada=").and_then(|v| v.parse::<u64>().ok()) {
            every = Duration::from_secs(seconds.max(1));
        } else {
            flags.push(arg.clone());
        }
    }
    if !(minutes > 0.0 && minutes <= 600.0) {
        eprintln!("Los minutos deben ser un número entre 1 y 600.");
        return ExitCode::from(2);
    }
    let options = match read_options(&flags) {
        Ok(options) => options,
        Err(code) => return code,
    };

    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    }
    let engine = Engine::spawn();
    engine.start(options);
    println!("Conectando con el teléfono...");
    let mut capture = None;
    let live = loop {
        thread::sleep(Duration::from_millis(150));
        let state = engine.snapshot();
        match state.phase {
            Phase::Live => {
                capture = state.capture;
                break true;
            }
            Phase::Off => {
                eprintln!("{}", state.error.unwrap_or_else(|| "No se pudo encender la cámara.".into()));
                break false;
            }
            Phase::Starting if interrupted.load(Ordering::SeqCst) => break false,
            Phase::Starting => {}
        }
    };
    let Some((serial, model, link)) = live
        .then(sys::phone)
        .and_then(|phone| match phone {
            Phone::Ready { serial, model, link } => Some((serial, model, link)),
            _ => None,
        })
    else {
        engine.shutdown();
        return ExitCode::FAILURE;
    };

    let setup = format!(
        "{} {}{}",
        match options.facing {
            Facing::Front => "frontal",
            Facing::Back => "trasera",
        },
        match (options.saver, options.quality) {
            // En ahorro el tamaño depende del teléfono: se anota el que se usó.
            (true, _) => format!(
                "ahorro ({})",
                capture.as_ref().map(|c| c.describe()).unwrap_or_else(|| "tamaño desconocido".into())
            ),
            (false, Quality::Hd) => "720p".into(),
            (false, Quality::FullHd) => "1080p".into(),
        },
        if options.mic { " + micro" } else { "" },
    );
    let setup = if options.screen_off { setup } else { format!("{setup} + pantalla prendida") };
    let setup = match capture.as_ref().and_then(|c| c.camera_id.as_ref()) {
        Some(id) => format!("{setup}, lente {id}"),
        None => setup,
    };
    let by = match link {
        Link::Usb => "USB",
        Link::Wifi => "WiFi",
    };
    println!("Midiendo {minutes} min: {model}, {setup}, por {by}. Ctrl+C para terminar antes.\n");
    println!("  min  batería    temp  carga      CPU  memoria");

    let started = std::time::Instant::now();
    let mut samples: Vec<measure::Sample> = Vec::new();
    let mut last = (started, measure::usage());
    let mut problem = None;
    loop {
        let now = std::time::Instant::now();
        let usage = measure::usage();
        let elapsed = now.duration_since(last.0).as_secs_f64();
        let sample = measure::Sample {
            minute: now.duration_since(started).as_secs_f64() / 60.0,
            battery: measure::battery(&serial),
            cpu: (elapsed > 0.5).then(|| (usage.cpu_seconds - last.1.cpu_seconds).max(0.0) / elapsed * 100.0),
            memory_mb: usage.memory_mb,
        };
        last = (now, usage);
        let cpu = sample.cpu.map(|cpu| format!("{cpu:5.1} %")).unwrap_or_else(|| "     --".into());
        match sample.battery {
            Some(battery) => println!(
                "{:5.1}    {:3} %  {:4.1}°  {:5}  {cpu}  {:4.0} MB",
                sample.minute,
                battery.level,
                battery.temperature,
                if battery.charging { "sí" } else { "no" },
                sample.memory_mb,
            ),
            None => println!("{:5.1}      -- %    --°     --  {cpu}  {:4.0} MB", sample.minute, sample.memory_mb),
        }
        samples.push(sample);
        if sample.minute >= minutes {
            break;
        }
        // Espera a la siguiente muestra, atento a Ctrl+C y a que la cámara siga.
        let next = now + every.min(Duration::from_secs_f64((minutes - sample.minute).max(0.01) * 60.0));
        while std::time::Instant::now() < next {
            thread::sleep(Duration::from_millis(150));
            if interrupted.load(Ordering::SeqCst) {
                break;
            }
            let state = engine.snapshot();
            if state.phase == Phase::Off {
                problem = Some(state.error.unwrap_or_else(|| "La cámara se apagó.".into()));
                break;
            }
        }
        if interrupted.load(Ordering::SeqCst) || problem.is_some() {
            break;
        }
    }
    engine.shutdown();
    if let Some(problem) = &problem {
        eprintln!("\n{problem}");
    }

    let Some(summary) = measure::summarize(&samples) else {
        eprintln!("\nNo hubo tiempo para medir nada.");
        return ExitCode::FAILURE;
    };
    let signed = |value: f64| format!("{value:+.1}");
    let battery = summary.battery_per_hour.map(signed).unwrap_or_else(|| "--".into());
    let temperature = summary
        .temperature_change
        .map(|change| signed(f64::from(change)))
        .unwrap_or_else(|| "--".into());
    println!(
        "\nResumen de {:.1} min: batería {battery} puntos por hora, temperatura {temperature}°, CPU media {:.1} % de un núcleo, memoria máxima {:.0} MB.",
        summary.minutes, summary.cpu_average, summary.memory_max
    );
    if summary.charging {
        println!("El teléfono se estuvo cargando, así que la batería no refleja el gasto. Mide por WiFi para verlo.");
    }
    if summary.minutes < 10.0 {
        println!("Con menos de 10 minutos la batería casi no cambia; para comparar, mide 15 o más.");
    }
    let date = sys::run("date", &["+%F"]).map(|d| d.trim().to_string()).unwrap_or_default();
    println!("\nFila para docs/mediciones.md:");
    println!(
        "| {date} | {model} | {by}{} | {setup} | {:.1} | {battery} | {temperature} | {:.1} | {:.0} |",
        if summary.charging { " (cargando)" } else { "" },
        summary.minutes,
        summary.cpu_average,
        summary.memory_max
    );
    if problem.is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn start(flags: &[String]) -> ExitCode {
    let options = match read_options(flags) {
        Ok(options) => options,
        Err(code) => return code,
    };

    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let _ = ctrlc::set_handler(move || interrupted.store(true, Ordering::SeqCst));
    }

    let engine = Engine::spawn();
    engine.start(options);
    println!("Conectando con el teléfono...");

    let mut announced = false;
    let mut warned = false;
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
            // Se avisa una vez por sesión si al teléfono le queda poca batería.
            Phase::Live if !warned && state.battery.is_some_and(|b| b.low()) => {
                warned = true;
                let level = state.battery.map(|b| b.level).unwrap_or_default();
                println!("Al teléfono le queda {level} % de batería. Conéctalo con el cable USB para que se cargue.");
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
