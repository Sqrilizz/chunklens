# ChunkLens

ChunkLens reads Minecraft Java Edition Anvil worlds without modifying world files. It ranks saved chunks by a **Potential Load Score**, an offline heuristic for content that might be expensive when loaded and ticking. It does not measure a chunk's MSPT or prove that a particular chunk is causing lag.

```bash
cargo run --release -- scan /path/to/world --fast --json report.json.gz --top 20
cargo run --release -- load report.json.gz --top 20
cargo run --release -- tui report.json.gz
```

`scan` reads `.mca` region and entity files, including external `.mcc` chunks. `--min-score` filters retained chunks; the default is `0.1`. Use `--min-score 0` only when a complete chunk inventory is needed, since full reports can be very large. JSON and CSV exports can be compressed by using `.json.gz` or `.csv.gz`. The report loader rejects decoded reports over 256 MiB so a large export cannot exhaust memory while opening. Rescan with a higher `--min-score` to make a smaller interactive report.

The interactive shell is available through `chunklens servers`; it discovers local servers and offers a manual path or an optional full filesystem search if none are found. `chunklens` without arguments scans `./world` or the current directory when it contains a `region` folder. Otherwise it prints quick help.

```bash
cargo run --release -- servers
cargo run --release -- ping example.com
cargo run --release -- watch example.com
cargo run --release -- tui report.json.gz --host example.com --spark profile.pb
cargo run --release -- spark profile.pb
CHUNKLENS_RCON_PASSWORD='...' cargo run --release -- rcon localhost:25575 list
```

The TUI combines saved chunk data, server list ping, Spark data, and optional RCON. `spark` accepts a Spark sampler protobuf export or a nested JSON call tree; a renamed protobuf file is detected by its contents. Spark MSPT is a live measurement when present in the export, while chunk scores remain estimates. RCON executes commands on the selected server, so provide credentials only for servers you administer.

Run the quality checks with:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
