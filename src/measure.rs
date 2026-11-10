//! Mediciones para `ixtli medir`: cuánta batería gasta el teléfono y cuánta
//! CPU y memoria usa Ixtli (con scrcpy y adb) en la computadora.

use std::fs;
use std::time::Duration;

use crate::sys;

/// Estado de la batería del teléfono.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Battery {
    /// Nivel, de 0 a 100.
    pub level: u8,
    /// Temperatura en grados Celsius.
    pub temperature: f32,
    /// Está recibiendo carga (por el cable USB o un cargador).
    pub charging: bool,
}

/// Debajo de este nivel, sin carga, Ixtli sugiere conectar el cable.
pub const LOW_BATTERY: u8 = 20;

impl Battery {
    /// Queda poca batería y no se está cargando.
    pub fn low(&self) -> bool {
        self.level < LOW_BATTERY && !self.charging
    }
}

pub fn battery(serial: &str) -> Option<Battery> {
    let output = sys::run_for(
        "adb",
        &["-s", serial, "shell", "dumpsys", "battery"],
        Duration::from_secs(5),
    )
    .ok()?;
    parse_battery(&output)
}

/// Interpreta `dumpsys battery`. La temperatura viene en décimas de grado.
fn parse_battery(output: &str) -> Option<Battery> {
    let value = |key: &str| {
        output
            .lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix(':'))
            .map(str::trim)
    };
    let powered = ["AC powered", "USB powered", "Wireless powered", "Dock powered"]
        .iter()
        .any(|key| value(key) == Some("true"));
    Some(Battery {
        level: value("level")?.parse().ok()?,
        temperature: value("temperature")?.parse::<f32>().ok()? / 10.0,
        charging: powered,
    })
}

/// Lo que llevan gastado los procesos de Ixtli en este momento.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    /// Tiempo de CPU acumulado, en segundos.
    pub cpu_seconds: f64,
    /// Memoria en uso, en megabytes.
    pub memory_mb: f64,
}

struct Process {
    pid: u32,
    parent: u32,
    name: String,
    /// Tiempo de CPU en "ticks" del reloj del sistema.
    ticks: u64,
}

/// De una línea de `/proc/PID/stat` saca lo necesario. El nombre va entre
/// paréntesis y puede traer espacios.
fn parse_stat(line: &str) -> Option<Process> {
    let (head, tail) = (line.split_once('(')?, line.rsplit_once(')')?);
    let pid = head.0.trim().parse().ok()?;
    let name = line[head.0.len() + 1..line.len() - tail.1.len() - 1].to_string();
    let fields: Vec<&str> = tail.1.split_whitespace().collect();
    // Después del nombre: estado, padre, ... utime y stime son el 12 y el 13.
    Some(Process {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        name,
        ticks: fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?,
    })
}

/// Suma este proceso, sus descendientes (scrcpy) y el servidor de adb, que
/// es quien mueve el video aunque no sea hijo de Ixtli.
pub fn usage() -> Usage {
    let processes: Vec<Process> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().into_string().ok()?.parse::<u32>().ok()?;
            parse_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
        })
        .collect();
    let mut wanted = vec![std::process::id()];
    let mut index = 0;
    while index < wanted.len() {
        let parent = wanted[index];
        wanted.extend(processes.iter().filter(|p| p.parent == parent).map(|p| p.pid));
        index += 1;
    }
    let (hz, page) = unsafe { (libc::sysconf(libc::_SC_CLK_TCK), libc::sysconf(libc::_SC_PAGESIZE)) };
    let mut total = Usage::default();
    for process in processes.iter().filter(|p| wanted.contains(&p.pid) || p.name == "adb") {
        total.cpu_seconds += process.ticks as f64 / hz.max(1) as f64;
        // El segundo número de statm son las páginas residentes en memoria.
        let pages = fs::read_to_string(format!("/proc/{}/statm", process.pid))
            .ok()
            .and_then(|statm| statm.split_whitespace().nth(1)?.parse::<f64>().ok())
            .unwrap_or(0.0);
        total.memory_mb += pages * page.max(0) as f64 / (1024.0 * 1024.0);
    }
    total
}

/// Una muestra de la medición.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub minute: f64,
    pub battery: Option<Battery>,
    /// Porcentaje de un núcleo usado desde la muestra anterior.
    pub cpu: Option<f64>,
    pub memory_mb: f64,
}

/// Resumen de toda la medición.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub minutes: f64,
    /// Cambio de batería por hora, en puntos (negativo si bajó).
    pub battery_per_hour: Option<f64>,
    pub temperature_change: Option<f32>,
    pub cpu_average: f64,
    pub memory_max: f64,
    pub charging: bool,
}

pub fn summarize(samples: &[Sample]) -> Option<Summary> {
    let (first, last) = (samples.first()?, samples.last()?);
    let minutes = last.minute - first.minute;
    if minutes <= 0.0 {
        return None;
    }
    let cpus: Vec<f64> = samples.iter().filter_map(|sample| sample.cpu).collect();
    let both = first.battery.zip(last.battery);
    Some(Summary {
        minutes,
        battery_per_hour: both
            .map(|(start, end)| (f64::from(end.level) - f64::from(start.level)) * 60.0 / minutes),
        temperature_change: both.map(|(start, end)| end.temperature - start.temperature),
        cpu_average: if cpus.is_empty() { 0.0 } else { cpus.iter().sum::<f64>() / cpus.len() as f64 },
        memory_max: samples.iter().map(|sample| sample.memory_mb).fold(0.0, f64::max),
        charging: samples.iter().any(|sample| sample.battery.is_some_and(|b| b.charging)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_battery() {
        let battery = |level, charging| Battery { level, temperature: 30.0, charging };
        assert!(battery(19, false).low());
        assert!(!battery(20, false).low());
        assert!(!battery(5, true).low());
    }

    #[test]
    fn battery_dump() {
        let dump = "Current Battery Service state:\n  AC powered: false\n  USB powered: true\n  Wireless powered: false\n  status: 2\n  level: 85\n  scale: 100\n  temperature: 301\n";
        assert_eq!(
            parse_battery(dump),
            Some(Battery { level: 85, temperature: 30.1, charging: true })
        );
        let unplugged = dump.replace("USB powered: true", "USB powered: false");
        assert!(!parse_battery(&unplugged).unwrap().charging);
        assert_eq!(parse_battery("nada"), None);
    }

    #[test]
    fn stat_line() {
        let line = "4242 (Web Content (x)) S 100 4242 4242 0 -1 4194560 1 0 0 0 250 50 0 0 20 0 9 0 1 2 3";
        let process = parse_stat(line).unwrap();
        assert_eq!((process.pid, process.parent, process.ticks), (4242, 100, 300));
        assert_eq!(process.name, "Web Content (x)");
        assert!(usage().memory_mb > 0.0);
    }

    #[test]
    fn summary() {
        let battery = |level| Some(Battery { level, temperature: 30.0, charging: false });
        let samples = [
            Sample { minute: 0.0, battery: battery(80), cpu: None, memory_mb: 100.0 },
            Sample { minute: 5.0, battery: battery(79), cpu: Some(10.0), memory_mb: 140.0 },
            Sample { minute: 10.0, battery: battery(78), cpu: Some(14.0), memory_mb: 120.0 },
        ];
        let summary = summarize(&samples).unwrap();
        assert_eq!(summary.battery_per_hour, Some(-12.0));
        assert_eq!(summary.cpu_average, 12.0);
        assert_eq!(summary.memory_max, 140.0);
        assert!(!summary.charging);
        assert_eq!(summarize(&samples[..1]), None);
    }
}
