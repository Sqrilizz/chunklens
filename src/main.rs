mod analysis;
mod discovery;
mod model;
mod nbt;
pub mod protocol;
mod region;
mod shell;
pub mod tui;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use discovery::{Server, discover_servers, discover_worlds, server_at};
use model::ScanResult;

#[derive(Parser)]
#[command(
    name = "chunklens",
    version,
    about = "Fast, lightweight Minecraft Anvil world scanner & lag investigator"
)]
struct Cli {
    /// World folder path to scan (defaults to ./world if present)
    #[arg(value_name = "WORLD")]
    world: Option<PathBuf>,

    /// Maximum worker threads (defaults to CPU cores - 1)
    #[arg(short = 't', long)]
    threads: Option<usize>,

    /// Use 100% of all CPU cores
    #[arg(long, conflicts_with = "eco")]
    fast: bool,

    /// Low CPU usage (half cores) for background scanning
    #[arg(long, conflicts_with = "fast")]
    eco: bool,

    /// Minimum chunk score to retain (default: 0.1)
    #[arg(long, default_value_t = 0.1)]
    min_score: f64,

    /// Save JSON report to file
    #[arg(long)]
    json: Option<PathBuf>,

    /// Save CSV report to file
    #[arg(long)]
    csv: Option<PathBuf>,

    /// Number of top potential load chunks to show (default: 10)
    #[arg(short = 'n', long, default_value_t = 10)]
    top: usize,

    /// Inspect NBT size, oversized chunks (>1MB), and dense storage
    #[arg(short = 'b', long)]
    bloat: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch interactive fullscreen TUI dashboard (loads JSON report or scans world)
    Tui {
        /// Path to world folder or existing JSON report
        target: Option<PathBuf>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        spark: Option<PathBuf>,
        #[arg(long)]
        rcon_address: Option<std::net::SocketAddr>,
    },
    /// Launch real-time live server monitoring TUI with latency sparklines & player load
    Watch {
        /// Server address to monitor
        address: String,
    },

