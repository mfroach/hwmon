use eframe::egui;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

// ─── API data types ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct SensorData {
    timestamp: u64,
    cpu_usage: CpuUsage,
    hwmon: Vec<Device>,
    gpu: Vec<GpuDevice>,
    thermal_zones: Vec<ThermalZone>,
}

#[derive(Debug, Clone, Deserialize)]
struct CpuUsage {
    total_percent: f64,
    cores: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct Device {
    name: String,
    path: String,
    temperatures: Vec<TempInfo>,
    fans: Vec<FanInfo>,
    powers: Vec<PowerInfo>,
    voltages: Vec<VoltageInfo>,
    utilizations: Vec<UtilInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct TempInfo {
    label: String,
    value: f64,
    max: Option<f64>,
    crit: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct FanInfo {
    label: String,
    value: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct PowerInfo {
    label: String,
    value: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct VoltageInfo {
    label: String,
    value: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct UtilInfo {
    label: String,
    value: f64,
}

#[derive(Debug, Clone, Deserialize)]
struct ThermalZone {
    name: String,
    path: String,
    temp: f64,
    #[allow(dead_code)]
    type_: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GpuDevice {
    name: String,
    pci_bus_id: String,
    temperature: f64,
    fan_speed_percent: Option<u32>,
    power_draw_watts: Option<f64>,
    power_limit_watts: Option<f64>,
    utilization_gpu: f64,
    utilization_memory: f64,
    memory_used_mb: Option<u64>,
    memory_total_mb: Option<u64>,
    clock_graphics: Option<u64>,
    clock_memory: Option<u64>,
    clock_sm: Option<u64>,
}

// ─── Shared state ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum AppState {
    Loading,
    Ready(SensorData),
    Error(String),
}

// ─── Async poller ───────────────────────────────────────────────────

async fn poll_api(state: &Mutex<AppState>) {
    let client = reqwest::Client::new();
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        let resp = client.get("http://localhost:9899/api").send().await;
        match resp {
            Ok(r) => match r.json::<SensorData>().await {
                Ok(data) => {
                    *state.lock().unwrap() = AppState::Ready(data);
                }
                Err(e) => {
                    *state.lock().unwrap() =
                        AppState::Error(format!("Parse error: {}", e));
                }
            },
            Err(e) => {
                *state.lock().unwrap() = AppState::Error(format!("Request failed: {}", e));
            }
        }
    }
}

// ─── egui app ───────────────────────────────────────────────────────

struct HwmonApp {
    state: Arc<Mutex<AppState>>,
}

impl HwmonApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Spawn the polling loop
        let state_for_thread = std::sync::Arc::new(std::sync::Mutex::new(AppState::Loading));
        let state_for_thread_clone = state_for_thread.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    poll_api(&state_for_thread_clone).await;
                });
        });

        HwmonApp { state: state_for_thread }
    }
}

impl eframe::App for HwmonApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self.state.lock().unwrap().clone();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Hardware Monitor");
            ui.separator();

            match state {
                AppState::Loading => {
                    ui.label("Loading sensor data...");
                }
                AppState::Error(msg) => {
                    ui.colored_label(egui::Color32::RED, &msg);
                }
                AppState::Ready(data) => {
                    Self::render_cpu(ui, &data.cpu_usage);
                    ui.separator();
                    Self::render_hwmon_devices(ui, &data.hwmon);
                    ui.separator();
                    Self::render_gpus(ui, &data.gpu);
                    ui.separator();
                    Self::render_thermal_zones(ui, &data.thermal_zones);
                }
            }
        });
    }
}

impl HwmonApp {
    fn render_cpu(ui: &mut egui::Ui, cpu: &CpuUsage) {
        ui.heading("CPU Usage");
        ui.horizontal(|ui| {
            ui.label(format!("Total: {:.1}%", cpu.total_percent));
            let _color = if cpu.total_percent > 90.0 {
                egui::Color32::RED
            } else if cpu.total_percent > 70.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::GREEN
            };
            ui.allocate_ui((200.0, 24.0).into(), |ui| {
                    ui.add(
                        egui::ProgressBar::new((cpu.total_percent / 100.0) as f32)
                            .show_percentage()
                            .desired_width(200.0),
                    );
                },
            );
            ui.label(format!("Cores: {}", cpu.cores.len()));
        });

