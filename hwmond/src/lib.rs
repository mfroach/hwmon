use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, ClockId, TemperatureSensor};
use nvml_wrapper::struct_wrappers::device::Utilization;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ─── Public data types ───────────────────────────────────────────────

/// Complete sensor snapshot — what the HTTP API returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorData {
    pub timestamp: u64,
    pub cpu_usage: CpuUsage,
    pub hwmon: Vec<Device>,
    pub gpu: Vec<GpuDevice>,
    pub thermal_zones: Vec<ThermalZone>,
}

/// CPU utilization (overall + per-core).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuUsage {
    pub total_percent: f64,
    pub cores: Vec<f64>,
}

/// A single hwmon device (GPU, chipset, HWMON chip, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub path: String,
    pub temperatures: Vec<TempInfo>,
    pub fans: Vec<FanInfo>,
    pub powers: Vec<PowerInfo>,
    pub voltages: Vec<VoltageInfo>,
    pub utilizations: Vec<UtilInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempInfo {
    pub label: String,
    pub value: f64,        // °C
    pub max: Option<f64>,   // °C
    pub crit: Option<f64>,  // °C
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanInfo {
    pub label: String,
    pub value: u32,        // RPM
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PowerInfo {
    pub label: String,
    pub value: f64,        // watts
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoltageInfo {
    pub label: String,
    pub value: f64,        // volts
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtilInfo {
    pub label: String,
    pub value: f64,        // percent
}

/// A Linux thermal zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalZone {
    pub name: String,
    pub path: String,
    pub temp: f64,          // °C
    pub type_: Option<String>,
}

/// A single NVIDIA GPU device (populated via NVML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuDevice {
    pub name: String,
    pub pci_bus_id: String,
    pub temperature: f64,            // °C
    pub fan_speed_percent: Option<u32>,
    pub power_draw_watts: Option<f64>,
    pub power_limit_watts: Option<f64>,
    pub utilization_gpu: f64,        // percent
    pub utilization_memory: f64,     // percent
    pub memory_used_mb: Option<u64>,
    pub memory_total_mb: Option<u64>,
    pub clock_graphics: Option<u64>, // MHz
    pub clock_memory: Option<u64>,   // MHz
    pub clock_sm: Option<u64>,       // MHz
}

// ─── Sensor collection ───────────────────────────────────────────────

/// Collect a full snapshot of all available hardware sensors.
pub fn collect() -> SensorData {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    SensorData {
        timestamp,
        cpu_usage: read_cpu_usage(),
        hwmon: read_hwmon_devices(),
        gpu: read_gpu_devices(),
        thermal_zones: read_thermal_zones(),
    }
}

// ─── CPU usage from /proc/stat ───────────────────────────────────────

fn read_cpu_usage() -> CpuUsage {
    let contents = match fs::read_to_string("/proc/stat") {
        Ok(c) => c,
        Err(_) => return CpuUsage { total_percent: 0.0, cores: vec![] },
    };

    let mut total_usage = 0.0;
    let mut core_usages: Vec<(f64, String)> = Vec::new();

    for line in contents.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        // Skip the aggregate "cpu  " line — we only want per-core
        if parts[0] == "cpu" {
            continue;
        }

        let user: u64 = parts[1].parse().unwrap_or(0);
        let nice: u64 = parts[2].parse().unwrap_or(0);
        let system: u64 = parts[3].parse().unwrap_or(0);
        let idle: u64 = parts[4].parse().unwrap_or(0);
        let iowait: u64 = parts[5].parse().unwrap_or(0);
        let irq: u64 = parts[6].parse().unwrap_or(0);
        let softirq: u64 = parts[7].parse().unwrap_or(0);
        let steal: u64 = parts.get(8).and_then(|v| v.parse().ok()).unwrap_or(0);

        let total = user + nice + system + idle + iowait + irq + softirq + steal;
        let busy = total.saturating_sub(idle).saturating_sub(iowait);
        let pct = if total == 0 { 0.0 } else { busy as f64 / total as f64 * 100.0 };

        total_usage += pct;
        core_usages.push((pct, parts[0].to_string()));
    }

    let total_percent = if core_usages.is_empty() {
        0.0
    } else {
        total_usage / core_usages.len() as f64
    };

    core_usages.sort_by(|a, b| a.1.cmp(&b.1));
    let cores = core_usages
        .into_iter()
        .map(|(p, _)| (p * 100.0).round() / 100.0)
        .collect();

    CpuUsage {
        total_percent: (total_percent * 100.0).round() / 100.0,
        cores,
    }
}

// ─── hwmon devices ───────────────────────────────────────────────────

fn read_hwmon_devices() -> Vec<Device> {
    let base = Path::new("/sys/class/hwmon");
    let mut devices = Vec::new();

    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return devices,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let dev_path = base.join(&name);

        let dev_name = fs::read_to_string(dev_path.join("name"))
            .ok()
            .and_then(|s| {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    Some(trimmed)
                } else {
                    None
                }
            })
            .unwrap_or_else(|| name.to_string_lossy().to_string());

        let temps = read_hwmon_temps(&dev_path);
        let fans = read_hwmon_fans(&dev_path);
        let powers = read_hwmon_powers(&dev_path);
        let voltages = read_hwmon_voltages(&dev_path);
        let utils = read_hwmon_utils(&dev_path);

        devices.push(Device {
            name: dev_name,
            path: dev_path.to_string_lossy().to_string(),
            temperatures: temps,
            fans,
            powers,
            voltages,
            utilizations: utils,
        });
    }

    devices
}