    /// Ping a live Minecraft server using SLP (Server List Ping) protocol
    Ping {
        /// Host or host:port (defaults to port 25565)
        address: String,
    },
    /// Execute a command on a live Minecraft server via Source RCON
    Rcon {
        /// Server address host:port (defaults to port 25575)
        address: String,
        /// RCON password
        #[arg(short, long, env = "CHUNKLENS_RCON_PASSWORD", hide_env_values = true)]
        password: Option<String>,
        /// Command to execute (e.g. "tps" or "list")
        command: String,
    },
    /// Ingest and analyze a Spark profiler report JSON
    Spark {
        /// Path to spark report JSON file
        report: PathBuf,
    },
    /// Scan a world directly, without starting a server
    Scan {
        world: PathBuf,
        /// Maximum worker threads (defaults to CPU cores - 1 to keep OS responsive)
        #[arg(long)]
        threads: Option<usize>,
        /// Use 100% of all CPU cores
        #[arg(long, conflicts_with = "eco")]
        fast: bool,
        /// Low CPU usage (half cores) for background scanning
        #[arg(long, conflicts_with = "fast")]
        eco: bool,
        /// Minimum chunk score to export (default 0.1 to save gigabytes of RAM/disk, use 0 for all)
        #[arg(long, default_value_t = 0.1)]
        min_score: f64,
        #[arg(long)]
        json: Option<PathBuf>,
        #[arg(long)]
        csv: Option<PathBuf>,
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// Inspect NBT size, oversized chunks (>1MB), and dense storage
        #[arg(short = 'b', long)]
        bloat: bool,
    },
    /// Load an existing JSON report for instant offline analysis without rescanning
    Load {
        report: PathBuf,
        #[arg(long, default_value_t = 10)]
        top: usize,
        /// Inspect NBT size, oversized chunks (>1MB), and dense storage
        #[arg(short = 'b', long)]
        bloat: bool,
    },
    /// Discover installed Minecraft servers
    Servers,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(cmd) = cli.command {
        match cmd {
            Command::Scan {
                world,
                threads,
                fast,
                eco,
                min_score,
                json,
                csv,
                top,
                bloat,
            } => {
                let options = analysis::ScanOptions::new(threads, fast, eco, min_score);
                run_scan(world, options, json, csv, top, bloat)?;
            }
            Command::Tui {
                target,
                host,
                spark,
                rcon_address,
            } => {
                launch_unified_tui(target, host, spark, rcon_address)?;
            }
            Command::Watch { address } => {
                launch_unified_tui(None, Some(address), None, None)?;
            }
            Command::Ping { address } => {
                let (host, socket_addr) = protocol::slp::resolve_server(&address, 25565, true)?;
                let ping = protocol::slp::ping_server(
                    &host,
                    socket_addr,
                    std::time::Duration::from_secs(4),
                )?;

                let lat_col = match ping.latency_ms {
                    ms if ms < 50 => format!("\x1b[38;2;52;211;153m{ms}ms\x1b[0m"),
                    ms if ms < 150 => format!("\x1b[38;2;245;158;11m{ms}ms\x1b[0m"),
                    ms => format!("\x1b[38;2;244;63;94m{ms}ms\x1b[0m"),
                };

                println!(
                    "\n  \x1b[1;38;2;241;245;249mServer Ping: {}\x1b[0m",
                    address
                );
                println!(
                    "  \x1b[38;2;71;85;105m──────────────────────────────────────────────\x1b[0m"
                );
                println!(
                    "  \x1b[38;2;148;163;184mStatus:  \x1b[0m \x1b[1;38;2;52;211;153mONLINE\x1b[0m (latency: {})",
                    lat_col
                );
                println!(
                    "  \x1b[38;2;148;163;184mPlayers: \x1b[0m \x1b[1;38;2;241;245;249m{}/{}\x1b[0m",
                    ping.online_players, ping.max_players
                );
                println!(
                    "  \x1b[38;2;148;163;184mVersion: \x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m \x1b[38;2;100;116;139m(protocol {})\x1b[0m",
                    ping.version_name, ping.protocol_version
                );
                if !ping.description_clean.is_empty() {
                    println!(
                        "  \x1b[38;2;148;163;184mMOTD:    \x1b[0m {}",
                        discovery::ansi_motd(&ping.description_clean)
                    );
                }
                println!(
                    "  \x1b[38;2;71;85;105m──────────────────────────────────────────────\x1b[0m\n"
                );
            }
            Command::Rcon {
                address,
                password,
                command,
            } => {
                let (_, socket_addr) = protocol::slp::resolve_server(&address, 25575, false)?;
                let pwd = match password {
                    Some(pwd) => pwd,
                    None => std::env::var("RCON_PASSWORD").unwrap_or_else(|_| {
                        rpassword::prompt_password("RCON password: ").unwrap_or_default()
                    }),
                };
                let mut client = protocol::rcon::RconClient::connect(
                    socket_addr,
                    &pwd,
                    std::time::Duration::from_secs(5),
                )?;
                println!(
                    "\n  \x1b[1;32m✔\x1b[0m Connected to RCON at \x1b[1m{}\x1b[0m",
                    socket_addr
                );
                println!("  \x1b[90mExecuting:\x1b[0m \x1b[1m/{}\x1b[0m\n", command);
                let resp = client.execute(&command)?;
                for line in resp.lines() {
                    println!("  {}", line);
                }
                println!();
            }
            Command::Spark { report } => {
                let summary = protocol::spark::parse_spark_report(&report)?;
                print_spark_summary(&summary);
            }
            Command::Load { report, top, bloat } => {
                let result = ScanResult::read_json(&report)?;
                print_scan(&result, top, bloat);
            }
            Command::Servers => interactive_discovery()?,
        }
        return Ok(());
    }

