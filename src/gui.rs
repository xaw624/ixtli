//! La ventana de Ixtli.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iced::widget::{
    button, column, container, horizontal_space, image, row, scrollable, slider, text,
    text_input, toggler, Space,
};
use iced::{
    border, color, font, window, Alignment, Background, Color, ContentFit, Element, Font, Length,
    Size, Subscription, Task, Theme,
};

use crate::audio;
use crate::config::{self, Facing, Options, Quality, Rotation};
use crate::engine::{Engine, Phase, Snapshot};
use crate::measure::Battery;
use crate::pairing::{self, Progress};
use crate::preview::Preview;
use crate::relay;
use crate::sys::{self, Link, Phone, Tool};

// Paleta: fondo oscuro con acento verde jade.
const BACKGROUND: Color = color!(0x12141a);
const CARD: Color = color!(0x1c1f27);
const INSET: Color = color!(0x12141a);
const LINE: Color = color!(0x2a2e39);
const TEXT: Color = color!(0xe8eaed);
const MUTED: Color = color!(0x9aa1ad);
const JADE: Color = color!(0x2ec4a6);
const AMBER: Color = color!(0xf5a524);
const RED: Color = color!(0xe5484d);
const GRAY: Color = color!(0x5b6270);
const ON_ACCENT: Color = color!(0x06231d);

const ICON_RIGHT: &[u8] = include_bytes!("../assets/iconos/girar-derecha.svg");
const ICON_LEFT: &[u8] = include_bytes!("../assets/iconos/girar-izquierda.svg");
const ICON_MIRROR: &[u8] = include_bytes!("../assets/iconos/espejo.svg");

const BOLD: Font = Font {
    weight: font::Weight::Bold,
    ..Font::DEFAULT
};