        if !cpu.cores.is_empty() {
            ui.indent("core_graph", |ui| {
                ui.label("Per-core usage:");
                let _max_val = cpu.cores.iter().cloned().fold(0.0f64, f64::max);
                let bar_height = 20.0;
                let bar_spacing = 2.0;
                let total_width = cpu.cores.len() as f32 * (bar_height + bar_spacing) - bar_spacing;
                ui.allocate_ui(
                    egui::Vec2::new(total_width.max(100.0), bar_height + 16.0),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (i, core) in cpu.cores.iter().enumerate() {
                                let _color = if *core > 90.0 {
                                    egui::Color32::RED
                                } else if *core > 70.0 {
                                    egui::Color32::YELLOW
                                } else {
                                    egui::Color32::GREEN
                                };
                                ui.label(format!(
                                    "{}:{:.0}%",
                                    i, *core
                                ));
                            }
                        });
                    },
                );
            });
        }
    }

    fn render_hwmon_devices(ui: &mut egui::Ui, devices: &[Device]) {
        ui.heading("Hardware Devices");
        if devices.is_empty() {
            ui.label("No hwmon devices found");
            return;
        }
        for dev in devices {
            ui.collapsing(&dev.name, |ui| {
                if !dev.temperatures.is_empty() {
                    ui.heading("Temperatures");
                    for t in &dev.temperatures {
                        let color = if t.value > 90.0 {
                            egui::Color32::RED
                        } else if t.value > 70.0 {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::GREEN
                        };
                        let text = if let Some(max) = t.max {
                            format!(
                                "{}: {:.1}°C (max: {:.1}°C)",
                                t.label, t.value, max
                            )
                        } else {
                            format!("{}: {:.1}°C", t.label, t.value)
                        };
                        ui.colored_label(color, text);
                    }
                }
                if !dev.fans.is_empty() {
                    ui.heading("Fans");
                    for f in &dev.fans {
                        ui.label(format!(
                            "{}: {} RPM",
                            if f.label.is_empty() {
                                "Fan"
                            } else {
                                &f.label
                            },
                            f.value
                        ));
                    }
                }
                if !dev.powers.is_empty() {
                    ui.heading("Powers");
                    for p in &dev.powers {
                        ui.label(format!(
                            "{}: {:.2} W",
                            if p.label.is_empty() {
                                "Power"
                            } else {
                                &p.label
                            },
                            p.value
                        ));
                    }
                }
                if !dev.voltages.is_empty() {
                    ui.heading("Voltages");
                    for v in &dev.voltages {
                        ui.label(format!(
                            "{}: {:.3} V",
                            if v.label.is_empty() {
                                "Voltage"
                            } else {
                                &v.label
                            },
                            v.value
                        ));
                    }
                }
                if !dev.utilizations.is_empty() {
                    ui.heading("Utilizations");
                    for u in &dev.utilizations {
                        ui.label(format!(
                            "{}: {:.1}%",
                            if u.label.is_empty() {
                                "Utilization"
                            } else {
                                &u.label
                            },
                            u.value
                        ));
                    }
                }
            });
        }
    }

    fn render_gpus(ui: &mut egui::Ui, gpus: &[GpuDevice]) {
        ui.heading("GPUs");
        if gpus.is_empty() {
            ui.label("No GPUs found");
            return;
        }
        for gpu in gpus {
            ui.collapsing(
                format!(
                    "{} ({}% util, {:.0}°C)",
                    gpu.name, gpu.utilization_gpu, gpu.temperature
                ),
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("PCI: {}", gpu.pci_bus_id));
                    });

                    // GPU utilization bar
                    ui.horizontal(|ui| {
                        ui.label("GPU:");
                        ui.add(
                            egui::ProgressBar::new((gpu.utilization_gpu / 100.0) as f32)
                                .show_percentage()
                                .desired_width(150.0),
                        );
                    });

                    // Memory utilization bar
                    if let (Some(used), Some(total)) =
                        (gpu.memory_used_mb, gpu.memory_total_mb)
                    {
                        let pct = if total > 0 {
                            used as f64 / total as f64 * 100.0
                        } else {
                            0.0
                        };
                        ui.horizontal(|ui| {
                            ui.label("Memory:");
                            ui.add(
                                egui::ProgressBar::new((pct / 100.0) as f32)
                                    .show_percentage()
                                    .desired_width(150.0),
                            );
                            ui.label(format!(
                                "{} / {} MB",
                                used, total
                            ));
                        });
                    }

                    ui.horizontal(|ui| {
                        ui.label(format!("Temp: {:.0}°C", gpu.temperature));
                        if let Some(fan) = gpu.fan_speed_percent {
                            ui.label(format!("Fan: {}%", fan));
                        }
                        if let Some(power) = gpu.power_draw_watts {
                            let label = if let Some(limit) =
                                gpu.power_limit_watts
                            {
                                format!(
                                    "Power: {:.1}W / {:.1}W",
                                    power, limit
                                )
                            } else {
                                format!("Power: {:.1}W", power)
                            };
                            ui.label(label);
                        }
                    });

                    ui.horizontal(|ui| {
                        if let Some(clock) = gpu.clock_graphics {
                            ui.label(format!("Clock(G): {} MHz", clock));
                        }
                        if let Some(clock) = gpu.clock_memory {
                            ui.label(format!("Clock(M): {} MHz", clock));
                        }
                        if let Some(clock) = gpu.clock_sm {
                            ui.label(format!("Clock(SM): {} MHz", clock));
                        }
                    });
                },
            );
        }
    }

    fn render_thermal_zones(
        ui: &mut egui::Ui,
        zones: &[ThermalZone],
    ) {
        ui.heading("Thermal Zones");
        if zones.is_empty() {
            ui.label("No thermal zones found");
            return;
        }
        for zone in zones {
            let color = if zone.temp > 90.0 {
                egui::Color32::RED
            } else if zone.temp > 70.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::GREEN
            };
            let name = if zone.name.is_empty() {
                zone.path.clone()
            } else {
                zone.name.clone()
            };
            ui.colored_label(color, format!("{}: {:.1}°C", name, zone.temp));
        }
    }
}

// ─── Main ────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 800.0]),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "Hardware Monitor",
        native_options,
        Box::new(|cc| Ok(Box::new(HwmonApp::new(cc)))),
    );
}