    let default_options = analysis::ScanOptions::new(cli.threads, cli.fast, cli.eco, cli.min_score);

    if let Some(world) = cli.world {
        run_scan(
            world,
            default_options,
            cli.json,
            cli.csv,
            cli.top,
            cli.bloat,
        )?;
        return Ok(());
    }

    // Auto-detect world in current working directory
    let cwd_world = PathBuf::from("world");
    let cwd_region = PathBuf::from("region");
    if cwd_world.join("region").is_dir() || cwd_world.is_dir() {
        run_scan(
            cwd_world,
            default_options,
            cli.json,
            cli.csv,
            cli.top,
            cli.bloat,
        )?;
        return Ok(());
    } else if cwd_region.is_dir() {
        run_scan(
            PathBuf::from("."),
            default_options,
            cli.json,
            cli.csv,
            cli.top,
            cli.bloat,
        )?;
        return Ok(());
    }

    print_quick_help();
    Ok(())
}

fn run_scan(
    world: PathBuf,
    options: analysis::ScanOptions,
    json: Option<PathBuf>,
    csv: Option<PathBuf>,
    top: usize,
    bloat: bool,
) -> Result<()> {
    let min_score = options.min_score;
    let result = analysis::scan_world(&world, options)?;
    print_scan(&result, top, bloat);
    if let Some(path) = json {
        result.write_json(&path, min_score)?;
        println!(
            "  \x1b[32m✔\x1b[0m JSON report written to {}",
            path.display()
        );
    }
    if let Some(path) = csv {
        result.write_csv(&path, min_score)?;
        println!(
            "  \x1b[32m✔\x1b[0m CSV report written to {}",
            path.display()
        );
    }
    Ok(())
}

fn print_quick_help() {
    println!(
        "\n  \x1b[1;37mChunkLens\x1b[0m \x1b[96mv{}\x1b[0m \x1b[90m— Fast Minecraft Anvil world scanner & lag investigator\x1b[0m\n",
        env!("CARGO_PKG_VERSION")
    );
    println!("  \x1b[1;37mUSAGE:\x1b[0m");
    println!(
        "    \x1b[96mchunklens [PATH]\x1b[0m           Scan world directory (e.g. \x1b[90mchunklens ./world\x1b[0m)"
    );
    println!(
        "    \x1b[96mchunklens -n 20\x1b[0m            Show top 20 potential load chunks (default: 10)"
    );
    println!("    \x1b[96mchunklens tui\x1b[0m              Interactive fullscreen TUI dashboard");
    println!(
        "    \x1b[96mchunklens ping <addr>\x1b[0m      Ping server via SLP (e.g. \x1b[90mchunklens ping localhost\x1b[0m)"
    );
    println!("    \x1b[96mchunklens spark <file>\x1b[0m     Analyze a Spark profile report");
    println!("    \x1b[96mchunklens servers\x1b[0m          Discover local Minecraft servers");
    println!("    \x1b[96mchunklens --help\x1b[0m           Show all CLI options and flags\n");
}

fn launch_unified_tui(
    target: Option<PathBuf>,
    host: Option<String>,
    spark: Option<PathBuf>,
    rcon_address: Option<std::net::SocketAddr>,
) -> Result<()> {
    let scan = match target {
        Some(path) if path.is_file() => ScanResult::read_json(&path)?,
        Some(path) if path.join("region").is_dir() => {
            analysis::scan_world(&path, analysis::ScanOptions::default())?
        }
        Some(path) => {
            let worlds = discover_worlds(&path)?;
            let world = worlds
                .first()
                .ok_or_else(|| anyhow::anyhow!("no world or report found at {}", path.display()))?;
            analysis::scan_world(&world.path, analysis::ScanOptions::default())?
        }
        None => ScanResult::default(),
    };
    let spark = spark
        .as_deref()
        .map(protocol::spark::parse_spark_report)
        .transpose()?;
    let password = std::env::var("CHUNKLENS_RCON_PASSWORD").ok();
    tui::run_tui(scan, host, spark, rcon_address, password)
}