/// Abre la ventana principal. Con `autostart` enciende la cámara al abrir.
pub fn run(autostart: bool) -> iced::Result {
    let engine = Engine::spawn();
    let for_window = engine.clone();
    // Si el sistema cierra Ixtli (apagar la sesión, `kill`), también se
    // apaga la cámara y se quita el micrófono virtual.
    {
        let engine = engine.clone();
        let _ = ctrlc::set_handler(move || {
            engine.shutdown();
            std::process::exit(0);
        });
    }
    let result = iced::application("Ixtli", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window(window::Settings {
            size: Size::new(440.0, 780.0),
            min_size: Some(Size::new(380.0, 480.0)),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "ixtli".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run_with(move || {
            let app = App::new(for_window);
            if autostart {
                app.engine.start(app.options);
            }
            (app, Task::none())
        });
    // Al cerrar la ventana se apaga la cámara y se quita el micrófono virtual.
    engine.shutdown();
    result
}

/// La ventana suelta de la vista previa (`ixtli vista`).
pub fn run_viewer() -> iced::Result {
    iced::application("Ixtli - vista previa", Viewer::update, Viewer::view)
        .subscription(|_| frames())
        .theme(|_| theme())
        .window(window::Settings {
            size: Size::new(480.0, 300.0),
            min_size: Some(Size::new(200.0, 150.0)),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "ixtli".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run_with(|| {
            let viewer = Viewer {
                preview: Preview::spawn(),
                picture: Picture::default(),
            };
            viewer.preview.set_wanted(true);
            (viewer, Task::none())
        })
}

fn theme() -> Theme {
    Theme::custom(
        "Ixtli".to_string(),
        iced::theme::Palette {
            background: BACKGROUND,
            text: TEXT,
            primary: JADE,
            success: JADE,
            danger: RED,
        },
    )
}

/// Aviso periódico para redibujar la vista previa (unas 10 veces por segundo).
fn frames() -> Subscription<Message> {
    iced::time::every(relay::PREVIEW_EVERY).map(|_| Message::Frame)
}

/// La última imagen de la cámara, lista para dibujarse.
#[derive(Default)]
struct Picture {
    number: u64,
    handle: Option<image::Handle>,
}

impl Picture {
    fn refresh(&mut self, preview: &Preview) {
        match preview.latest() {
            Some(frame) if frame.number != self.number || self.handle.is_none() => {
                self.number = frame.number;
                self.handle = Some(image::Handle::from_rgba(
                    frame.width,
                    frame.height,
                    // `Bytes`: se comparte, no se copia.
                    frame.rgba.clone(),
                ));
            }
            Some(_) => {}
            None => self.handle = None,
        }
    }
}

struct Viewer {
    preview: Preview,
    picture: Picture,
}

impl Viewer {
    fn update(&mut self, _message: Message) -> Task<Message> {
        self.picture.refresh(&self.preview);
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match &self.picture.handle {
            Some(handle) => image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                .into(),
            None => text("Sin imagen. Enciende la cámara en Ixtli.")
                .size(14)
                .color(MUTED)
                .into(),
        };
        container(content)
            .center(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::BLACK)),
                ..container::Style::default()
            })
            .into()
    }
}

struct App {
    engine: Engine,
    state: Snapshot,
    options: Options,
    copied: bool,
    preview: Preview,
    picture: Picture,
    /// La ventana suelta de la vista previa, si está desacoplada.
    viewer: Option<Child>,
    /// Ángulo que marca el control mientras se arrastra.
    angle_draft: i16,
    icons: Icons,
    /// El código QR para vincular, ya dibujado, con el texto que lleva.
    qr: Option<(String, image::Handle)>,
    /// Formulario de vinculación con código: dirección, código y, si hace
    /// falta, la dirección de conexión.
    pair_addr: String,
    pair_code: String,
    connect_addr: String,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Frame,
    Power,
    AutoRotation,
    Rotate { clockwise: bool },
    Mirror,
    /// El control del ángulo se está moviendo (todavía no se aplica).
    AngleDrag(i16),
    AngleApply,
    AngleReset,
    Detach,
    Attach,
    Facing(Facing),
    Quality(Quality),
    Saver(bool),
    /// Otro lente del mismo lado, por su número de cámara (`None`, el principal).
    Lens(Option<u16>),
    Mic(bool),
    Preview(bool),
    LoadModule,
    RetryPhone,
    WifiConnect,
    WifiForget,
    PairStart,
    PairCancel,
    /// Lo que se escribe en el formulario de vinculación con código.
    PairAddrInput(String),
    PairCodeInput(String),
    PairCodeSubmit,
    ConnectAddrInput(String),
    ConnectAddrSubmit,
    CopyInstall,
}

/// Qué tan bien está cada requisito.
#[derive(Clone, Copy)]
enum Level {
    Good,
    Pending,
    Bad,
    /// Algo opcional que no hace falta para encender.
    Optional,
}

impl Level {
    fn color(self) -> Color {
        match self {
            Level::Good => JADE,
            Level::Pending => AMBER,
            Level::Bad => RED,
            Level::Optional => GRAY,
        }
    }
}

impl App {
    fn new(engine: Engine) -> App {
        let options = config::load();
        App {
            state: engine.snapshot(),
            engine,
            angle_draft: options.angle,
            icons: Icons::load(),
            options,
            copied: false,
            preview: Preview::spawn(),
            picture: Picture::default(),
            viewer: None,
            qr: None,
            pair_addr: String::new(),
            pair_code: String::new(),
            connect_addr: String::new(),
        }
    }

    fn theme(&self) -> Theme {
        theme()
    }

    /// La vista previa se dibuja dentro de esta ventana.
    fn preview_embedded(&self) -> bool {
        self.options.preview && self.viewer.is_none() && self.state.phase != Phase::Off
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = iced::time::every(Duration::from_millis(300)).map(|_| Message::Tick);
        if self.preview_embedded() {
            Subscription::batch([tick, frames()])
        } else {
            tick
        }
    }

    fn close_viewer(&mut self) {
        if let Some(mut viewer) = self.viewer.take() {
            let _ = viewer.kill();
            let _ = viewer.wait();
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        // Mientras otro programa usa la cámara no se puede cambiar su tamaño.
        let blocked = match &message {
            Message::Quality(_) => self.quality_locked(),
            Message::Facing(_)
            | Message::Lens(_)
            | Message::Saver(_)
            | Message::AutoRotation
            | Message::Rotate { .. }
            | Message::AngleApply
            | Message::AngleReset => self.image_locked(),
            _ => false,
        };
        if blocked {
            self.angle_draft = self.options.angle;
            return Task::none();
        }
        match message {
            Message::Tick | Message::Frame => {}
            Message::AutoRotation => {
                self.options.rotation = match self.options.rotation {
                    Rotation::Auto => Rotation::Fixed(0),
                    Rotation::Fixed(_) => Rotation::Auto,
                };
                self.options_changed();
            }
            Message::Rotate { clockwise } => {
                self.options.rotation = self.options.rotation.turned(clockwise);
                self.options_changed();
            }
            Message::Mirror => {
                self.options.mirror = !self.options.mirror;
                self.options_changed();
            }
            Message::AngleDrag(angle) => self.angle_draft = config::clamp_angle(angle),
            Message::AngleApply => {
                if self.options.angle != self.angle_draft {
                    self.options.angle = self.angle_draft;
                    self.options_changed();
                }
            }
            Message::AngleReset => {
                self.angle_draft = 0;
                if self.options.angle != 0 {
                    self.options.angle = 0;
                    self.options_changed();
                }
            }
            Message::Detach => {
                if let Ok(program) = std::env::current_exe() {
                    let mut command = Command::new(program);
                    command.arg("vista").stdin(Stdio::null());
                    // Si Ixtli se cierra, la ventana suelta se cierra con él.
                    unsafe {
                        command.pre_exec(|| {
                            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                            Ok(())
                        });
                    }
                    self.viewer = command.spawn().ok();
                }
            }
            Message::Attach => self.close_viewer(),
            Message::Power => {
                if self.state.phase == Phase::Off {
                    self.engine.start(self.options);
                } else {
                    self.engine.stop();
                }
            }
            Message::Facing(facing) => {
                // Los lentes son de un lado: al cambiar de lado se vuelve al principal.
                if self.options.facing != facing {
                    self.options.lens = None;
                }
                self.options.facing = facing;
                self.options_changed();
            }
            Message::Quality(quality) => {
                self.options.quality = quality;
                self.options_changed();
            }
            Message::Lens(lens) => {
                if self.options.lens != lens {
                    self.options.lens = lens;
                    self.options_changed();
                }
            }
            Message::Saver(saver) => {
                self.options.saver = saver;
                self.options_changed();
            }
            Message::Mic(mic) => {
                self.options.mic = mic;
                self.options_changed();
            }
            Message::Preview(preview) => {
                self.options.preview = preview;
                self.options_changed();
            }
            Message::LoadModule => self.engine.load_module(),
            Message::RetryPhone => self.engine.restart_adb(),
            Message::WifiConnect => self.engine.wifi_connect(),
            Message::WifiForget => self.engine.wifi_forget(),
            Message::PairStart => {
                self.pair_code.clear();
                self.connect_addr.clear();
                self.engine.pair_start();
            }
            Message::PairCancel => {
                self.pair_addr.clear();
                self.pair_code.clear();
                self.connect_addr.clear();
                self.engine.pair_cancel();
            }
            Message::PairAddrInput(value) => self.pair_addr = value.trim().to_string(),
            Message::PairCodeInput(value) => {
                self.pair_code = value.chars().filter(char::is_ascii_digit).take(6).collect()
            }
            Message::PairCodeSubmit => {
                if self.code_ready() {
                    self.engine.pair_code(self.pair_addr.clone(), self.pair_code.clone());
                }
            }
            Message::ConnectAddrInput(value) => self.connect_addr = value.trim().to_string(),
            Message::ConnectAddrSubmit => {
                if sys::valid_addr(&self.connect_addr) {
                    self.engine.pair_address(self.connect_addr.clone());
                }
            }
            Message::CopyInstall => {
                self.copied = true;
                return iced::clipboard::write(self.state.deps.install_command());
            }
        }
        self.state = self.engine.snapshot();
        // Si se vio en la red un teléfono esperando código, se propone su dirección.
        if self.pair_addr.is_empty() {
            if let Some(found) = self.state.wifi.pairing.as_ref().and_then(|view| view.suggestion.clone()) {
                self.pair_addr = found;
            }
        }
        // El código QR se dibuja solo cuando cambia.
        let wanted = self.state.wifi.pairing.as_ref().and_then(|view| view.qr.as_deref());
        if wanted != self.qr.as_ref().map(|(text, _)| text.as_str()) {
            self.qr = wanted.and_then(|text| Some((text.to_string(), render_qr(text)?)));
        }
        // Si cerraron la ventana suelta, la vista previa regresa a esta.
        if let Some(viewer) = &mut self.viewer {
            if !matches!(viewer.try_wait(), Ok(None)) {
                self.viewer = None;
            }
        }
        self.preview.set_wanted(self.preview_embedded());
        self.picture.refresh(&self.preview);
        Task::none()
    }

    /// Guarda las opciones y, si la cámara está encendida, la reinicia con ellas.
    fn options_changed(&mut self) {
        config::save(&self.options);
        if self.state.phase != Phase::Off {
            self.engine.start(self.options);
        }
    }

    /// La dirección y el código del formulario tienen la forma correcta.
    fn code_ready(&self) -> bool {
        sys::valid_addr(&self.pair_addr) && self.pair_code.len() == 6
    }

    /// Otro programa (una videollamada) tiene abierta la cámara «Ixtli», y la
    /// cámara virtual no deja cambiar su tamaño mientras tanto.
    fn size_locked(&self) -> bool {
        !self.state.cam_users.is_empty()
    }

    /// La calidad cambia el tamaño de «Ixtli».
    fn quality_locked(&self) -> bool {
        self.size_locked()
    }

    /// Sin la cámara interna (módulo viejo), la cámara, el modo ahorro, el
    /// lente y el giro también cambian el tamaño.
    fn image_locked(&self) -> bool {
        self.size_locked() && !self.state.fixed_size
    }

    /// Aviso de por qué hay opciones bloqueadas.
    fn lock_note(&self) -> Option<Element<'_, Message>> {
        if !self.size_locked() {
            return None;
        }
        let names: Vec<&str> = self.state.cam_users.iter().map(|name| program_name(name)).collect();
        let what = if self.state.fixed_size { "la calidad" } else { "la cámara, la calidad, el modo ahorro o el giro" };
        Some(
            text(format!(
                "{} está usando la cámara «Ixtli». Para cambiar {what}, apaga la cámara en la videollamada.",
                names.join(" y ")
            ))
            .size(13)
            .color(AMBER)
            .into(),
        )
    }

    fn can_start(&self) -> bool {
        self.state.checked
            && self.state.deps.problem(self.options.mic).is_none()
            && matches!(self.state.phone, Phone::Ready { .. })
    }

    fn view(&self) -> Element<'_, Message> {
        let header = column![
            text("Ixtli").size(30).font(BOLD),
            text("Tu celular como cámara web y micrófono").size(14).color(MUTED),
        ]
        .spacing(2);

        let content = column![header, self.status_card()]
            .push_maybe(self.pairing_card())
            .push_maybe(self.preview_card())
            .push(self.requirements_card())
            .push(self.options_card())
            .push(self.power_button())
            .push(text("Funciona con scrcpy y v4l2loopback.").size(11).color(GRAY))
            .spacing(14)
            .padding(20)
            .max_width(560);

        container(scrollable(content))
            .center_x(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn status_card(&self) -> Element<'_, Message> {
        let state = &self.state;
        let (color, title, detail) = match state.phase {
            Phase::Live => {
                let cam = state
                    .cam
                    .as_ref()
                    .map(|cam| cam.label.as_str())
                    .unwrap_or(sys::CAM_LABEL);
                let mut detail = format!("En tu videollamada elige la cámara «{cam}»");
                if state.mic {
                    detail.push_str(&format!(" y el micrófono «{}»", audio::MIC_LABEL));
                }
                detail.push('.');
                if state.screen_off {
                    detail.push_str(" La pantalla del teléfono se apagó para gastar menos batería.");
                }
                if state.own_motor {
                    detail.push_str(" Motor propio (en prueba).");
                }
                (JADE, "En vivo", detail)
            }
            Phase::Starting => (
                AMBER,
                "Conectando…",
                "Abriendo la cámara del teléfono.".to_string(),
            ),
            Phase::Off => match &state.error {
                Some(problem) => (RED, "No se pudo encender", problem.clone()),
                None if !state.checked => (GRAY, "Revisando…", "Buscando tu teléfono.".to_string()),
                None if self.can_start() => (
                    GRAY,
                    "Apagado",
                    "Todo listo. Presiona «Encender» cuando quieras.".to_string(),
                ),
                None => (
                    GRAY,
                    "Apagado",
                    "Completa lo que falta en la lista de abajo.".to_string(),
                ),
            },
        };

        let mut words = column![
            text(title).size(20).font(BOLD),
            text(detail).size(13).color(MUTED),
        ]
        .spacing(4)
        .width(Length::Fill);
        if let Some(warning) = self.battery_warning() {
            words = words.push(text(warning).size(13).color(AMBER));
        }
        let body = row![dot(color, 14.0), words]
        .spacing(14)
        .align_y(Alignment::Center);

        card(body)
    }

    /// Aviso de batería baja, solo si el teléfono no se está cargando.
    fn battery_warning(&self) -> Option<String> {
        let battery = self.state.battery.filter(Battery::low)?;
        let Phone::Ready { link, .. } = &self.state.phone else { return None };
        Some(match link {
            Link::Wifi => format!("Al teléfono le queda {} % de batería. Conéctalo con el cable USB para que se cargue.", battery.level),
            Link::Usb => format!("Al teléfono le queda {} % de batería y este cable no lo está cargando. Conéctalo a un cargador.", battery.level),
        })
    }

    /// La imagen de la cámara, o el aviso de que está en otra ventana.
    fn preview_card(&self) -> Option<Element<'_, Message>> {
        if !self.options.preview || self.state.phase == Phase::Off {
            return None;
        }
        let note = |message: &'static str| -> Element<'_, Message> {
            container(text(message).size(13).color(MUTED))
                .center_x(Length::Fill)
                .padding([24, 0])
                .into()
        };
        let (picture, action) = if self.viewer.is_some() {
            (
                note("La vista previa está en otra ventana."),
                small_button("Acoplar", Message::Attach),
            )
        } else {
            let picture = match &self.picture.handle {
                Some(handle) => image(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fixed(230.0))
                    .content_fit(ContentFit::Contain)
                    .into(),
                None if !self.state.fixed_size => note("Para ver la vista previa, actualiza la cámara virtual."),
                None => note("Esperando la imagen…"),
            };
            (picture, small_button("Desacoplar", Message::Detach))
        };
        let footer = row![
            text("Vista previa").size(13).color(MUTED),
            horizontal_space(),
            action,
        ]
        .align_y(Alignment::Center);
        Some(card(column![picture, footer].spacing(12)))
    }

    fn requirements_card(&self) -> Element<'_, Message> {
        let state = &self.state;
        let live = state.phase != Phase::Off;

        let phone = match (&state.phone, &state.wifi.remembered) {
            (Phone::Ready { model, link, .. }, _) => {
                let by = match link {
                    Link::Usb => "por USB",
                    Link::Wifi => "por WiFi",
                };
                let mut detail = format!("{model} conectado {by}.");
                let mut level = Level::Good;
                if let Some(battery) = &state.battery {
                    detail.push_str(&format!(" Batería: {} %", battery.level));
                    if battery.charging {
                        detail.push_str(", cargando");
                    }
                    detail.push('.');
                    if battery.low() {
                        level = Level::Pending;
                    }
                }
                requirement(level, "Teléfono", detail, None)
            }
            (Phone::Missing, Some(remembered)) => requirement(
                Level::Pending,
                "Teléfono",
                format!("Buscando tu {} por WiFi. También puedes conectarlo por USB.", remembered.model),
                None,
            ),
            (other, _) => {
                let retry = (!matches!(other, Phone::Missing)).then_some(("Reintentar", Message::RetryPhone));
                requirement(Level::Pending, "Teléfono", other.advice().to_string(), retry)
            }
        };

        let cam = match &state.cam {
            Some(cam) if state.fixed_size => requirement(
                Level::Good,
                "Cámara virtual",
                format!("Lista. Aparece como «{}».", cam.label),
                None,
            ),
            // Módulo cargado por una versión vieja, sin la cámara interna.
            Some(cam) => requirement(
                Level::Pending,
                "Cámara virtual",
                format!(
                    "Aparece como «{}», pero hay que actualizarla para que las videollamadas no la pierdan al cambiar algo (te pedirá tu contraseña; cierra antes las videollamadas).",
                    cam.label
                ),
                (!live).then_some(("Actualizar", Message::LoadModule)),
            ),
            None if state.deps.module => requirement(
                Level::Pending,
                "Cámara virtual",
                "Apagada. Se activa al encender (te pedirá tu contraseña).".to_string(),
                (!live).then_some(("Activar", Message::LoadModule)),
            ),
            None => requirement(
                Level::Bad,
                "Cámara virtual",
                "Falta instalar v4l2loopback.".to_string(),
                None,
            ),
        };

        let mut list = column![phone].push_maybe(self.wifi_row()).push(cam).spacing(14);

        let missing = state.deps.missing_packages();
        if let Tool::TooOld(version) = &state.deps.scrcpy {
            list = list.push(requirement(
                Level::Bad,
                "Programas",
                format!("Tu scrcpy ({version}) es muy viejo: se necesita la versión 3.0 o una más nueva."),
                None,
            ));
        } else if state.checked && !missing.is_empty() {
            let optional = state.deps.ready();
            let detail = if optional {
                format!("Para el micrófono falta instalar: {}.", missing.join(", "))
            } else {
                format!("Falta instalar: {}.", missing.join(", "))
            };
            list = list.push(requirement(
                if optional { Level::Pending } else { Level::Bad },
                "Programas",
                detail,
                Some((if self.copied { "Copiado" } else { "Copiar comando" }, Message::CopyInstall)),
            ));
            list = list.push(
                container(text(state.deps.install_command()).size(12).font(Font::MONOSPACE).color(MUTED))
                    .padding(10)
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(INSET)),
                        border: border::rounded(8),
                        ..container::Style::default()
                    }),
            );
        }

        card(list)
    }

    /// La conexión sin cable: activarla, ver si está conectada y olvidarla.
    fn wifi_row(&self) -> Option<Element<'_, Message>> {
        let state = &self.state;
        let wifi = &state.wifi;
        // Mientras la cámara está encendida no se cambia la conexión.
        let idle = state.phase == Phase::Off;
        let on_usb = matches!(state.phone, Phone::Ready { link: Link::Usb, .. });
        let connect = (idle && on_usb).then_some(("Conectar por WiFi", Message::WifiConnect));
        let pair = (idle && wifi.pairing.is_none()).then_some(("Vincular con QR", Message::PairStart));
        let forget = idle.then_some(("Olvidar", Message::WifiForget));

        if wifi.busy {
            return Some(requirement(
                Level::Pending,
                "WiFi",
                "Pasando el teléfono a WiFi…".to_string(),
                None,
            ));
        }
        let row = match &wifi.remembered {
            Some(remembered) if wifi.connected => {
                let place = if remembered.paired {
                    "Vinculado".to_string()
                } else {
                    format!("Listo en {}", remembered.addr)
                };
                let detail = match (on_usb, idle) {
                    (true, true) => format!("{place}. Ya puedes desconectar el cable."),
                    (true, false) => {
                        format!("{place}. Si desconectas el cable, la cámara sigue por WiFi.")
                    }
                    (false, _) if remembered.paired => "Conectado sin cable.".to_string(),
                    (false, _) => format!("Conectado sin cable a {}.", remembered.addr),
                };
                requirement(Level::Good, "WiFi", detail, forget)
            }
            Some(remembered) => {
                let detail = wifi.problem.clone().unwrap_or_else(|| {
                    "Recuerdo tu teléfono; me conecto en cuanto esté en la red.".to_string()
                });
                // Al vinculado no le sirve el cable; al otro, el QR lo libera del cable.
                let fix = if remembered.paired { pair } else { connect.or(pair) };
                requirement_many(Level::Pending, "WiFi", detail, fix.into_iter().chain(forget).collect())
            }
            None => {
                let (level, detail) = match &wifi.problem {
                    Some(problem) => (Level::Bad, problem.clone()),
                    None => (
                        Level::Optional,
                        "Úsalo sin cable. El teléfono debe estar en la misma red WiFi que esta computadora."
                            .to_string(),
                    ),
                };
                requirement_many(level, "WiFi", detail, pair.into_iter().chain(connect).collect())
            }
        };
        Some(row)
    }

    /// La tarjeta para vincular el teléfono: código QR, código de 6 dígitos
    /// y, si no aparece en la red, su dirección.
    fn pairing_card(&self) -> Option<Element<'_, Message>> {
        let view = self.state.wifi.pairing.as_ref()?;
        let (color, status) = match &view.progress {
            Progress::Waiting => (AMBER, "Esperando al teléfono…".to_string()),
            Progress::Connecting => (AMBER, "Vinculando y conectando…".to_string()),
            Progress::NeedAddress(None) => (AMBER, "Vinculado. Falta su dirección.".to_string()),
            Progress::NeedAddress(Some(problem)) => (RED, problem.clone()),
            Progress::Done(phone) => (JADE, format!("Listo: {} conectado sin cable.", phone.model)),
            Progress::Failed(problem) => (RED, problem.clone()),
        };
        let failed = matches!(view.progress, Progress::Failed(_));
        let done = failed || matches!(view.progress, Progress::Done(_));

        let mut footer = row![dot(color, 10.0), text(status).size(13).width(Length::Fill)]
            .spacing(10)
            .align_y(Alignment::Center);
        if failed {
            footer = footer.push(small_button("Otro código", Message::PairStart));
        }
        footer = footer.push(small_button(if done { "Cerrar" } else { "Cancelar" }, Message::PairCancel));

        let title = text("Vincular sin cable").size(17).font(BOLD);
        let note = |words: &'static str| text(words).size(13).color(MUTED);
        let mut body = column![].spacing(12);
        match (&view.progress, &self.qr) {
            (Progress::Waiting, Some((_, handle))) => {
                let code = container(
                    image(handle.clone())
                        .width(Length::Fixed(200.0))
                        .height(Length::Fixed(200.0))
                        .filter_method(image::FilterMethod::Nearest),
                )
                .center_x(Length::Fill);
                let form = row![
                    field("IP:puerto", &self.pair_addr, Message::PairAddrInput, Message::PairCodeSubmit)
                        .width(Length::FillPortion(3)),
                    field("Código", &self.pair_code, Message::PairCodeInput, Message::PairCodeSubmit)
                        .width(Length::FillPortion(2)),
                    form_button("Vincular", self.code_ready().then_some(Message::PairCodeSubmit)),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                body = body
                    .push(title)
                    .push(note("1. En el teléfono abre Opciones de desarrollador → Depuración inalámbrica y actívala."))
                    .push(note("2. Ahí mismo toca «Vincular dispositivo con código QR» y escanea este código con el lector que se abre."))
                    .push(
                        text("No uses la app de cámara: la cámara lo confunde con una red WiFi.")
                            .size(13)
                            .color(AMBER),
                    )
                    .push(code)
                    .push(note("¿No puedes escanearlo, o estás por VPN? Toca «Vincular dispositivo con código de vinculación» y escribe aquí la dirección y el código que muestra el teléfono:"))
                    .push(form);
            }
            (Progress::NeedAddress(_), _) => {
                let form = row![
                    field("IP:puerto", &self.connect_addr, Message::ConnectAddrInput, Message::ConnectAddrSubmit)
                        .width(Length::Fill),
                    form_button(
                        "Conectar",
                        sys::valid_addr(&self.connect_addr).then_some(Message::ConnectAddrSubmit),
                    ),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                body = body
                    .push(title)
                    .push(note("El teléfono ya quedó vinculado, pero no lo encuentro en la red (pasa por VPN o en redes que bloquean los anuncios)."))
                    .push(note("Escribe la «Dirección IP y puerto» que muestra la pantalla de Depuración inalámbrica. No es la misma de la vinculación."))
                    .push(form);
            }
            (Progress::Done(_), _) => {}
            _ => body = body.push(title),
        }
        Some(card(body.push(footer)))
    }

    fn options_card(&self) -> Element<'_, Message> {
        let options = &self.options;
        let facing = option_row(
            "Cámara",
            segmented(
                &[(Facing::Front, "Frontal"), (Facing::Back, "Trasera")],
                options.facing,
                Message::Facing,
                !self.image_locked(),
            ),
        );
        let quality = option_row(
            "Calidad",
            segmented(
                &[(Quality::Hd, "720p"), (Quality::FullHd, "1080p")],
                options.quality,
                Message::Quality,
                !self.quality_locked(),
            ),
        );
        let list = column![]
            .push_maybe(self.lock_note())
            .push(facing)
            .push_maybe(self.lens_row())
            .push(quality)
            .push(self.saver_row())
            .push(self.rotation_row())
            .push(self.angle_row())
            .push(option_row(
                "Usar el micrófono del celular",
                toggler(options.mic).on_toggle(Message::Mic).into(),
            ))
            .push(option_row(
                "Mostrar vista previa",
                toggler(options.preview).on_toggle(Message::Preview).into(),
            ))
            .spacing(14);
        card(list)
    }

    /// Lentes del lado elegido. Solo aparece si el teléfono tiene más de uno.
    fn lens_row(&self) -> Option<Element<'_, Message>> {
        let cameras = self.state.cameras.as_deref()?;
        let lenses: Vec<_> = config::lenses(cameras, self.options.facing)
            .into_iter()
            .filter_map(|camera| Some((camera.id.parse::<u16>().ok()?, camera)))
            .collect();
        if lenses.len() < 2 {
            return None;
        }
        // Si el lente guardado no existe en este teléfono, se usa el principal.
        let current = lenses.iter().position(|(id, _)| Some(*id) == self.options.lens).unwrap_or(0);
        let mut controls = row![].spacing(4);
        for (position, (id, _)) in lenses.iter().enumerate() {
            let (label, lens) = match position {
                0 => ("Principal".to_string(), None),
                // El mismo número que muestra «ixtli lentes».
                _ => (id.to_string(), Some(*id)),
            };
            controls = controls.push(choice_button(
                text(label).size(13).into(),
                position == current,
                Message::Lens(lens),
                !self.image_locked(),
            ));
        }
        let detail = match lenses[current].1.megapixels() {
            Some(mp) => format!("Sensor de {mp} MP. Prueba cuál es el gran angular."),
            None => "Prueba cuál es el gran angular.".to_string(),
        };
        Some(
            row![
                column![text("Lente").size(15), text(detail).size(12).color(MUTED)]
                    .spacing(2)
                    .width(Length::Fill),
                inset(controls),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        )
    }

    /// Modo ahorro. Encendido, dice qué tamaño se está usando.
    fn saver_row(&self) -> Element<'_, Message> {
        let detail = match (&self.state.capture, self.state.phase) {
            (Some(capture), Phase::Live) if self.options.saver && capture.bit_rate.is_some() => capture.describe(),
            _ if self.options.saver => "Cerca de 480p y 15 cuadros/s, en lugar de la calidad elegida".into(),
            _ => "Gasta menos batería, con menos calidad".into(),
        };
        row![
            column![text("Modo ahorro").size(15), text(detail).size(12).color(MUTED)]
                .spacing(2)
                .width(Length::Fill),
            toggler(self.options.saver).on_toggle_maybe((!self.image_locked()).then_some(Message::Saver)),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    /// Giro: automático, cuartos de vuelta a cada lado y efecto espejo.
    fn rotation_row(&self) -> Element<'_, Message> {
        let auto = self.options.rotation == Rotation::Auto;
        let current = match self.options.rotation {
            Rotation::Auto => "Automático".to_string(),
            Rotation::Fixed(degrees) => format!("{degrees}°"),
        };
        let controls = row![
            choice_button(text("Auto").size(13).into(), auto, Message::AutoRotation, !self.image_locked()),
            choice_button(icon(&self.icons.left), false, Message::Rotate { clockwise: false }, !self.image_locked()),
            choice_button(icon(&self.icons.right), false, Message::Rotate { clockwise: true }, !self.image_locked()),
            choice_button(
                icon(if self.options.mirror { &self.icons.mirror_on } else { &self.icons.mirror }),
                self.options.mirror,
                Message::Mirror,
                true,
            ),
        ]
        .spacing(4);
        row![
            column![text("Giro").size(15), text(current).size(12).color(MUTED)]
                .spacing(2)
                .width(Length::Fill),
            inset(controls),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
    }

    /// Giro extra a cualquier ángulo. Se aplica al soltar el control.
    fn angle_row(&self) -> Element<'_, Message> {
        let mut controls = row![
            slider(-180..=180, self.angle_draft, Message::AngleDrag)
                .on_release(Message::AngleApply)
                .width(Length::Fixed(150.0)),
            text(format!("{}°", self.angle_draft)).size(13).width(Length::Fixed(42.0)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if self.angle_draft != 0 {
            controls = controls.push(small_button("Quitar", Message::AngleReset));
        }
        row![text("Ángulo").size(15), horizontal_space(), controls]
            .spacing(12)
            .align_y(Alignment::Center)
            .into()
    }

    fn power_button(&self) -> Element<'_, Message> {
        let (label, color, text_color, enabled) = match self.state.phase {
            Phase::Off => ("Encender", JADE, ON_ACCENT, self.can_start()),
            Phase::Starting => ("Cancelar", LINE, TEXT, true),
            Phase::Live => ("Apagar", RED, Color::WHITE, true),
        };
        button(
            text(label)
                .size(17)
                .font(BOLD)
                .width(Length::Fill)
                .center(),
        )
        .width(Length::Fill)
        .padding(15)
        .on_press_maybe(enabled.then_some(Message::Power))
        .style(move |_, status| {
            let (background, text_color) = match status {
                button::Status::Active => (color, text_color),
                button::Status::Hovered => (mix(color, Color::WHITE, 0.12), text_color),
                button::Status::Pressed => (mix(color, Color::BLACK, 0.15), text_color),
                button::Status::Disabled => (LINE, GRAY),
            };
            button::Style {
                background: Some(Background::Color(background)),
                text_color,
                border: border::rounded(12),
                ..button::Style::default()
            }
        })
        .into()
    }
}

