# ChunkLens

[![CI](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml/badge.svg)](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sqrilizz/chunklens?color=blue)](https://github.com/Sqrilizz/chunklens/releases)
[![Rust: 2024](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)

Fast, read-only Minecraft Java Edition Anvil & Linear region scanner, split-screen lag investigator, and Spark profile correlator written in Rust.

It scans `.mca` and `.linear` files at NVMe speed, surfaces root-cause culprits (villager AI clusters, unoptimized hoppers, redstone clocks, NBT bloat/dupe stashes), and gives you an instant master-detail TUI with one-key fixes (`/tp`, targeted `/kill`) and zero-config remote RCON integration.

---

## Highlights

* **Fast parallel scanning:** ~75,000 chunks/second throughput on NVMe with multi-threaded parallel scanning (Rayon).
* **Anvil & Linear formats:** Native support for both vanilla Anvil (`.mca`) and high-performance Zstandard Linear (`.linear`) region formats used in Leaf, Gale, Purpur, and LeavesMC.
* **Low-overhead decompression:** SIMD-accelerated decompression via `libdeflater` for zlib/deflate and `zstd` for Linear regions with minimal allocations.
* **Zero-config RCON & discovery:** Automatically detects RCON passwords and ports from local `.env`, `server.properties`, and `pumpkin.toml` when inside a server folder.
* **Dual-pane TUI:** Master-detail split screen (lazygit/btop style) — see all top offenders on the left, instant deep diagnosis and one-key commands on the right.
* **Topographic Minimap Radar:** 9×9 ASCII density radar (`m` toggle) showing surrounding chunk load and relative player proximity.
* **Interactive RCON Actions:** Live confirmation modal (`x`/`p`) to execute targeted entity purges directly via RCON or copy commands safely.
* **Server Config Doctor:** Automated audits of `server.properties`, `paper.yml`, and `purpur.yml` flagging unoptimized simulation distance, hopper events, and redstone engines.
* **Lag Sentinel & Webhooks:** Real-time TPS/MSPT monitoring daemon (`chunklens live`) with Discord alert webhooks on sudden performance spikes.
* **Spark profiler correlation:** Ingests Spark `.json` and binary `.pb` dumps to correlate offline world hotspots with live thread MSPT (`EntityTickList.forEach`, etc.).
* **Memory safe:** Bounded report decoding to prevent OOM on massive multi-gigabyte worlds.

---

## Installation

### Pre-built Binaries

Download pre-compiled standalone binaries for Linux (`glibc` and static `musl`), macOS, and Windows from [GitHub Releases](https://github.com/Sqrilizz/chunklens/releases).

### From Source

Requires Rust (edition 2024 / stable 1.85+):

```bash
git clone https://github.com/Sqrilizz/chunklens.git
cd chunklens
cargo build --release
```

The binary will be built at `target/release/chunklens`.

---

## Usage

### Quick Start (Interactive Menu)

Run `chunklens` with no arguments to discover local server installations or inspect the current directory:

```bash
chunklens
```

### World Scanning

```bash
# Fast scan with all CPU cores and export report (supports .mca and .linear)
chunklens scan /path/to/world --fast --json report.json.gz -n 20

# Eco scan (half cores) for background scanning on production servers
chunklens scan /path/to/world --eco --min-score 20.0 -n 10

# Scan for oversized chunks (>1MB) and dense container storage
chunklens scan /path/to/world --bloat -n 20

# Rank chunks by specific category (villagers, hoppers, redstone, bloat, etc.)
chunklens scan /path/to/world --sort villagers -n 15
```

### Server Configuration Doctor

Audit server settings and flags for known performance bottlenecks:

```bash
# Audit the current server directory
chunklens doctor

# Audit a specific server path
chunklens doctor /srv/minecraft/paper-survival
```

Inspects `server.properties`, `paper.yml` / `paper-world-defaults.yml`, and `purpur.yml` for:
* Simulation & View distances (>6 and >10)
* Synchronous chunk writes (`sync-chunk-writes`)
* Aggressive packet compression thresholds
* Hopper `disable-move-event` overhead
* Redstone engine implementations (`alternate-current`)
* Villager brain ticking and lobotomization settings

### Emergency Triage TUI (Master-Detail)

Launch the dashboard on a world folder or saved report:

```bash
# Open saved report in TUI
chunklens tui report.json.gz

# Open world with automatic RCON connection (auto-detects password from server.properties/.env)
chunklens tui /path/to/server

# Open report with explicit live server ping & remote RCON
chunklens tui report.json.gz --host play.example.com:25565 --rcon play.example.com:25575 -p <password>

# Correlate world chunks with a Spark profile dump
chunklens tui report.json.gz --spark spark-report.json
```

### Real-time Lag Sentinel & Alerting

Monitor a live server's TPS/MSPT via RCON with automatic webhook alerts on performance drops:

```bash
# Sentinel monitor with Discord webhook alerting on lag spikes
chunklens live 127.0.0.1:25575 -p secret \
  --mspt-spike 60.0 \
  --webhook "https://discord.com/api/webhooks/1234/abcd..."
```

### Remote Operations & CLI Utilities

```bash
# Ping a live server via SLP (Server List Ping)
chunklens ping play.example.com:25565

# Real-time watcher with latency sparklines
chunklens watch play.example.com:25565

# Execute command on local server via RCON (auto-detects credentials from server.properties)
chunklens rcon "tps"

# Execute command on remote server via RCON
chunklens rcon play.example.com:25575 -p <password> "tps"

# Ingest and summarize a Spark profile dump in CLI
chunklens spark spark-profile.json
```

---

## TUI Keyboard Shortcuts

| Key | Action |
|:---:|:---|
| `1` – `4` / `Tab` | Switch main views (`1: Chunks`, `2: Clusters`, `3: Live Monitor`, `4: Console`) |
| `j` / `k` or `↑` / `↓` | Navigate top offender rows (right pane updates instantly) |
| `PgUp` / `PgDn` | Scroll table page |
| `f` | Cycle category filter (`All` → `Villagers & Mobs` → `Hoppers` → `Redstone` → `NBT & Storage`) |
| `/` | Instant text search (coordinates, dimension, score) |
| `m` | Toggle Topographic 2D Minimap Radar in the inspector panel |
| `y` / `c` | Copy `/tp @s X ~ Z` teleport command to clipboard |
| `x` / `p` | Open live RCON purge modal (executes `/kill` on confirmation or copies command) |
| `Enter` / `d` | Open full fullscreen chunk inspector |
| `h` | Open 2D potential load heatmap |
| `s` | Open Spark profile analysis tab |
| `r` | Refresh live server status & ping |
| `?` | Toggle help popup |
| `Esc` / `q` | Back / Quit |

---

## Development & Testing

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --locked
```
