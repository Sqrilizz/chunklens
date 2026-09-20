use crate::{
    analysis::{self, ScanOptions},
    discovery::{Server, World},
    model::{ChunkMetrics, ScanResult},
};
use anyhow::Result;
use rustyline::{DefaultEditor, error::ReadlineError};
use std::path::PathBuf;

pub fn run(server: Server, worlds: Vec<World>) -> Result<()> {
    let (status_dot, status_badge) = if server.running {
        (
            "\x1b[38;2;52;211;153m●\x1b[0m",
            "\x1b[1;38;2;52;211;153mONLINE\x1b[0m",
        )
    } else {
        (
            "\x1b[38;2;100;116;139m○\x1b[0m",
            "\x1b[38;2;100;116;139mOFFLINE\x1b[0m",
        )
    };
    let software = server.software.as_deref().unwrap_or("Server");
    let port_str = server.port.map_or("?".to_owned(), |p| p.to_string());

    println!(
        "\n  \x1b[38;2;71;85;105m╭─\x1b[0m \x1b[1;38;2;248;250;252m{} Shell\x1b[0m \x1b[38;2;71;85;105m──────────────────────────────────────────────\x1b[0m [ {} {} ] \x1b[38;2;71;85;105m─╮\x1b[0m",
        server.name, status_dot, status_badge
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mDirectory:\x1b[0m \x1b[38;2;226;232;240m{}\x1b[0m",
        server.path.display()
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mEngine:   \x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mPort:\x1b[0m \x1b[38;2;245;158;11m{}\x1b[0m",
        software, port_str
    );
    let worlds_summary = if worlds.is_empty() {
        "none".to_string()
    } else {
        worlds
            .iter()
            .map(|w| format!("{} ({:.1}MB)", w.name, w.size_bytes as f64 / 1_048_576.0))
            .collect::<Vec<_>>()
            .join(", ")
    };
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mWorlds:   \x1b[0m \x1b[38;2;52;211;153m{}\x1b[0m",
        worlds_summary
    );
    println!(
        "  \x1b[38;2;71;85;105m╰──────────────────────────────────────────────────────────────────────────╯\x1b[0m"
    );
    println!(
        "  \x1b[38;2;148;163;184mType \x1b[1;38;2;248;250;252m'scan'\x1b[0;38;2;148;163;184m to analyze world, \x1b[1;38;2;248;250;252m'top'\x1b[0;38;2;148;163;184m to view heaviest chunks, or \x1b[1;38;2;248;250;252m'help'\x1b[0;38;2;148;163;184m for commands.\x1b[0m\n"
    );

    if server.running {
        println!(
            "  \x1b[38;2;251;146;60m⚡ Notice: Server is running. ChunkLens is read-only; live saves may occur during scan.\x1b[0m\n"
        );
    }
    let mut selected = worlds.first().cloned();
    let mut result: Option<ScanResult> = None;
    let mut editor = DefaultEditor::new()?;
    loop {
        let clean_prompt_name = server.name.to_lowercase().replace([' ', '/', '\\'], "-");
        let prompt = format!(
            "  \x1b[1;38;2;99;102;241m{}\x1b[0m \x1b[1;38;2;52;211;153m❯\x1b[0m ",
            clean_prompt_name
        );
        match editor.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                editor.add_history_entry(line)?;
                let normalized = line.strip_prefix('/').unwrap_or(line);
                let parts: Vec<_> = normalized.split_whitespace().collect();
                match parts.as_slice() {
                    ["clear"] => {
                        use std::io::Write;
                        print!("\x1b[2J\x1b[1;1H");
                        let _ = std::io::stdout().flush();
                    }
                    ["help"] => {
                        println!(
                            "\n  \x1b[1;38;2;248;250;252mChunkLens Commands\x1b[0m  \x1b[38;2;100;116;139m(Slash commands like /scan and /top supported)\x1b[0m\n"
                        );
                        println!("  \x1b[1;38;2;129;140;248mAnalysis & TUI Dashboard\x1b[0m");
                        println!(
                            "    \x1b[38;2;56;189;248mtui / dashboard\x1b[0m                                 Launch interactive fullscreen TUI dashboard"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mscan\x1b[0m \x1b[38;2;100;116;139m[--eco|--fast|--threads N|--min-score X]\x1b[0m   Scan Anvil world for lag hotspots"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mtop\x1b[0m \x1b[38;2;100;116;139m[N] [category]\x1b[0m                            Inspect heaviest chunks (villagers, hoppers...)"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mbloat\x1b[0m \x1b[38;2;100;116;139m[N]\x1b[0m                                          Detect NBT bloat, oversized chunks & dupe stashes"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248minspect\x1b[0m \x1b[38;2;245;158;11m<chunk_x> <chunk_z>\x1b[0m                       Full NBT entity & tile diagnostic for chunk"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mclusters\x1b[0m                                      List connected multi-chunk lag complexes"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mcluster\x1b[0m \x1b[38;2;245;158;11m<chunk_x> <chunk_z>\x1b[0m                       Show cluster containing chunk\n"
                        );

                        println!("  \x1b[1;38;2;129;140;248mEnvironment & Dimensions\x1b[0m");
                        println!(
                            "    \x1b[38;2;56;189;248mworlds\x1b[0m                                        List detected world dimensions"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248muse\x1b[0m \x1b[38;2;245;158;11m<world>\x1b[0m                                       Switch active world (world_nether, world_the_end)"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mfind\x1b[0m \x1b[38;2;100;116;139mentity|block|block-entity <id>\x1b[0m            Search occurrences across all chunks"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mstats\x1b[0m                                         Global world totals & entity counts"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248minfo\x1b[0m                                          Display server engine, ports, and paths\n"
                        );
                        println!("  \x1b[1;38;2;129;140;248mReports & Live Server Protocol\x1b[0m");
                        println!(
                            "    \x1b[38;2;56;189;248mrcon\x1b[0m \x1b[38;2;245;158;11m<command>\x1b[0m                                   Execute live command via Source RCON (/tps, /kill)"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mping\x1b[0m \x1b[38;2;100;116;139m[port]\x1b[0m                                       Query live server latency, online players & MOTD"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mexport\x1b[0m \x1b[38;2;100;116;139mjson|csv <file>\x1b[0m                            Export scan report to disk"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mclear\x1b[0m                                         Clear terminal screen"
                        );
                        println!(
                            "    \x1b[38;2;56;189;248mback / exit / quit\x1b[0m                             Exit ChunkLens\n"
                        );
                    }
                    ["ping", target @ ..] => {
                        let port = target
                            .first()
                            .and_then(|p| p.parse::<u16>().ok())
                            .or(server.port)
                            .unwrap_or(25565);
                        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
                        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
                        match crate::protocol::slp::ping_server(
                            "localhost",
                            addr,
                            std::time::Duration::from_secs(3),
                        ) {
                            Ok(res) => {
                                println!(
                                    "  \x1b[1;38;2;52;211;153m✔ Live SLP Ping:\x1b[0m {}ms │ Players: {}/{} │ Version: {} │ MOTD: {}",
                                    res.latency_ms,
                                    res.online_players,
                                    res.max_players,
                                    res.version_name,
                                    res.description_clean
                                );
                            }
                            Err(e) => println!(
                                "  \x1b[38;2;244;63;94m✖ Server not responding on port {port}: {e}\x1b[0m"
                            ),
                        }
                    }
                    ["rcon", cmd_parts @ ..] => {
                        let cmd = cmd_parts.join(" ");
                        if cmd.is_empty() {
                            println!("  Usage: rcon <command>  (e.g. rcon tps, rcon list)");
                            continue;
                        }
                        let prop_path = server.path.join("server.properties");
                        let rcon_pwd = if prop_path.exists() {
                            let content = std::fs::read_to_string(&prop_path).unwrap_or_default();
                            content
                                .lines()
                                .find(|l| l.starts_with("rcon.password="))
                                .map(|l| l.trim_start_matches("rcon.password=").to_string())
                        } else {
                            None
                        };
                        let rcon_port = if prop_path.exists() {
                            let content = std::fs::read_to_string(&prop_path).unwrap_or_default();
                            content
                                .lines()
                                .find(|l| l.starts_with("rcon.port="))
                                .and_then(|l| {
                                    l.trim_start_matches("rcon.port=").parse::<u16>().ok()
                                })
                                .unwrap_or(25575)
                        } else {
                            25575
                        };

                        let pwd = match rcon_pwd {
                            Some(p) if !p.is_empty() => p,
                            _ => std::env::var("CHUNKLENS_RCON_PASSWORD")
                                .ok()
                                .map(Ok)
                                .unwrap_or_else(|| rpassword::prompt_password("RCON password: "))?,
                        };

                        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
                        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), rcon_port);
                        match crate::protocol::rcon::RconClient::connect(
                            addr,
                            &pwd,
                            std::time::Duration::from_secs(3),
                        ) {
                            Ok(mut client) => match client.execute(&cmd) {
                                Ok(resp) => {
                                    println!("  \x1b[1;38;2;52;211;153m[RCON Output]:\x1b[0m");
                                    for line in resp.lines() {
                                        println!("    {line}");
                                    }
                                }
                                Err(e) => println!(
                                    "  \x1b[38;2;244;63;94m✖ RCON execution failed: {e}\x1b[0m"
                                ),
                            },
                            Err(e) => println!(
                                "  \x1b[38;2;244;63;94m✖ Failed to connect to RCON on port {rcon_port}: {e}\x1b[0m"
                            ),
                        }
                    }

                    ["info"] => println!(
                        "  Server: {}\n  Path: {}\n  Port: {}\n  Active World: {}",
                        server.name,
                        server.path.display(),
                        server.port.map_or("?".to_owned(), |p| p.to_string()),
                        selected.as_ref().map_or("none", |w| w.name.as_str())
                    ),
                    ["servers"] => println!(
                        "  Selected server: {} ({})",
                        server.name,
                        server.path.display()
                    ),
                    ["worlds"] => {
                        println!("\n  \x1b[1;38;2;248;250;252mAvailable Worlds:\x1b[0m");
                        for world in &worlds {
                            let is_active = selected.as_ref().is_some_and(|s| s.path == world.path);
                            let prefix = if is_active {
                                "\x1b[1;38;2;52;211;153m●\x1b[0m"
                            } else {
                                "\x1b[38;2;100;116;139m○\x1b[0m"
                            };
                            let active_badge = if is_active {
                                " \x1b[1;38;2;52;211;153m(active)\x1b[0m"
                            } else {
                                ""
                            };
                            println!(
                                "    {} \x1b[1;38;2;248;250;252m{:<20}\x1b[0m \x1b[38;2;148;163;184m{:>8.1} MB\x1b[0m{}",
                                prefix,
                                world.name,
                                world.size_bytes as f64 / 1_048_576.0,
                                active_badge
                            );
                        }
                        println!();
                    }
                    ["use", name] => match worlds.iter().find(|w| w.name == *name) {
                        Some(w) => {
                            selected = Some(w.clone());
                            result = None;
                            println!(
                                "  ✔ Switched active world to \x1b[1;38;2;52;211;153m{}\x1b[0m",
                                w.name
                            );
                        }
                        None => println!("  ✖ Unknown world: {name}. Type 'worlds' to view list."),
                    },
                    ["tui"] | ["dashboard"] | ["ui"] => {
                        let scan_data = match &result {
                            Some(r) => r.clone(),
                            None => {
                                if let Some(world) = &selected {
                                    println!(
                                        "  \x1b[38;2;129;140;248m⠋\x1b[0m Scanning {} before launching TUI dashboard...",
                                        world.name
                                    );
                                    let res =
                                        analysis::scan_world(&world.path, ScanOptions::default())?;
                                    result = Some(res.clone());
                                    res
                                } else {
                                    println!("  ✖ Select a world first or run 'scan'.");
                                    continue;
                                }
                            }
                        };
                        let host = format!("127.0.0.1:{}", server.port.unwrap_or(25565));
                        let prop_path = server.path.join("server.properties");
                        let rcon_pwd = if prop_path.exists() {
                            let content = std::fs::read_to_string(&prop_path).unwrap_or_default();
                            content
                                .lines()
                                .find(|l| l.starts_with("rcon.password="))
                                .map(|l| l.trim_start_matches("rcon.password=").to_string())
                        } else {
                            None
                        };
                        let rcon_port = if prop_path.exists() {
                            let content = std::fs::read_to_string(&prop_path).unwrap_or_default();
                            content
                                .lines()
                                .find(|l| l.starts_with("rcon.port="))
                                .and_then(|l| {
                                    l.trim_start_matches("rcon.port=").parse::<u16>().ok()
                                })
                                .unwrap_or(25575)
                        } else {
                            25575
                        };
                        let rcon_addr = format!("127.0.0.1:{rcon_port}").parse().ok();
                        let spark = if PathBuf::from("spark-report.json").exists() {
                            crate::protocol::spark::parse_spark_report(&PathBuf::from(
                                "spark-report.json",
                            ))
                            .ok()
                        } else {
                            None
                        };
                        crate::tui::run_tui(scan_data, Some(host), spark, rcon_addr, rcon_pwd)?;
                    }
                    ["scan", rest @ ..] => {
                        if let Some(world) = &selected {
                            let threads = rest
                                .windows(2)
                                .find(|p| p[0] == "--threads")
                                .and_then(|p| p[1].parse().ok());
                            let fast = rest.contains(&"--fast");
                            let eco = rest.contains(&"--eco");
                            let min_score = rest
                                .windows(2)
                                .find(|p| p[0] == "--min-score")
                                .and_then(|p| p[1].parse().ok())
                                .unwrap_or(0.1);
                            let res = analysis::scan_world(
                                &world.path,
                                ScanOptions::new(threads, fast, eco, min_score),
                            )?;
                            crate::print_scan(&res, 10, false);
                            result = Some(res);
                        } else {
                            println!("  ✖ Select a world first using 'use <world>'");
                        }
                    }
                    ["top", args @ ..] => match &result {
                        Some(r) => {
                            let category = args
                                .iter()
                                .copied()
                                .find(|v| v.parse::<usize>().is_err())
                                .unwrap_or("score");
                            let count = args.iter().find_map(|v| v.parse().ok()).unwrap_or(10);
                            if category == "score" {
                                crate::print_scan(r, count, false);
                            } else if category == "bloat" || category == "nbt" || category == "size"
                            {
                                crate::print_scan(r, count, true);
                            } else {
                                print_top(r, category, count);
                            }
                        }
                        None => println!("  ✖ Run 'scan' first to populate data."),
                    },
                    ["bloat", args @ ..] => match &result {
                        Some(r) => {
                            let count = args.iter().find_map(|v| v.parse().ok()).unwrap_or(10);
                            crate::print_scan(r, count, true);
                        }
                        None => println!("  ✖ Run 'scan' first to populate data."),
                    },
                    ["inspect", x, z] => match (&result, x.parse(), z.parse()) {
                        (Some(r), Ok(x), Ok(z)) => match r.find(x, z) {
                            Some(c) => inspect(c),
                            None => println!("  ✖ Chunk ({x}, {z}) not found in scan results."),
                        },
                        _ => println!("  Usage: inspect <chunk_x> <chunk_z>"),
                    },
                    ["clusters"] => match &result {
                        Some(r) => {
                            if r.clusters.is_empty() {
                                println!("  No heavy multi-chunk clusters detected.");
                            } else {
                                println!(
                                    "\n  \x1b[1;38;2;241;245;249mConnected Heavy Clusters\x1b[0m  \x1b[38;2;100;116;139m({} complexes found)\x1b[0m\n",
                                    r.clusters.len()
                                );
                                for (i, c) in r.clusters.iter().enumerate() {
                                    let bx = c.center.0 * 16 + 8;
                                    let bz = c.center.1 * 16 + 8;
                                    println!(
                                        "  \x1b[38;2;71;85;105m╭─\x1b[0m \x1b[1;38;2;129;140;248m#{:<2}\x1b[0m \x1b[1;38;2;248;250;252mCenter [ {}, {} ]\x1b[0m \x1b[38;2;71;85;105m─────────────────────────\x1b[0m [ Combined Score: \x1b[1;38;2;244;63;94m{:.1}\x1b[0m ] \x1b[38;2;71;85;105m─╮\x1b[0m",
                                        i + 1,
                                        bx,
                                        bz,
                                        c.total_score
                                    );
                                    println!(
                                        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mArea:\x1b[0m {} chunks  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mEntities:\x1b[0m {}  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mHoppers:\x1b[0m {}  \x1b[38;2;71;85;105m•\x1b[0m  \x1b[38;2;148;163;184mVillagers:\x1b[0m {}",
                                        c.chunks.len(),
                                        c.entities,
                                        c.hoppers,
                                        c.villagers
                                    );
                                    println!(
                                        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mTeleport:\x1b[0m \x1b[38;2;56;189;248m/tp @s {} ~ {}\x1b[0m",
                                        bx, bz
                                    );
                                    println!(
                                        "  \x1b[38;2;71;85;105m╰──────────────────────────────────────────────────────────────────────────╯\x1b[0m\n"
                                    );
                                }
                            }
                        }
                        None => println!("  ✖ Run 'scan' first."),
                    },
                    ["cluster", x, z] => match (&result, x.parse::<i32>(), z.parse::<i32>()) {
                        (Some(r), Ok(x), Ok(z)) => {
                            match r.clusters.iter().find(|c| c.chunks.contains(&(x, z))) {
                                Some(c) => {
                                    let bx = c.center.0 * 16 + 8;
                                    let bz = c.center.1 * 16 + 8;
                                    println!(
                                        "\n  Cluster at center ({}, {}) ➔ Block [{}, {}]: {} chunks, combined score {:.1}, entities {}, villagers {}, hoppers {} | \x1b[38;2;56;189;248m/tp @s {} ~ {}\x1b[0m\n",
                                        c.center.0,
                                        c.center.1,
                                        bx,
                                        bz,
                                        c.chunks.len(),
                                        c.total_score,
                                        c.entities,
                                        c.villagers,
                                        c.hoppers,
                                        bx,
                                        bz
                                    );
                                }
                                None => println!("  Chunk is not part of a heavy cluster."),
                            }
                        }
                        _ => println!("  Usage: cluster <chunk_x> <chunk_z>"),
                    },
                    ["find", "entity", id] => find(result.as_ref(), id, true),
                    ["find", "block", id] => find_blocks(result.as_ref(), id),
                    ["find", "block-entity", id] => find(result.as_ref(), id, false),
                    ["stats"] => match &result {
                        Some(r) => println!(
                            "\n  Chunks: {}  •  Entities: {}  •  Block Entities: {}  •  Combined Score: {:.1}  •  Clusters: {}\n",
                            r.chunks.len(),
                            r.chunks.iter().map(|c| c.entity_count).sum::<u64>(),
                            r.chunks.iter().map(|c| c.block_entity_count).sum::<u64>(),
                            r.chunks.iter().map(|c| c.score).sum::<f64>(),
                            r.clusters.len()
                        ),
                        None => println!("  ✖ Run 'scan' first."),
                    },
                    ["export", "json", file] => match &result {
                        Some(r) => {
                            r.write_json(&PathBuf::from(file), 0.0)?;
                            println!(
                                "  ✔ Exported JSON report to \x1b[38;2;52;211;153m{file}\x1b[0m"
                            )
                        }
                        None => println!("  ✖ Run 'scan' first."),
                    },
                    ["export", "csv", file] => match &result {
                        Some(r) => {
                            r.write_csv(&PathBuf::from(file), 0.0)?;
                            println!(
                                "  ✔ Exported CSV report to \x1b[38;2;52;211;153m{file}\x1b[0m"
                            )
                        }
                        None => println!("  ✖ Run 'scan' first."),
                    },
                    ["back"] => return Ok(()),
                    ["exit"] | ["quit"] => std::process::exit(0),
                    _ => println!(
                        "  Unknown command. Type \x1b[1;38;2;248;250;252m'help'\x1b[0m for available commands."
                    ),
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}

fn print_top(result: &ScanResult, category: &str, count: usize) {
    println!(
        "\n  \x1b[1;38;2;241;245;249mTOP CHUNKS BY {}\x1b[0m\n",
        category.to_ascii_uppercase()
    );
    for (i, c) in result.top_by(category, count).iter().enumerate() {
        println!(
            "  \x1b[38;2;129;140;248m#{:<2}\x1b[0m Chunk ({:>5}, {:>5}) ➔ Block [{:>7}, {:>7}]  Score: {:>5.1} ({})  Ent:{} Vil:{} Hop:{}  |  \x1b[38;2;56;189;248m{}\x1b[0m",
            i + 1,
            c.chunk_x,
            c.chunk_z,
            c.block_x(),
            c.block_z(),
            c.score,
            c.severity_colored(),
            c.entity_count,
            c.villagers,
            c.hoppers,
            c.tp_command()
        );
    }
    println!();
}

fn inspect(c: &ChunkMetrics) {
    println!(
        "\n  \x1b[38;2;99;102;241m╭─\x1b[0m \x1b[1;38;2;248;250;252mChunk Diagnostic [ {}, {} ]\x1b[0m \x1b[38;2;71;85;105m──────────────────────────────\x1b[0m [ {} ] \x1b[38;2;71;85;105m─╮\x1b[0m",
        c.chunk_x,
        c.chunk_z,
        c.severity_colored()
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mIn-Game Block:\x1b[0m   \x1b[1;38;2;245;158;11mX: {}, Z: {}\x1b[0m  \x1b[38;2;100;116;139m(Center of chunk)\x1b[0m",
        c.block_x(),
        c.block_z()
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mTeleport Command:\x1b[0m \x1b[38;2;56;189;248m{}\x1b[0m",
        c.tp_command()
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mDimension:\x1b[0m        \x1b[38;2;226;232;240m{}\x1b[0m",
        c.dimension
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[38;2;148;163;184mPotential Score:\x1b[0m  \x1b[1;38;2;244;63;94m{:.1}\x1b[0m \x1b[38;2;100;116;139m/ 100\x1b[0m",
        c.score
    );
    println!(
        "  \x1b[38;2;71;85;105m├──────────────────────────────────────────────────────────────────────────┤\x1b[0m"
    );
    println!("  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[1;38;2;248;250;252mKey Mechanics:\x1b[0m");
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m    \x1b[38;2;148;163;184mHoppers:\x1b[0m {:<6} \x1b[38;2;148;163;184mHopper Minecarts:\x1b[0m {:<6} \x1b[38;2;148;163;184mVillagers:\x1b[0m {:<6}",
        c.hoppers, c.hopper_minecarts, c.villagers
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m    \x1b[38;2;148;163;184mEntities:\x1b[0m {:<5} \x1b[38;2;148;163;184mBlock Entities:\x1b[0m   {:<6} \x1b[38;2;148;163;184mFurnaces:\x1b[0m  {:<6}",
        c.entity_count, c.block_entity_count, c.furnaces
    );
    println!(
        "  \x1b[38;2;71;85;105m│\x1b[0m    \x1b[38;2;148;163;184mRedstone:\x1b[0m Wire: {}  •  Observers: {}  •  Pistons: {}  •  Comparators: {}",
        c.redstone_wire, c.observers, c.pistons, c.comparators
    );
    if !c.entity_types.is_empty() {
        println!(
            "  \x1b[38;2;71;85;105m├──────────────────────────────────────────────────────────────────────────┤\x1b[0m"
        );
        println!(
            "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[1;38;2;248;250;252mEntity Breakdown:\x1b[0m"
        );
        for (id, count) in &c.entity_types {
            println!(
                "  \x1b[38;2;71;85;105m│\x1b[0m    \x1b[38;2;148;163;184m•\x1b[0m \x1b[38;2;226;232;240m{:<28}\x1b[0m \x1b[1;38;2;245;158;11m{}\x1b[0m",
                id, count
            );
        }
    }
    if !c.block_entity_types.is_empty() {
        println!(
            "  \x1b[38;2;71;85;105m├──────────────────────────────────────────────────────────────────────────┤\x1b[0m"
        );
        println!(
            "  \x1b[38;2;71;85;105m│\x1b[0m  \x1b[1;38;2;248;250;252mBlock Entity Breakdown:\x1b[0m"
        );
        for (id, count) in &c.block_entity_types {
            println!(
                "  \x1b[38;2;71;85;105m│\x1b[0m    \x1b[38;2;148;163;184m•\x1b[0m \x1b[38;2;226;232;240m{:<28}\x1b[0m \x1b[1;38;2;245;158;11m{}\x1b[0m",
                id, count
            );
        }
    }
    println!(
        "  \x1b[38;2;71;85;105m╰──────────────────────────────────────────────────────────────────────────╯\x1b[0m\n"
    );
}

fn find(result: Option<&ScanResult>, id: &str, entities: bool) {
    let Some(result) = result else {
        println!("Run scan first.");
        return;
    };
    let mut found: Vec<_> = result
        .chunks
        .iter()
        .filter_map(|chunk| {
            let count = if entities {
                chunk.entity_types.get(id)
            } else {
                chunk.block_entity_types.get(id)
            }?;
            Some((chunk, count))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(a.1));
    if found.is_empty() {
        println!("No matches for {id}.");
    }
    for (chunk, count) in found.into_iter().take(20) {
        println!("{}, {}: {}", chunk.chunk_x, chunk.chunk_z, count);
    }
}

fn find_blocks(result: Option<&ScanResult>, id: &str) {
    let Some(result) = result else {
        println!("Run scan first.");
        return;
    };
    let mut found: Vec<_> = result
        .chunks
        .iter()
        .filter_map(|chunk| chunk.block_types.get(id).map(|count| (chunk, count)))
        .collect();
    found.sort_by(|a, b| b.1.cmp(a.1));
    if found.is_empty() {
        println!("No matches for {id}.");
    }
    for (chunk, count) in found.into_iter().take(20) {
        println!("{}, {}: {}", chunk.chunk_x, chunk.chunk_z, count);
    }
}