// ---------------------------------------------------------------------------
// Piezas de la interfaz
// ---------------------------------------------------------------------------

fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * amount,
        g: a.g + (b.g - a.g) * amount,
        b: a.b + (b.b - a.b) * amount,
        a: 1.0,
    }
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(18)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(CARD)),
            border: border::rounded(14).color(LINE).width(1),
            ..container::Style::default()
        })
        .into()
}

fn dot<'a>(color: Color, size: f32) -> Element<'a, Message> {
    container(Space::new(size, size))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: border::rounded(size / 2.0),
            ..container::Style::default()
        })
        .into()
}

/// Un renglón de la lista de requisitos, con un botón opcional.
fn requirement<'a>(
    level: Level,
    title: &'a str,
    detail: String,
    action: Option<(&'a str, Message)>,
) -> Element<'a, Message> {
    requirement_many(level, title, detail, action.into_iter().collect())
}

/// Un renglón de requisitos con varios botones: van debajo del texto.
fn requirement_many<'a>(
    level: Level,
    title: &'a str,
    detail: String,
    actions: Vec<(&'a str, Message)>,
) -> Element<'a, Message> {
    let mut words = column![
        text(title).size(15).font(BOLD),
        text(detail).size(13).color(MUTED),
    ]
    .spacing(3)
    .width(Length::Fill);
    let several = actions.len() > 1;
    if several {
        let mut buttons = row![].spacing(8);
        for (label, message) in actions.iter().cloned() {
            buttons = buttons.push(small_button(label, message));
        }
        words = words.push(Space::with_height(4)).push(buttons);
    }
    let mut line = row![dot(level.color(), 10.0), words]
        .spacing(12)
        .align_y(Alignment::Center);
    if let (false, Some((label, message))) = (several, actions.into_iter().next()) {
        line = line.push(small_button(label, message));
    }
    line.into()
}