fn print_spark_summary(summary: &protocol::spark::SparkProfileSummary) {
    let mspt_col = summary.average_mspt.map_or_else(
        || "not recorded".to_owned(),
        |value| format!("{value:.1} ms (measured, last minute)"),
    );

    println!(
        "\n  \x1b[38;2;99;102;241m╭─\x1b[0m \x1b[1;38;2;248;250;252mSpark Profiler Analysis\x1b[0m \x1b[38;2;71;85;105m────────────────────────────────────────────╮\x1b[0m"
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mPlatform: \x1b[0m \x1b[38;2;226;232;240m{}\x1b[0m  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mTicks Profiled:\x1b[0m {}",
        summary.server_version, summary.number_of_ticks
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mDuration: \x1b[0m {:.1}s  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mAverage MSPT:  \x1b[0m {}",
        summary.duration_seconds, mspt_col
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mBreakdown:\x1b[0m Entities: \x1b[38;2;245;158;11m{:.1}%\x1b[0m │ Block Entities: \x1b[38;2;245;158;11m{:.1}%\x1b[0m │ Redstone: \x1b[38;2;245;158;11m{:.1}%\x1b[0m │ Chunks: \x1b[38;2;245;158;11m{:.1}%\x1b[0m",
        summary.category_breakdown.entities_pct,
        summary.category_breakdown.tile_entities_pct,
        summary.category_breakdown.redstone_pct,
        summary.category_breakdown.chunk_loading_pct
    );
    println!(
        "  \x1b[38;2;71;85;105m╰──────────────────────────────────────────────────────────────────────────╯\x1b[0m\n"
    );

    if !summary.top_consumers.is_empty() {
        println!("  \x1b[1;38;2;241;245;249mTOP METHODS BY SELF SAMPLE SHARE\x1b[0m\n");
        for (i, m) in summary.top_consumers.iter().enumerate() {
            println!(
                "    \x1b[1;38;2;129;140;248m#{:<2}\x1b[0m \x1b[1;38;2;244;63;94m{:>5.1}%\x1b[0m  \x1b[38;2;226;232;240m{}.{}\x1b[0m",
                i + 1,
                m.percentage,
                m.class_name,
                m.method_name
            );
        }
        println!();
    }
}

pub fn print_banner() {
    println!("\x1b[38;2;99;102;241m");
    println!("  ╭───────────────────────────────────────────────────────────────────╮");
    println!(
        "  │  \x1b[1;38;2;248;250;252mChunkLens\x1b[0;38;2;99;102;241m \x1b[38;2;129;140;248mv{}\x1b[0;38;2;99;102;241m   \x1b[38;2;148;163;184mFast Anvil World Analyzer & Lag Investigator\x1b[0;38;2;99;102;241m      │",
        env!("CARGO_PKG_VERSION")
    );
    println!("  ╰───────────────────────────────────────────────────────────────────╯\x1b[0m\n");
}

fn interactive_discovery() -> Result<()> {
    print_banner();
    println!(
        "  \x1b[38;2;129;140;248m⠋\x1b[0m \x1b[1;38;2;241;245;249mScanning filesystem for Minecraft server instances...\x1b[0m\n"
    );
    let servers = discover_servers(false)?;
    if servers.is_empty() {
        use std::io::{self, Write};
        println!(
            "  \x1b[38;2;251;146;60mNo Minecraft servers discovered automatically.\x1b[0m\n\n  \x1b[1;38;2;99;102;241m[F]\x1b[0m Scan entire filesystem (/)\n  \x1b[1;38;2;99;102;241m[P]\x1b[0m Enter path manually\n  \x1b[1;38;2;148;163;184m[Q]\x1b[0m Quit"
        );
        print!("\n  \x1b[1;38;2;99;102;241m❯\x1b[0m \x1b[1;38;2;241;245;249mChoice: \x1b[0m");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        match choice.trim().to_ascii_lowercase().as_str() {
            "f" => {
                println!(
                    "  \x1b[38;2;148;163;184mScanning entire filesystem... this may take a moment.\x1b[0m"
                );
                let all = discover_servers(true)?;
                if all.is_empty() {
                    println!("  \x1b[38;2;244;63;94m✖ No Minecraft servers found.\x1b[0m");
                    return Ok(());
                }
                print_servers(&all);
                let selected = choose_server(&all)?;
                return shell::run(selected.clone(), discover_worlds(&selected.path)?);
            }
            "p" => {
                print!(
                    "  \x1b[1;38;2;99;102;241m❯\x1b[0m \x1b[1;38;2;241;245;249mEnter server directory path: \x1b[0m"
                );
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let path = PathBuf::from(input.trim());
                let server = server_at(&path)?;
                let worlds = if path.join("region").is_dir() {
                    vec![discovery::World {
                        name: path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("world")
                            .to_owned(),
                        path: path.clone(),
                        size_bytes: 0,
                    }]
                } else {
                    discover_worlds(&path)?
                };
                return shell::run(server, worlds);
            }
            _ => return Ok(()),
        }
    }
    print_servers(&servers);
    let selected = choose_server(&servers)?;
    let worlds = discover_worlds(&selected.path)?;
    shell::run(selected, worlds)
}

fn choose_server(servers: &[Server]) -> Result<Server> {
    use std::io::{self, Write};
    loop {
        print!(
            "  \x1b[1;38;2;99;102;241m❯\x1b[0m \x1b[1;38;2;248;250;252mSelect server \x1b[0;38;2;148;163;184m[1-{}]\x1b[0m \x1b[38;2;100;116;139m(or 'q' to quit):\x1b[0m ",
            servers.len()
        );
        io::stdout().flush()?;
        let mut input = String::new();
        anyhow::ensure!(
            io::stdin().read_line(&mut input)? > 0,
            "server selection input closed"
        );
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("q") || trimmed.eq_ignore_ascii_case("exit") {
            std::process::exit(0);
        }
        if let Ok(index) = trimmed.parse::<usize>()
            && index > 0
            && let Some(server) = servers.get(index - 1)
        {
            return Ok(server.clone());
        }
        println!(
            "  \x1b[38;2;244;63;94m✖ Invalid selection. Please enter a number between 1 and {}.\x1b[0m",
            servers.len()
        );
    }
}

fn print_servers(servers: &[Server]) {
    println!(
        "  \x1b[1;38;2;241;245;249mDiscovered Servers\x1b[0m  \x1b[38;2;100;116;139m({} instances detected)\x1b[0m\n",
        servers.len()
    );
    for (index, server) in servers.iter().enumerate() {
        let (status_dot, status_badge) = if server.running {
            let badge = if let (Some(online), Some(max), Some(lat)) =
                (server.online_players, server.max_players, server.latency_ms)
            {
                format!(
                    "\x1b[1;38;2;52;211;153mONLINE\x1b[0m \x1b[38;2;148;163;184m({online}/{max} • {lat}ms)\x1b[0m"
                )
            } else {
                "\x1b[1;38;2;52;211;153mONLINE\x1b[0m".to_string()
            };
            ("\x1b[38;2;52;211;153m●\x1b[0m", badge)
        } else {
            (
                "\x1b[38;2;100;116;139m○\x1b[0m",
                "\x1b[38;2;100;116;139mOFFLINE\x1b[0m".to_string(),
            )
        };
        let software = server.software.as_deref().unwrap_or("Server");
        let port_str = server.port.map_or("?".to_owned(), |p| p.to_string());

        let badge_plain_len = if server.running {
            if let (Some(online), Some(max), Some(lat)) =
                (server.online_players, server.max_players, server.latency_ms)
            {
                format!("ONLINE ({online}/{max} • {lat}ms)").len() + 4
            } else {
                10
            }
        } else {
            11
        };

        let header_prefix_len = 8; // "  ╭─ #1 "
        let name_len = server.name.chars().count();
        let target_width = 82usize.max(header_prefix_len + name_len + badge_plain_len + 8);
        let dash_count = target_width
            .saturating_sub(header_prefix_len + name_len + badge_plain_len + 4)
            .max(2);
        let dashes = "─".repeat(dash_count);
        let bottom_bar = "─".repeat(target_width - 4);

        println!(
            "  \x1b[38;2;71;85;105m╭─\x1b[0m \x1b[1;38;2;129;140;248m#{:<2}\x1b[0m \x1b[1;38;2;248;250;252m{}\x1b[0m \x1b[38;2;71;85;105m{}\x1b[0m [ {} {} ] \x1b[38;2;71;85;105m─╮\x1b[0m",
            index + 1,
            server.name,
            dashes,
            status_dot,
            status_badge
        );
        println!(
            "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mDirectory:\x1b[0m \x1b[38;2;226;232;240m{}\x1b[0m",
            server.path.display()
        );
        println!(
            "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mPlatform: \x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mPort:\x1b[0m \x1b[38;2;245;158;11m{}\x1b[0m",
            software, port_str
        );
        if let Some(motd) = &server.motd {
            let clean = discovery::ansi_motd(motd);
            let mut lines = clean.lines();
            if let Some(first) = lines.next() {
                println!(
                    "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mMOTD:     \x1b[0m {}",
                    first
                );
                for rest in lines {
                    println!("  \x1b[38;2;71;85;105m│\x1b[0m            {}", rest);
                }
            }
        }
        println!("  \x1b[38;2;71;85;105m╰{}╯\x1b[0m\n", bottom_bar);
    }
}

pub fn print_scan(result: &ScanResult, count: usize, bloat_only: bool) {
    if bloat_only {
        let bloat_chunks = result.top_by("bloat", count);
        if bloat_chunks.is_empty() {
            println!(
                "\n  \x1b[1;92m✔\x1b[0m No oversized chunks or NBT bloat hazards detected in this world.\n"
            );
            return;
        }

        println!(
            "\n  \x1b[1;37mNBT BLOAT & DUPE HAZARDS\x1b[0m \x1b[90m(Ranked by NBT payload size and stash risk)\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );
        println!(
            "  \x1b[90m #   Size / Hazard           Teleport Command       Details & Culprits\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );

        for (index, metric) in bloat_chunks.iter().enumerate() {
            let (badge_plain, badge_colored) = metric.bloat_hazard_badge();
            let size_str = if metric.payload_size >= 1024 * 1024 {
                format!("{:.2} MiB", metric.payload_size as f64 / 1_048_576.0)
            } else {
                format!("{} KiB", metric.payload_size / 1024)
            };
            let badge_padding = " ".repeat(12usize.saturating_sub(badge_plain.len()));
            let size_padding = " ".repeat(8usize.saturating_sub(size_str.len()));

            println!(
                "  \x1b[90m{:>2}\x1b[0m   {}{}{}{}   \x1b[1;96m{:<22}\x1b[0m {}",
                index + 1,
                badge_colored,
                badge_padding,
                size_padding,
                size_str,
                metric.tp_command(),
                metric.bloat_details()
            );
        }
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m\n"
        );
        return;
    }

    let top_chunks = result.top_by("score", count);
    if top_chunks.is_empty() {
        println!("\n  \x1b[1;92m✔\x1b[0m No retained chunks with potential load signals.\n");
    } else {
        println!(
            "\n  \x1b[1;37mTOP POTENTIAL LOAD CHUNKS\x1b[0m \x1b[90m(Offline heuristic; confirm with live profiling)\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );
        println!("  \x1b[90m #   Score   Teleport Command       Culprits & Details\x1b[0m");
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );

        for (index, metric) in top_chunks.iter().enumerate() {
            let score_str = format!("{:>5.1}", metric.score);
            let score_colored = match metric.score {
                s if s >= 80.0 => format!("\x1b[1;91m{}\x1b[0m", score_str),
                s if s >= 60.0 => format!("\x1b[1;93m{}\x1b[0m", score_str),
                s if s >= 40.0 => format!("\x1b[93m{}\x1b[0m", score_str),
                s if s >= 20.0 => format!("\x1b[92m{}\x1b[0m", score_str),
                _ => format!("\x1b[90m{}\x1b[0m", score_str),
            };

            println!(
                "  \x1b[90m{:>2}\x1b[0m   {}   \x1b[1;96m{:<22}\x1b[0m {}",
                index + 1,
                score_colored,
                metric.tp_command(),
                metric.culprits_summary()
            );
        }
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m\n"
        );
    }

    if !result.clusters.is_empty() {
        println!(
            "  \x1b[1;37mMULTI-CHUNK CLUSTERS\x1b[0m \x1b[90m(Farms and bases spanning across chunk borders)\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );
        for (i, c) in result.clusters.iter().take(5).enumerate() {
            let bx = c.center.0 * 16 + 8;
            let bz = c.center.1 * 16 + 8;
            let score_str = format!("{:>5.1}", c.total_score);
            let score_colored = match c.total_score {
                s if s >= 80.0 => format!("\x1b[1;91m{}\x1b[0m", score_str),
                s if s >= 50.0 => format!("\x1b[1;93m{}\x1b[0m", score_str),
                _ => format!("\x1b[92m{}\x1b[0m", score_str),
            };
            let tp_cmd = format!("/tp @s {} ~ {}", bx, bz);
            println!(
                "  \x1b[90m#{}\x1b[0m   Score {}  •  \x1b[1;96m{:<22}\x1b[0m •  {} chunks ({})",
                i + 1,
                score_colored,
                tp_cmd,
                c.chunks.len(),
                c.culprits_summary()
            );
        }
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m\n"
        );
    }

    let bloat_hazards = result.top_by("bloat", 5);
    let hazardous: Vec<_> = bloat_hazards
        .into_iter()
        .filter(|c| c.is_bloat_hazard())
        .collect();
    if !hazardous.is_empty() {
        println!(
            "  \x1b[1;38;2;251;146;60m⚠️  NBT BLOAT & DUPE HAZARDS\x1b[0m \x1b[38;2;148;163;184m(Extreme data sizes, stashes, or crash risks)\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );
        println!(
            "  \x1b[90m #   Hazard / Size           Teleport Command       Details & Culprits\x1b[0m"
        );
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m"
        );
        for (i, m) in hazardous.iter().enumerate() {
            let (badge_plain, badge_colored) = m.bloat_hazard_badge();
            let size_str = if m.payload_size >= 1024 * 1024 {
                format!("{:.2} MiB", m.payload_size as f64 / 1_048_576.0)
            } else {
                format!("{} KiB", m.payload_size / 1024)
            };
            let badge_padding = " ".repeat(12usize.saturating_sub(badge_plain.len()));
            let size_padding = " ".repeat(8usize.saturating_sub(size_str.len()));
            println!(
                "  \x1b[90m{:>2}\x1b[0m   {}{}{}{}   \x1b[1;96m{:<22}\x1b[0m {}",
                i + 1,
                badge_colored,
                badge_padding,
                size_padding,
                size_str,
                m.tp_command(),
                m.bloat_details()
            );
        }
        println!(
            "  \x1b[90m────────────────────────────────────────────────────────────────────────────────────────\x1b[0m\n"
        );
    }
}
