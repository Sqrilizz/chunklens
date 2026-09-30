# ChunkLens

[![CI](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml/badge.svg)](https://github.com/Sqrilizz/chunklens/actions/workflows/ci.yml)
[![Rust: 2024](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)

ChunkLens is a fast, read-only Minecraft Java Edition Anvil region scanner, terminal UI lag investigator, and Spark profile correlator written in Rust.

It safely parses saved `.mca` region files, extracts dense entity clusters, detects NBT bloat and dupe stashes, clusters neighboring loaded regions, and provides an interactive terminal dashboard without modifying world files or degrading server MSPT.

---

## Performance Highlights

* ~75,000 chunks/second throughput on modern NVMe drives with multi-threaded parallel scanning (Rayon).
* Real-world benchmark: scanned an entire production world (2,587,788 chunks) in 35.2 seconds with zero impact on active players.
* SIMD-accelerated decompression powered by `libdeflater` for fast zlib/gzip chunk handling with zero heap allocations for standard palettes.
* Memory-safe guardrails: rejects decoded reports over 256 MiB to prevent OOM when loading historical scans.

---

## Key Features

* Anvil & MCC Parsing: reads standard `.mca` region and entity files as well as external `.mcc` oversized chunks.
* Potential Load Score: multi-factor heuristic ranking chunks based on hoppers, hopper minecarts, villagers, mob counts, tile entities, and redstone clocks.
* NBT Bloat & Dupe Detection: flags chunks with massive payloads (>1 MB), excessive shulker boxes, item frames, or thousands of container slots.
* Adjacent Chunk Clustering: detects sprawling mega-bases and mob farms that cross chunk borders and computes cumulative lag scores.
* Interactive Terminal UI (TUI): built with Ratatui; featuring chunk tables, multi-chunk heatmaps, Spark profile inspector, and live RCON console.
* Live Server Integration:
  * SLP Ping: real-time server latency, player counts, protocol version, and ANSI MOTD.
  * Spark Integration: ingests Spark `.pb` protobuf or `.json` profile dumps to correlate offline world hotspots with live thread MSPT.
  * Built-in RCON: inspect, teleport (`/tp @s`), or execute console commands directly from the dashboard.

---

## Installation

### From Source

Ensure you have Rust (edition 2024 / latest stable) installed:

```bash
git clone https://github.com/Sqrilizz/chunklens.git
cd chunklens
cargo build --release
```

The compiled binary will be located at `target/release/chunklens`.

---

## Usage

### Quick Start (Interactive Menu)

Run `chunklens` with no arguments to automatically detect local Minecraft servers or search the filesystem:

```bash
chunklens
```

### World Scanning

```bash
# Fast scan with all CPU cores and export compressed JSON report
chunklens scan /path/to/world --fast --json report.json.gz -n 20

# Eco scan (half cores) for background scanning on production servers
chunklens scan /path/to/world --eco --min-score 20.0 -n 10

# Inspect NBT bloat and oversized storage stashes
chunklens scan /path/to/world --bloat -n 20

# Rank chunks by specific category (villagers, hoppers, stands, entities, redstone, chests, shulkers)
chunklens scan /path/to/world --sort villagers -n 15
```

### Offline Report Analysis

```bash
# Inspect a saved report without re-reading the world
chunklens load report.json.gz -n 20

# Rank saved report by category
chunklens load report.json.gz --sort hoppers -n 10

# Launch interactive fullscreen TUI
chunklens tui report.json.gz
```

### Server Monitoring & Live Diagnostics

```bash
# Ping a live server via Server List Ping (SLP)
chunklens ping play.example.com:25565

# Real-time watcher with latency sparklines
chunklens watch play.example.com:25565

# Correlate world chunks with a Spark profile
chunklens tui report.json.gz --host play.example.com:25565 --spark spark-profile.pb

# Ingest and summarize a Spark profile in CLI
chunklens spark spark-profile.json
```

---

## TUI Navigation & Shortcuts

| Key | Action |
|:---:|:---|
| `Tab` / `1`–`4` | Switch primary tabs (Chunks, Clusters, Live Monitor, RCON) |
| `↑` / `↓` or `J` / `K` | Navigate rows |
| `PageUp` / `PageDown` | Scroll table pages |
| `/` | Search & filter chunks |
| `Enter` / `D` | Open detailed Chunk Inspector |
| `H` | Open 2D World Heatmap |
| `S` | Open Spark Profiler Analysis |
| `C` / `Y` | Copy teleport command (`/tp @s X ~ Z`) to clipboard |
| `?` | Toggle Help popup |
| `Esc` / `Q` | Back / Quit |

---

## Testing & Code Quality

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --locked
```