/// Un campo de texto del formulario de vinculación.
fn field<'a>(
    hint: &'a str,
    value: &'a str,
    on_input: fn(String) -> Message,
    on_submit: Message,
) -> iced::widget::TextInput<'a, Message> {
    text_input(hint, value)
        .on_input(on_input)
        .on_submit(on_submit)
        .size(13)
        .padding(8)
        .font(Font::MONOSPACE)
}

/// Botón de un formulario; sin mensaje queda apagado.
fn form_button<'a>(label: &'a str, message: Option<Message>) -> Element<'a, Message> {
    button(text(label).size(13))
        .padding([8, 12])
        .on_press_maybe(message)
        .style(|_, status| {
            let (background, text_color) = match status {
                button::Status::Disabled => (LINE, GRAY),
                button::Status::Hovered => (mix(JADE, Color::WHITE, 0.12), ON_ACCENT),
                _ => (JADE, ON_ACCENT),
            };
            button::Style {
                background: Some(Background::Color(background)),
                text_color,
                border: border::rounded(8),
                ..button::Style::default()
            }
        })
        .into()
}

/// Dibuja el código QR en negro sobre blanco, con su margen.
fn render_qr(text: &str) -> Option<image::Handle> {
    let (width, dark) = pairing::qr_modules(text)?;
    const SCALE: usize = 8;
    const QUIET: usize = 3;
    let side = (width + QUIET * 2) * SCALE;
    let mut rgba = vec![255u8; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / SCALE, y / SCALE);
            let inside = (QUIET..QUIET + width).contains(&mx) && (QUIET..QUIET + width).contains(&my);
            if inside && dark[(my - QUIET) * width + (mx - QUIET)] {
                let at = (y * side + x) * 4;
                rgba[at..at + 3].fill(0);
            }
        }
    }
    Some(image::Handle::from_rgba(side as u32, side as u32, rgba))
}

