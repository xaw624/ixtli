//! Micrófono virtual con PipeWire/PulseAudio (vía `pactl`).
//!
//! scrcpy reproduce el audio del teléfono en una salida "nula" (no suena en
//! las bocinas) y un micrófono virtual toma lo que llega a esa salida. Así
//! las videollamadas ven un micrófono normal.

use crate::sys;

/// Salida interna a la que scrcpy manda el audio del teléfono.
pub const SINK: &str = "ixtli_sink";
const MIC: &str = "ixtli_mic";

const SINK_LABEL: &str = "Ixtli (enlace interno)";
/// Nombre con el que aparece el micrófono en las videollamadas.
pub const MIC_LABEL: &str = "Micrófono del celular (Ixtli)";

fn pactl(args: &[&str]) -> Result<String, String> {
    sys::run("pactl", args).map_err(|failure| failure.message)
}

/// Crea el micrófono virtual. Si ya existía uno de Ixtli, lo reemplaza.
pub fn create() -> Result<(), String> {
    remove();
    let error = |e: String| format!("No se pudo crear el micrófono virtual: {e}");
    pactl(&[
        "load-module",
        "module-null-sink",
        &format!("sink_name={SINK}"),
        &format!("sink_properties='device.description=\"{SINK_LABEL}\"'"),
    ])
    .map_err(error)?;
    let source = pactl(&[
        "load-module",
        "module-remap-source",
        &format!("master={SINK}.monitor"),
        &format!("source_name={MIC}"),
        &format!("source_properties='device.description=\"{MIC_LABEL}\"'"),
    ]);
    if let Err(e) = source {
        remove();
        return Err(error(e));
    }
    Ok(())
}

/// Quita el micrófono virtual de Ixtli, si existe.
pub fn remove() {
    let Ok(modules) = pactl(&["list", "short", "modules"]) else {
        return;
    };
    for id in module_ids(&modules) {
        let _ = pactl(&["unload-module", id]);
    }
}

/// Módulos de Ixtli en la salida de `pactl list short modules`.
/// Primero el micrófono, porque depende de la salida interna.
fn module_ids(modules: &str) -> Vec<&str> {
    let with = |needle: &str| {
        let needle = needle.to_string();
        modules.lines().filter_map(move |line| {
            let mut fields = line.split('\t');
            let id = fields.next()?;
            fields.any(|field| field.split_whitespace().any(|arg| arg == needle)).then_some(id)
        })
    };
    with(&format!("source_name={MIC}"))
        .chain(with(&format!("sink_name={SINK}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_own_modules_mic_first() {
        let modules = "1\tmodule-always-sink\t\t\n\
            536870916\tmodule-null-sink\tsink_name=ixtli_sink sink_properties='device.description=\"x\"'\t\n\
            536870917\tmodule-remap-source\tmaster=ixtli_sink.monitor source_name=ixtli_mic\t\n\
            536870918\tmodule-null-sink\tsink_name=otro_sink\t\n";
        assert_eq!(module_ids(modules), vec!["536870917", "536870916"]);
    }

    #[test]
    fn nothing_to_remove() {
        assert!(module_ids("1\tmodule-null-sink\tsink_name=ixtli_sink_viejo\t\n").is_empty());
    }
}