fn read_hwmon_temps(path: &Path) -> Vec<TempInfo> {
    let mut temps = Vec::new();
    let mut idx = 1;

    loop {
        let prefix = format!("temp{}", idx);
        let label_path = path.join(format!("{}_label", prefix));
        let input_path = path.join(format!("{}_input", prefix));

        let label = fs::read_to_string(&label_path)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Temp {}", idx));

        let value = fs::read_to_string(&input_path)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|v| v as f64 / 1000.0);

        let max = fs::read_to_string(path.join(format!("{}_max", prefix)))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|v| v as f64 / 1000.0);

        let crit = fs::read_to_string(path.join(format!("{}_crit", prefix)))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|v| v as f64 / 1000.0);

        if let Some(value) = value {
            temps.push(TempInfo { label, value, max, crit });
        }

        idx += 1;
        if idx > 20 {
            break;
        }
    }

    temps
}

fn read_hwmon_fans(path: &Path) -> Vec<FanInfo> {
    let mut fans = Vec::new();
    let mut idx = 1;

    loop {
        let label_path = path.join(format!("fan{}_label", idx));
        let input_path = path.join(format!("fan{}_input", idx));

        let label = fs::read_to_string(&label_path)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Fan {}", idx));

        let value = fs::read_to_string(&input_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(0);

        if value > 0 {
            fans.push(FanInfo { label, value });
        }

        idx += 1;
        if idx > 20 {
            break;
        }
    }

    fans
}

fn read_hwmon_powers(path: &Path) -> Vec<PowerInfo> {
    let mut powers = Vec::new();
    let mut idx = 1;

    loop {
        let label_path = path.join(format!("power{}_label", idx));
        let input_path = path.join(format!("power{}_input", idx));

        let label = fs::read_to_string(&label_path)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Power {}", idx));

        let value = fs::read_to_string(&input_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|v| v as f64 / 1_000_000.0);

        if let Some(value) = value {
            powers.push(PowerInfo { label, value });
        }

        idx += 1;
        if idx > 20 {
            break;
        }
    }

    powers
}

fn read_hwmon_voltages(path: &Path) -> Vec<VoltageInfo> {
    let mut voltages = Vec::new();
    let mut idx = 1;

    loop {
        let label_path = path.join(format!("in{}_label", idx));
        let input_path = path.join(format!("in{}_input", idx));

        let label = fs::read_to_string(&label_path)
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("In {}", idx));

        let value = fs::read_to_string(&input_path)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|v| v as f64 / 1000.0);

        if let Some(value) = value {
            voltages.push(VoltageInfo { label, value });
        }

        idx += 1;
        if idx > 20 {
            break;
        }
    }

    voltages
}

fn read_hwmon_utils(path: &Path) -> Vec<UtilInfo> {
    let mut utils = Vec::new();

    // Some hwmon chips expose utilization or frequency
    let names = ["util1_input", "freq1_input"];
    for name in &names {
        let input_path = path.join(name);
        let short = name.strip_suffix("_input").unwrap_or(name);

        let label = match short {
            s if s.starts_with("freq") => "Freq",
            s if s.starts_with("util") => "Util",
            _ => short,
        }
        .to_string();

        let value = fs::read_to_string(&input_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok());

        if let Some(value) = value {
            let pct = if value > 1_000_000_000 {
                (value as f64 / 1_000_000_000.0 * 100.0).min(100.0)
            } else {
                value as f64
            };
            utils.push(UtilInfo {
                label,
                value: (pct * 100.0).round() / 100.0,
            });
        }
    }

    utils
}