fn small_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(13))
        .padding([7, 12])
        .on_press(message)
        .style(|_, status| {
            let background = match status {
                button::Status::Hovered => mix(LINE, Color::WHITE, 0.1),
                button::Status::Pressed => mix(LINE, Color::BLACK, 0.2),
                _ => LINE,
            };
            button::Style {
                background: Some(Background::Color(background)),
                text_color: TEXT,
                border: border::rounded(8),
                ..button::Style::default()
            }
        })
        .into()
}

/// Iconos de los botones de giro, ya dibujados en el color que les toca.
struct Icons {
    left: image::Handle,
    right: image::Handle,
    mirror: image::Handle,
    mirror_on: image::Handle,
}

impl Icons {
    fn load() -> Icons {
        Icons {
            left: render_icon(ICON_LEFT, MUTED),
            right: render_icon(ICON_RIGHT, MUTED),
            mirror: render_icon(ICON_MIRROR, MUTED),
            mirror_on: render_icon(ICON_MIRROR, ON_ACCENT),
        }
    }
}

/// Dibuja un icono SVG como imagen de un solo color. Se dibuja al cuádruple
/// del tamaño en que se muestra para que se vea nítido en cualquier pantalla.
fn render_icon(bytes: &[u8], color: Color) -> image::Handle {
    use resvg::{tiny_skia, usvg};
    const SIDE: u32 = 72;
    let mut pixmap = tiny_skia::Pixmap::new(SIDE, SIDE).expect("tamaño de icono válido");
    if let Ok(tree) = usvg::Tree::from_data(bytes, &usvg::Options::default()) {
        let scale = SIDE as f32 / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
    }
    // Del dibujo solo se usa la transparencia; el color lo pone Ixtli.
    let [red, green, blue, _] = color.into_rgba8();
    let rgba: Vec<u8> = pixmap
        .data()
        .chunks_exact(4)
        .flat_map(|pixel| [red, green, blue, pixel[3]])
        .collect();
    image::Handle::from_rgba(SIDE, SIDE, rgba)
}

