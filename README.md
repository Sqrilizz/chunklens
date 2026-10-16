# ChunkLens

[![CI](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml/badge.svg)](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml)
[![Rust: 2024](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)

Fast, read-only Minecraft Java Edition Anvil region scanner, split-screen lag investigator, and Spark profile correlator written in Rust.

It scans `.mca` files at NVMe speed, surfaces root-cause culprits (villager AI clusters, unoptimized hoppers, redstone clocks, NBT bloat/dupe stashes), and gives you an instant master-detail TUI with one-key fixes (`/tp`, targeted `/kill`) and remote RCON integration.

---

## Highlights

* **Fast parallel scanning:** ~75,000 chunks/second throughput on NVMe with multi-threaded parallel scanning (Rayon).
* **Low-overhead decompression:** SIMD-accelerated decompression via `libdeflater` for zlib/deflate payloads with minimal allocations.
* **Dual-pane TUI:** Master-detail split screen (lazygit/btop style) — see all top offenders on the left, instant deep diagnosis and one-key commands on the right.
* **Smart chunk diagnostics:** Automatic heuristic tags (`[VILLAGER]`, `[HOPPERS]`, `[REDSTONE]`, `[NBT-STASH]`, `[MOB-FARM]`) with clear severity levels.
* **Spark profiler correlation:** Ingests Spark `.json` and binary `.pb` dumps to correlate offline world hotspots with live thread MSPT (`EntityTickList.forEach`, etc.).
* **Live RCON & SLP:** Live server ping watcher with latency history, online player monitoring, and interactive remote console.
* **Memory safe:** Bounded report decoding to prevent OOM on massive multi-gigabyte worlds.

---

## Installation

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
# Fast scan with all CPU cores and export report
chunklens scan /path/to/world --fast --json report.json.gz -n 20

# Eco scan (half cores) for background scanning on production servers
chunklens scan /path/to/world --eco --min-score 20.0 -n 10

# Scan for oversized chunks (>1MB) and dense container storage
chunklens scan /path/to/world --bloat -n 20

# Rank chunks by specific category (villagers, hoppers, redstone, bloat, etc.)
chunklens scan /path/to/world --sort villagers -n 15
```

### Emergency Triage TUI (Master-Detail)

Launch the dashboard on a world folder or saved report:

```bash
# Open saved report in TUI
chunklens tui report.json.gz

# Open report with live server ping & remote RCON
chunklens tui report.json.gz --host play.example.com:25565 --rcon play.example.com:25575 -p <password>

# Correlate world chunks with a Spark profile dump
chunklens tui report.json.gz --spark spark-report.json
```

### Remote Operations & Monitoring

```bash
# Ping a live server via SLP (Server List Ping)
chunklens ping play.example.com:25565

# Real-time watcher with latency sparklines
chunklens watch play.example.com:25565

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
| `y` / `c` | Copy `/tp @s X ~ Z` teleport command to clipboard |
| `x` / `p` | Copy targeted `/kill` purge command for selected chunk |
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