// ─── Thermal zones ───────────────────────────────────────────────────

fn read_thermal_zones() -> Vec<ThermalZone> {
    let base = Path::new("/sys/class/thermal");
    let mut zones = Vec::new();

    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return zones,
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("thermal_zone") {
            continue;
        }

        let zone_path = base.join(&name);
        let type_ = fs::read_to_string(zone_path.join("type"))
            .ok()
            .map(|s| s.trim().to_string());

        let temp = fs::read_to_string(zone_path.join("temp"))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(|v| v as f64 / 1000.0);

        if let Some(temp) = temp {
            zones.push(ThermalZone {
                name: name.to_string_lossy().to_string(),
                path: zone_path.to_string_lossy().to_string(),
                temp,
                type_,
            });
        }
    }

    zones
}

// ─── NVIDIA GPU (NVML) ───────────────────────────────────────────────

/// Read sensor data from all NVIDIA GPUs via NVML.
///
/// Returns an empty vector if NVML is unavailable (no driver, no GPU, etc.)
/// rather than panicking — the daemon stays running for non-GPU sensors.
fn read_gpu_devices() -> Vec<GpuDevice> {
    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("[hwmond] NVML init failed: {}. No NVIDIA GPUs detected.", e);
            return Vec::new();
        }
    };

    let device_count = match nvml.device_count() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[hwmond] NVML device_count failed: {}. Skipping GPUs.", e);
            return Vec::new();
        }
    };

    if device_count == 0 {
        return Vec::new();
    }

    let mut gpus = Vec::new();

    for i in 0..device_count {
        let device = match nvml.device_by_index(i) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[hwmond] NVML device({}) failed: {}", i, e);
                continue;
            }
        };

        let name = device.name().unwrap_or_else(|_| "Unknown".to_string());
        let pci_bus_id = device
            .pci_info()
            .map(|pci| pci.bus_id)
            .unwrap_or_else(|_| "Unknown".to_string());

        // Temperature (GPU core)
        let temperature = device
            .temperature(TemperatureSensor::Gpu)
            .map(|v| v as f64)
            .unwrap_or(0.0);

        // Fan speed (%)
        let fan_speed_percent = device
            .fan_speed(0)
            .map(|v| v as u32)
            .ok();

        // Power draw (watts)
        let power_draw_watts = device
            .power_usage()
            .ok()
            .map(|mw| mw as f64 / 1000.0);

        // Power limit (watts)
        let power_limit_watts = device
            .power_management_limit()
            .ok()
            .map(|mw| mw as f64 / 1000.0);

        // Utilization rates
        let util = device
            .utilization_rates()
            .unwrap_or_else(|_| Utilization {
                gpu: 0,
                memory: 0,
            });
        let utilization_gpu = util.gpu as f64;
        let utilization_memory = util.memory as f64;

        // Memory info
        let (memory_used_mb, memory_total_mb) = match device.memory_info() {
            Ok(mem) => (mem.used / 1024 / 1024, mem.total / 1024 / 1024),
            Err(e) => {
                eprintln!("[hwmond] NVML memory_info failed for device {i}: {e}. Skipping.");
                continue;
            }
        };

        // Clocks
        let clock_graphics = device
            .clock(Clock::Graphics, ClockId::Current)
            .ok()
            .map(|c| c as u64);
        let clock_memory = device
            .clock(Clock::Memory, ClockId::Current)
            .ok()
            .map(|c| c as u64);
        let clock_sm = device
            .clock_info(Clock::SM)
            .ok()
            .map(|c| c as u64);

        gpus.push(GpuDevice {
            name,
            pci_bus_id,
            temperature,
            fan_speed_percent,
            power_draw_watts,
            power_limit_watts,
            utilization_gpu,
            utilization_memory,
            memory_used_mb: Some(memory_used_mb),
            memory_total_mb: Some(memory_total_mb),
            clock_graphics,
            clock_memory,
            clock_sm,
        });
    }

    gpus
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect() {
        let data = collect();
        assert!(data.timestamp > 0);
    }
}