fn icon<'a>(handle: &image::Handle) -> Element<'a, Message> {
    image(handle.clone())
        .width(Length::Fixed(18.0))
        .height(Length::Fixed(18.0))
        .into()
}

/// Botón de un grupo de opciones; `selected` lo pinta de jade.
/// Sin `enabled`, no responde y se ve apagado.
fn choice_button<'a>(content: Element<'a, Message>, selected: bool, message: Message, enabled: bool) -> Element<'a, Message> {
    button(content)
        .padding([6, 10])
        .on_press_maybe(enabled.then_some(message))
        .style(move |_, status| {
            let (background, text_color) = match (selected, status) {
                (true, button::Status::Disabled) => (LINE, MUTED),
                (true, _) => (JADE, ON_ACCENT),
                (false, button::Status::Hovered | button::Status::Pressed) => (LINE, TEXT),
                (false, _) => (INSET, MUTED),
            };
            button::Style {
                background: Some(Background::Color(background)),
                text_color,
                border: border::rounded(8),
                ..button::Style::default()
            }
        })
        .into()
}

/// Fondo hundido para agrupar botones.
fn inset<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(4)
        .style(|_| container::Style {
            background: Some(Background::Color(INSET)),
            border: border::rounded(11),
            ..container::Style::default()
        })
        .into()
}

