# hwmond — Hardware Monitoring Daemon & Widget

A lightweight Linux hardware monitoring widget inspired by [LACT](https://github.com/ilyapr/LACT).

## Architecture

```
┌─────────────┐    HTTP JSON     ┌──────────────┐
│   hwmond     │◄────────────────│  widget.html  │
│  (Rust)      │  polling /api   │  (HTML/JS)    │
└─────────────┘    GET /api      └──────────────┘
     │
     ├─ /sys/class/hwmon/    ← temperatures, fans, power
     ├─ /sys/devices/.../cpu*/  ← thermal zones
     └─ /proc/stat            ← CPU utilization
```

**Daemon** reads hardware sensors via sysfs and exposes a JSON API on `localhost:9899`.  
**Widget** is a single-file HTML dashboard that polls the daemon every 2 seconds and renders real-time charts.

## Quick Start

```bash
# Build
cargo build --release

# Run daemon (requires root for some sensor access)
sudo target/release/hwmond

# Open widget in browser
xdg-open widget.html
```

## Sensors Read

| Source         | Data                        |
|----------------|-----------------------------|
| `hwmon` sysfs  | Temperatures, fan RPMs, power, utilization |
| CPU thermal    | Per-core/package temps      |
| `/proc/stat`   | CPU utilization %           |

## License

MIT
