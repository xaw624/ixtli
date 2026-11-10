//! Ixtli: usa tu celular Android como cámara web y micrófono en Linux.
//!
//! Por debajo automatiza scrcpy (captura la cámara y el micrófono del
//! teléfono), v4l2loopback (cámara virtual) y PipeWire/PulseAudio
//! (micrófono virtual).

mod audio;
mod cli;
mod config;
mod engine;
mod pairing;
mod gui;
mod preview;
mod session;
mod sys;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    cli::run(&args)
}
