//! La ventana de Ixtli.

use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iced::widget::{
    button, column, container, horizontal_space, image, row, scrollable, text, toggler, Space,
};
use iced::{
    border, color, font, window, Alignment, Background, Color, ContentFit, Element, Font, Length,
    Size, Subscription, Task, Theme,
};

use crate::audio;
use crate::config::{self, Facing, Options, Quality, Rotation};
use crate::engine::{Engine, Phase, Snapshot};
use crate::preview::Preview;
use crate::sys::{self, Phone, Tool};

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

/// Aviso periódico para redibujar la vista previa (unas 15 veces por segundo).
fn frames() -> Subscription<Message> {
    iced::time::every(Duration::from_millis(66)).map(|_| Message::Frame)
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
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Frame,
    Power,
    Rotation(Rotation),
    Detach,
    Attach,
    Facing(Facing),
    Quality(Quality),
    Mic(bool),
    Preview(bool),
    LoadModule,
    RetryPhone,
    CopyInstall,
}

/// Qué tan bien está cada requisito.
#[derive(Clone, Copy)]
enum Level {
    Good,
    Pending,
    Bad,
}

impl Level {
    fn color(self) -> Color {
        match self {
            Level::Good => JADE,
            Level::Pending => AMBER,
            Level::Bad => RED,
        }
    }
}

impl App {
    fn new(engine: Engine) -> App {
        App {
            state: engine.snapshot(),
            engine,
            options: config::load(),
            copied: false,
            preview: Preview::spawn(),
            picture: Picture::default(),
            viewer: None,
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
        match message {
            Message::Tick | Message::Frame => {}
            Message::Rotation(rotation) => {
                self.options.rotation = rotation;
                self.options_changed();
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
                self.options.facing = facing;
                self.options_changed();
            }
            Message::Quality(quality) => {
                self.options.quality = quality;
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
            Message::CopyInstall => {
                self.copied = true;
                return iced::clipboard::write(self.state.deps.install_command());
            }
        }
        self.state = self.engine.snapshot();
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

        let body = row![
            dot(color, 14.0),
            column![
                text(title).size(20).font(BOLD),
                text(detail).size(13).color(MUTED),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(14)
        .align_y(Alignment::Center);

        card(body)
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

        let phone = match &state.phone {
            Phone::Ready { model, .. } => requirement(Level::Good, "Teléfono", format!("{model} conectado por USB."), None),
            other => {
                let retry = (!matches!(other, Phone::Missing)).then_some(("Reintentar", Message::RetryPhone));
                requirement(Level::Pending, "Teléfono", other.advice().to_string(), retry)
            }
        };

        let cam = match &state.cam {
            Some(cam) => requirement(
                Level::Good,
                "Cámara virtual",
                format!("Lista. Aparece como «{}».", cam.label),
                None,
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

        let mut list = column![phone, cam].spacing(14);

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

    fn options_card(&self) -> Element<'_, Message> {
        let options = &self.options;
        let list = column![
            option_row(
                "Cámara",
                segmented(
                    &[(Facing::Front, "Frontal"), (Facing::Back, "Trasera")],
                    options.facing,
                    Message::Facing,
                ),
            ),
            option_row(
                "Calidad",
                segmented(
                    &[(Quality::Hd, "720p"), (Quality::FullHd, "1080p")],
                    options.quality,
                    Message::Quality,
                ),
            ),
            option_row(
                "Giro",
                segmented(
                    &[
                        (Rotation::Auto, "Auto"),
                        (Rotation::Fixed(0), "0°"),
                        (Rotation::Fixed(90), "90°"),
                        (Rotation::Fixed(180), "180°"),
                        (Rotation::Fixed(270), "270°"),
                    ],
                    options.rotation,
                    Message::Rotation,
                ),
            ),
            option_row(
                "Usar el micrófono del celular",
                toggler(options.mic).on_toggle(Message::Mic).into(),
            ),
            option_row(
                "Mostrar vista previa",
                toggler(options.preview).on_toggle(Message::Preview).into(),
            ),
        ]
        .spacing(14)
        .push_maybe((options.rotation == Rotation::Auto).then(|| {
            text("El giro automático sigue la posición del teléfono. Activa el giro automático de pantalla en el teléfono y deja la pantalla encendida.")
                .size(12)
                .color(MUTED)
        }));
        card(list)
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
    let mut line = row![
        dot(level.color(), 10.0),
        column![
            text(title).size(15).font(BOLD),
            text(detail).size(13).color(MUTED),
        ]
        .spacing(3)
        .width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    if let Some((label, message)) = action {
        line = line.push(small_button(label, message));
    }
    line.into()
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
) -> Element<'a, Message> {
    let mut buttons = row![].spacing(4);
    for &(value, label) in choices {
        let selected = value == current;
        buttons = buttons.push(
            button(text(label).size(13))
                .padding([6, 10])
                .on_press(on_pick(value))
                .style(move |_, status| {
                    let (background, text_color) = match (selected, status) {
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