fn option_row<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    row![text(label).size(15), horizontal_space(), control]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

/// Selector de dos o más opciones, donde una está marcada.
fn segmented<'a, T: Copy + PartialEq + 'a>(
    choices: &[(T, &'a str)],
    current: T,
    on_pick: impl Fn(T) -> Message,
    enabled: bool,
) -> Element<'a, Message> {
    let mut buttons = row![].spacing(4);
    for &(value, label) in choices {
        let selected = value == current;
        buttons = buttons.push(
            button(text(label).size(13))
                .padding([6, 10])
                .on_press_maybe(enabled.then(|| on_pick(value)))
                .style(move |_, status| {
                    let (background, text_color) = match (selected, status) {
                        (true, button::Status::Disabled) => (LINE, MUTED),
                        (true, _) => (JADE, ON_ACCENT),
                        (false, button::Status::Hovered) => (LINE, TEXT),
                        (false, _) => (INSET, MUTED),
                    };
                    button::Style {
                        background: Some(Background::Color(background)),
                        text_color,
                        border: border::rounded(8),
                        ..button::Style::default()
                    }
                }),
        );
    }
    container(buttons)
        .padding(4)
        .style(|_| container::Style {
            background: Some(Background::Color(INSET)),
            border: border::rounded(11),
            ..container::Style::default()
        })
        .into()
}

/// Nombre conocido de un programa a partir de su nombre de proceso.
fn program_name(process: &str) -> &str {
    match process {
        p if p.starts_with("firefox") => "Firefox",
        p if p.starts_with("chrom") || p.starts_with("google-chrome") => "Chrome",
        "brave" => "Brave",
        "obs" => "OBS",
        "zoom" | "ZoomWebviewHost" => "Zoom",
        p if p.starts_with("teams") => "Teams",
        other => other,
    }
}
