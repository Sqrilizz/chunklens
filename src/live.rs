use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Result};
use notify::{Event, EventKind, RecursiveMode, Watcher};

use crate::{
    analysis::{self, ScoreWeights},
    model::ChunkMetrics,
    nbt,
    protocol::rcon::RconClient,
    region,
};

#[derive(Debug, Clone)]
pub struct LiveOptions {
    pub world_dir: PathBuf,
    pub rcon_address: Option<SocketAddr>,
    pub rcon_password: Option<String>,
    pub spike_score: f64,
    pub spike_entities: u64,
    pub spike_hoppers: u64,
    pub spike_payload_kb: u64,
    pub mspt_threshold: f64,
    pub auto_flush: bool,
    pub poll_interval_secs: u64,
    pub webhook: Option<String>,
}

impl Default for LiveOptions {
    fn default() -> Self {
        Self {
            world_dir: PathBuf::from("world"),
            rcon_address: None,
            rcon_password: None,
            spike_score: 15.0,
            spike_entities: 40,
            spike_hoppers: 20,
            spike_payload_kb: 150,
            mspt_threshold: 45.0,
            auto_flush: true,
            poll_interval_secs: 2,
            webhook: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfo {
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl PlayerInfo {
    pub fn distance_to_chunk(&self, cx: i32, cz: i32) -> f64 {
        let center_x = cx as f64 * 16.0 + 8.0;
        let center_z = cz as f64 * 16.0 + 8.0;
        let dx = self.x - center_x;
        let dz = self.z - center_z;
        (dx * dx + dz * dz).sqrt()
    }
}

pub fn parse_player_positions(rcon_output: &str) -> Vec<PlayerInfo> {
    let mut players = Vec::new();
    let pattern = " has the following entity data: [";
    let mut rest = rcon_output;

    while let Some(idx) = rest.find(pattern) {
        let before = &rest[..idx];
        let token = before.split(']').next_back().unwrap_or("").trim();
        let name = token.split_whitespace().last().unwrap_or("").trim();

        rest = &rest[idx + pattern.len()..];
        if let Some(end_idx) = rest.find(']') {
            let coords_str = &rest[..end_idx];
            rest = &rest[end_idx + 1..];
            let coords: Vec<&str> = coords_str.split(',').collect();
            if coords.len() == 3 {
                let parse_coord = |s: &str| -> Option<f64> {
                    let clean = s.trim().trim_end_matches(['d', 'f', 'D', 'F']);
                    clean.parse::<f64>().ok()
                };
                if let (Some(x), Some(y), Some(z)) = (
                    parse_coord(coords[0]),
                    parse_coord(coords[1]),
                    parse_coord(coords[2]),
                ) && !name.is_empty()
                {
                    let chunk_x = (x / 16.0).floor() as i32;
                    let chunk_z = (z / 16.0).floor() as i32;
                    players.push(PlayerInfo {
                        name: name.to_string(),
                        x,
                        y,
                        z,
                        chunk_x,
                        chunk_z,
                    });
                }
            }
        }
    }
    players
}

pub fn parse_server_perf(output: &str) -> (Option<f64>, Option<f64>) {
    let mut mspt: Option<f64> = None;
    let mut tps: Option<f64> = None;
    let mut expect_tps_next_line = false;

    for line in output.lines() {
        let clean = strip_ansi_and_color_codes(line);
        let trimmed = clean.trim();

        if expect_tps_next_line
            && tps.is_none()
            && let Some(first_num) = trimmed.split(',').next()
            && let Ok(val) = first_num.trim().parse::<f64>()
        {
            tps = Some(val.clamp(0.0, 20.0));
            expect_tps_next_line = false;
        }

        if (clean.contains("TPS from last") || clean.starts_with("TPS:"))
            && tps.is_none()
            && let Some(idx) = clean.find(':')
        {
            let after = clean[idx + 1..].trim();
            if after.is_empty() {
                expect_tps_next_line = true;
            } else if let Some(first_num) = after.split(',').next()
                && let Ok(val) = first_num.trim().parse::<f64>()
            {
                tps = Some(val.clamp(0.0, 20.0));
            }
        }

        if clean.contains('/') {
            for part in clean.split_whitespace() {
                let clean_part = part.trim_matches(',');
                let slash_count = clean_part.chars().filter(|&c| c == '/').count();
                if slash_count >= 3 {
                    let numbers: Vec<&str> = clean_part.split('/').collect();
                    if numbers.len() >= 4
                        && let Ok(median) = numbers[1].trim().parse::<f64>()
                        && mspt.is_none()
                    {
                        mspt = Some(median);
                    }
                }
            }
        } else if clean.contains("MSPT:")
            && let Some(idx) = clean.find("MSPT:")
        {
            let after = clean[idx + 5..].trim();
            for part in after.split_whitespace() {
                if let Ok(val) = part.trim_end_matches("ms").trim().parse::<f64>()
                    && val > 0.0
                    && val < 5000.0
                {
                    mspt = Some(val);
                    break;
                }
            }
        }
    }

    (mspt, tps)
}

pub fn generate_purge_command(
    dimension: &str,
    chunk_x: i32,
    chunk_z: i32,
    entity_type: &str,
) -> String {
    let bx = chunk_x * 16;
    let bz = chunk_z * 16;
    let selector = match entity_type {
        "all" => format!("@e[type=!player,x={bx},y=-64,z={bz},dx=15,dy=384,dz=15]"),
        other => format!("@e[type={other},x={bx},y=-64,z={bz},dx=15,dy=384,dz=15]"),
    };
    let dim_prefix = if dimension.is_empty() || dimension == "world" || dimension == "live-server" {
        "execute in minecraft:overworld run kill "
    } else if dimension.contains("nether") {
        "execute in minecraft:the_nether run kill "
    } else if dimension.contains("end") {
        "execute in minecraft:the_end run kill "
    } else {
        "execute in minecraft:overworld run kill "
    };
    format!("{dim_prefix}{selector}")
}

pub fn parse_kill_count(rcon_response: &str) -> Option<u64> {
    if rcon_response.contains("No entity was found") {
        return Some(0);
    }
    let lower = rcon_response.to_ascii_lowercase();
    if let Some(idx) = lower.find("killed ") {
        let after = &lower[idx + 7..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        return num_str.parse().ok();
    }
    None
}

pub fn parse_execute_count(output: &str) -> Option<u64> {
    if output.contains("Test failed") {
        return Some(0);
    }
    let lower = output.to_ascii_lowercase();
    if let Some(idx) = lower.find("count:") {
        let after = output[idx + 6..].trim();
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        return num_str.parse().ok();
    }
    None
}

fn strip_ansi_and_color_codes(input: &str) -> String {
    let mut clean = String::with_capacity(input.len());
    let mut in_escape = false;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
            continue;
        }
        if c == '§' {
            // skip minecraft color code symbol and its following char
            chars.next();
            continue;
        }
        clean.push(c);
    }
    clean
}

#[derive(Debug, Clone)]
pub struct SpikeAlert {
    pub timestamp: String,
    pub dimension: String,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub block_x: i64,
    pub block_z: i64,
    pub old_score: f64,
    pub new_score: f64,
    pub delta_score: f64,
    pub delta_entities: i64,
    pub delta_villagers: i64,
    pub delta_hoppers: i64,
    pub delta_minecarts: i64,
    pub delta_payload_kb: i64,
    pub suspects: Vec<(String, f64)>,
    pub server_mspt: Option<f64>,
    pub server_tps: Option<f64>,
    pub reasons: Vec<String>,
}

pub fn evaluate_spike(
    old_metric: Option<&ChunkMetrics>,
    new_metric: &ChunkMetrics,
    old_score: f64,
    new_score: f64,
    options: &LiveOptions,
) -> Option<Vec<String>> {
    let mut reasons = Vec::new();
    let delta_score = new_score - old_score;

    let (old_ent, old_vil, old_hop, old_cart, old_size) = match old_metric {
        Some(m) => (
            m.entity_count as i64,
            m.villagers as i64,
            (m.hoppers + m.hopper_minecarts) as i64,
            m.minecarts as i64,
            (m.payload_size / 1024) as i64,
        ),
        None => (0, 0, 0, 0, 0),
    };

    let new_ent = new_metric.entity_count as i64;
    let new_vil = new_metric.villagers as i64;
    let new_hop = (new_metric.hoppers + new_metric.hopper_minecarts) as i64;
    let new_cart = new_metric.minecarts as i64;
    let new_size = (new_metric.payload_size / 1024) as i64;

    let delta_ent = new_ent - old_ent;
    let delta_vil = new_vil - old_vil;
    let delta_hop = new_hop - old_hop;
    let delta_cart = new_cart - old_cart;
    let delta_size = new_size - old_size;

    if delta_score >= options.spike_score {
        reasons.push(format!("load score jumped by +{:.1}", delta_score));
    }

    if delta_ent >= options.spike_entities as i64 {
        reasons.push(format!("entities surged by +{}", delta_ent));
    }

    if delta_vil >= 10 {
        reasons.push(format!("villagers surged by +{}", delta_vil));
    }

    if delta_hop >= options.spike_hoppers as i64 {
        reasons.push(format!("hoppers increased by +{}", delta_hop));
    }

    if delta_cart >= 20 {
        reasons.push(format!("minecarts increased by +{}", delta_cart));
    }

    if delta_size >= options.spike_payload_kb as i64 {
        reasons.push(format!("NBT payload grew by +{} KiB", delta_size));
    }

    if new_metric.is_bloat_hazard()
        && (old_metric.is_none() || !old_metric.unwrap().is_bloat_hazard())
    {
        reasons.push(format!(
            "crossed hazard threshold: {}",
            new_metric.bloat_hazard_badge().0
        ));
    }

    if new_score >= 60.0 && old_score < 30.0 {
        reasons.push("critical high-load state reached".to_string());
    }

    if reasons.is_empty() {
        None
    } else {
        Some(reasons)
    }
}

struct AlertContext<'a> {
    dimension: &'a str,
    chunk_x: i32,
    chunk_z: i32,
    old_metric: Option<&'a ChunkMetrics>,
    new_metric: &'a ChunkMetrics,
    old_score: f64,
    new_score: f64,
    reasons: Vec<String>,
}

pub struct LiveWatcher {
    options: LiveOptions,
    known_chunks: HashMap<(i32, i32), ChunkMetrics>,
    chunk_timestamps: HashMap<(PathBuf, usize), u32>,
    rcon: Option<RconClient>,
    cached_players: Vec<PlayerInfo>,
    last_perf: (Option<f64>, Option<f64>),
    decompress_buf: Vec<u8>,
    decompressor: libdeflater::Decompressor,
    score_weights: ScoreWeights,
}

impl LiveWatcher {
    pub fn new(mut options: LiveOptions) -> Self {
        let rcon = if let Some(addr) = options.rcon_address {
            let pwd = options.rcon_password.take().unwrap_or_default();
            match RconClient::connect(addr, &pwd, Duration::from_secs(4)) {
                Ok(client) => {
                    println!(
                        "  \x1b[1;32m✔\x1b[0m Connected to RCON at \x1b[1m{}\x1b[0m (Player Radar & Live MSPT enabled)",
                        addr
                    );
                    Some(client)
                }
                Err(e) => {
                    eprintln!(
                        "  \x1b[1;33m⚠\x1b[0m Could not connect to RCON at {}: {:#}",
                        addr, e
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            options,
            known_chunks: HashMap::with_capacity(4096),
            chunk_timestamps: HashMap::with_capacity(4096),
            rcon,
            cached_players: Vec::new(),
            last_perf: (None, None),
            decompress_buf: Vec::with_capacity(512 * 1024),
            decompressor: libdeflater::Decompressor::new(),
            score_weights: ScoreWeights::default(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let region_dir = self.options.world_dir.join("region");
        if !region_dir.is_dir() {
            if self.rcon.is_some() {
                return self.run_remote_rcon_sentinel();
            }
            anyhow::bail!(
                "region directory not found at: {} (provide world folder path or connect via --rcon)",
                region_dir.display()
            );
        }

        println!(
            "\n  \x1b[1;38;2;99;102;241m╭─────────────────────────────────────────────────────────────────────────────╮\x1b[0m"
        );
        println!(
            "  \x1b[1;38;2;99;102;241m│\x1b[0m  \x1b[1;38;2;248;250;252mChunkLens Live Lag & Spike Sentinel\x1b[0m                                     \x1b[1;38;2;99;102;241m│\x1b[0m"
        );
        println!(
            "  \x1b[1;38;2;99;102;241m│\x1b[0m  \x1b[38;2;148;163;184mWatching world:\x1b[0m \x1b[38;2;226;232;240m{:<57}\x1b[0m \x1b[1;38;2;99;102;241m│\x1b[0m",
            self.options.world_dir.display()
        );
        println!(
            "  \x1b[1;38;2;99;102;241m│\x1b[0m  \x1b[38;2;148;163;184mThresholds:    \x1b[0m \x1b[38;2;245;158;11mΔScore ≥ +{:<4.1}\x1b[0m • \x1b[38;2;245;158;11mΔEntities ≥ +{:<3}\x1b[0m • \x1b[38;2;245;158;11mΔHoppers ≥ +{:<3}\x1b[0m \x1b[1;38;2;99;102;241m│\x1b[0m",
            self.options.spike_score, self.options.spike_entities, self.options.spike_hoppers
        );
        println!(
            "  \x1b[1;38;2;99;102;241m╰─────────────────────────────────────────────────────────────────────────────╯\x1b[0m\n"
        );

        print!("  \x1b[38;2;129;140;248m⠋\x1b[0m Indexing initial region headers... ");
        let initial_count = self.index_headers(&region_dir)?;
        println!(
            "\x1b[1;32m✔\x1b[0m Indexed \x1b[1m{}\x1b[0m chunks across active regions.",
            initial_count
        );

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })
        .context("creating file watcher")?;

        watcher
            .watch(&region_dir, RecursiveMode::NonRecursive)
            .context("watching region directory")?;

        let entities_dir = self.options.world_dir.join("entities");
        if entities_dir.is_dir() {
            let _ = watcher.watch(&entities_dir, RecursiveMode::NonRecursive);
        }

        println!(
            "  \x1b[1;32m●\x1b[0m Sentinel active. Listening for autosaves and live chunk spikes... (Ctrl+C to stop)\n"
        );

        let mut last_poll = Instant::now();
        let mut modified_regions = HashSet::new();

        loop {
            // Drain notify events
            while let Ok(event) = rx.try_recv() {
                if matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
                ) {
                    for path in event.paths {
                        if path
                            .extension()
                            .is_some_and(|ext| ext == "mca" || ext == "linear")
                        {
                            modified_regions.insert(path);
                        }
                    }
                }
            }

            // Periodic RCON radar & MSPT check
            if last_poll.elapsed() >= Duration::from_secs(self.options.poll_interval_secs) {
                last_poll = Instant::now();
                self.poll_rcon_state();
            }

            // Process modified regions if any
            if !modified_regions.is_empty() {
                let paths: Vec<PathBuf> = modified_regions.drain().collect();
                for path in paths {
                    if let Err(e) = self.process_modified_region(&path) {
                        eprintln!(
                            "  \x1b[38;2;244;63;94m✖ Error inspecting {}:\x1b[0m {:#}",
                            path.display(),
                            e
                        );
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn index_headers(&mut self, region_dir: &Path) -> Result<usize> {
        let mut total_indexed = 0;
        let entries = std::fs::read_dir(region_dir)?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext == "mca" || ext == "linear")
                && let Ok(headers) = region::read_region_header(&path)
            {
                for slot in headers {
                    self.chunk_timestamps
                        .insert((path.clone(), slot.slot), slot.timestamp);
                    total_indexed += 1;
                }
            }
        }
        Ok(total_indexed)
    }

    fn poll_rcon_state(&mut self) {
        let Some(rcon) = self.rcon.as_mut() else {
            return;
        };

        // Query player positions
        if let Ok(resp) = rcon.execute("execute as @a run data get entity @s Pos") {
            self.cached_players = parse_player_positions(&resp);
        }

        // Query performance
        let perf_resp = rcon
            .execute("spark tps")
            .or_else(|_| rcon.execute("tps"))
            .unwrap_or_default();
        let (mspt, tps) = parse_server_perf(&perf_resp);
        self.last_perf = (mspt, tps);

        if let Some(m) = mspt
            && m >= self.options.mspt_threshold
            && self.options.auto_flush
        {
            println!(
                "  \x1b[1;38;2;245;158;11m⚡ Server MSPT Spike: {:.1} ms!\x1b[0m Triggering \x1b[1m/save-all flush\x1b[0m to capture live RAM chunks...",
                m
            );
            let _ = rcon.execute("save-all flush");
        }
    }

    fn process_modified_region(&mut self, path: &Path) -> Result<()> {
        let (rx, rz) = match region::region_coordinates(path) {
            Some(coords) => coords,
            None => return Ok(()),
        };

        let headers = region::read_region_header(path)?;
        let mut changed_slots = Vec::new();

        for slot in headers {
            let key = (path.to_path_buf(), slot.slot);
            let prev_ts = self.chunk_timestamps.get(&key).copied().unwrap_or(0);
            if slot.timestamp > prev_ts || prev_ts == 0 {
                self.chunk_timestamps.insert(key, slot.timestamp);
                changed_slots.push(slot);
            }
        }

        let dimension = self
            .options
            .world_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("world")
            .to_string();

        for slot in changed_slots {
            let local_x = (slot.slot % 32) as i32;
            let local_z = (slot.slot / 32) as i32;
            let chunk_x = rx * 32 + local_x;
            let chunk_z = rz * 32 + local_z;

            self.decompress_buf.clear();
            if region::read_single_chunk(
                path,
                slot.slot,
                slot.sector_offset,
                slot.sector_count,
                &mut self.decompress_buf,
                &mut self.decompressor,
            )
            .is_err()
            {
                continue;
            }

            let new_metric =
                match nbt::extract(&self.decompress_buf, &dimension, chunk_x, chunk_z, false) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

            let new_score = analysis::score(&new_metric, &self.score_weights);
            let old_metric = self.known_chunks.get(&(chunk_x, chunk_z));
            let old_score = old_metric
                .map(|m| analysis::score(m, &self.score_weights))
                .unwrap_or(0.0);

            if let Some(reasons) =
                evaluate_spike(old_metric, &new_metric, old_score, new_score, &self.options)
            {
                let alert = self.build_alert(AlertContext {
                    dimension: &dimension,
                    chunk_x,
                    chunk_z,
                    old_metric,
                    new_metric: &new_metric,
                    old_score,
                    new_score,
                    reasons,
                });
                Self::print_alert(&alert);
                if let Some(url) = &self.options.webhook {
                    send_webhook(url, &alert);
                }
            }

            self.known_chunks.insert((chunk_x, chunk_z), new_metric);
        }

        Ok(())
    }

    fn build_alert(&self, ctx: AlertContext<'_>) -> SpikeAlert {
        let block_x = ctx.new_metric.block_x();
        let block_z = ctx.new_metric.block_z();

        let (old_ent, old_vil, old_hop, old_cart, old_size) = match ctx.old_metric {
            Some(m) => (
                m.entity_count as i64,
                m.villagers as i64,
                (m.hoppers + m.hopper_minecarts) as i64,
                m.minecarts as i64,
                (m.payload_size / 1024) as i64,
            ),
            None => (0, 0, 0, 0, 0),
        };

        let delta_score = ctx.new_score - ctx.old_score;
        let delta_entities = ctx.new_metric.entity_count as i64 - old_ent;
        let delta_villagers = ctx.new_metric.villagers as i64 - old_vil;
        let delta_hoppers =
            (ctx.new_metric.hoppers + ctx.new_metric.hopper_minecarts) as i64 - old_hop;
        let delta_minecarts = ctx.new_metric.minecarts as i64 - old_cart;
        let delta_payload_kb = (ctx.new_metric.payload_size / 1024) as i64 - old_size;

        // Correlate suspects from player radar
        let mut suspects: Vec<(String, f64)> = self
            .cached_players
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.distance_to_chunk(ctx.chunk_x, ctx.chunk_z),
                )
            })
            .filter(|(_, dist)| *dist <= 128.0)
            .collect();
        suspects.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let now = chrono_lite_time_string();

        SpikeAlert {
            timestamp: now,
            dimension: ctx.dimension.to_string(),
            chunk_x: ctx.chunk_x,
            chunk_z: ctx.chunk_z,
            block_x,
            block_z,
            old_score: ctx.old_score,
            new_score: ctx.new_score,
            delta_score,
            delta_entities,
            delta_villagers,
            delta_hoppers,
            delta_minecarts,
            delta_payload_kb,
            suspects,
            server_mspt: self.last_perf.0,
            server_tps: self.last_perf.1,
            reasons: ctx.reasons,
        }
    }

    fn run_remote_rcon_sentinel(&mut self) -> Result<()> {
        let addr_str = self
            .options
            .rcon_address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "remote".to_string());

        println!("
  [1;38;2;99;102;241m╭─────────────────────────────────────────────────────────────────────────────╮[0m");
        println!(
            "  [1;38;2;99;102;241m│[0m  [1;38;2;248;250;252mChunkLens Remote RCON Sentinel & Live Radar[0m                         [1;38;2;99;102;241m│[0m"
        );
        println!(
            "  [1;38;2;99;102;241m│[0m  [38;2;148;163;184mServer Target: [0m [1;38;2;52;211;153m{:<57}[0m [1;38;2;99;102;241m│[0m",
            addr_str
        );
        println!(
            "  [1;38;2;99;102;241m│[0m  [38;2;148;163;184mMonitoring:    [0m [38;2;245;158;11mMSPT Spikes[0m • [38;2;245;158;11mActive Player Chunks[0m • [38;2;245;158;11mEntity Floods (Δ ≥ +{:<3})[0m [1;38;2;99;102;241m│[0m",
            self.options.spike_entities
        );
        println!("  [1;38;2;99;102;241m╰─────────────────────────────────────────────────────────────────────────────╯[0m
");
        println!("  [1;32m●[0m Remote Sentinel active. Scanning active player zones... (Ctrl+C to quit)
");

        let mut previous_counts: HashMap<(i32, i32), u64> = HashMap::new();

        loop {
            self.poll_rcon_state();

            if let Some(rcon) = self.rcon.as_mut() {
                let player_chunks: HashSet<(i32, i32)> = self
                    .cached_players
                    .iter()
                    .map(|p| (p.chunk_x, p.chunk_z))
                    .collect();

                for &(cx, cz) in &player_chunks {
                    let bx = cx * 16;
                    let bz = cz * 16;
                    let cmd =
                        format!("execute if entity @e[x={bx},y=-64,z={bz},dx=15,dy=384,dz=15]");
                    if let Ok(res) = rcon.execute(&cmd)
                        && let Some(count) = parse_execute_count(&res)
                    {
                        let prev_opt = previous_counts.get(&(cx, cz)).copied();
                        let is_spike = match prev_opt {
                            Some(prev) => {
                                count >= prev + self.options.spike_entities
                                    || (count >= 50 && prev < 30)
                            }
                            None => count >= 50 || count >= self.options.spike_entities,
                        };
                        let prev = prev_opt.unwrap_or(0);

                        if is_spike {
                            let item_count = rcon
                                .execute(&format!("execute if entity @e[type=item,x={bx},y=-64,z={bz},dx=15,dy=384,dz=15]"))
                                .ok()
                                .and_then(|r| parse_execute_count(&r))
                                .unwrap_or(0);
                            let vil_count = rcon
                                .execute(&format!("execute if entity @e[type=villager,x={bx},y=-64,z={bz},dx=15,dy=384,dz=15]"))
                                .ok()
                                .and_then(|r| parse_execute_count(&r))
                                .unwrap_or(0);

                            let mut reasons = Vec::new();
                            reasons.push(format!(
                                "entity count surged by +{} ({} ➔ {})",
                                count - prev,
                                prev,
                                count
                            ));
                            if item_count > 10 {
                                reasons.push(format!("{} dropped items detected", item_count));
                            }
                            if vil_count > 5 {
                                reasons.push(format!("{} villagers clustered", vil_count));
                            }

                            let suspects: Vec<(String, f64)> = self
                                .cached_players
                                .iter()
                                .map(|p| (p.name.clone(), p.distance_to_chunk(cx, cz)))
                                .filter(|(_, dist)| *dist <= 64.0)
                                .collect();

                            let alert = SpikeAlert {
                                timestamp: chrono_lite_time_string(),
                                dimension: "live-server".to_string(),
                                chunk_x: cx,
                                chunk_z: cz,
                                block_x: (cx * 16 + 8) as i64,
                                block_z: (cz * 16 + 8) as i64,
                                old_score: prev as f64 * 0.05,
                                new_score: count as f64 * 0.05,
                                delta_score: (count.saturating_sub(prev)) as f64 * 0.05,
                                delta_entities: (count as i64) - (prev as i64),
                                delta_villagers: vil_count as i64,
                                delta_hoppers: 0,
                                delta_minecarts: 0,
                                delta_payload_kb: 0,
                                suspects,
                                server_mspt: self.last_perf.0,
                                server_tps: self.last_perf.1,
                                reasons,
                            };
                            Self::print_alert(&alert);
                            if let Some(url) = &self.options.webhook {
                                send_webhook(url, &alert);
                            }
                        }

                        previous_counts.insert((cx, cz), count);
                    }
                }
            }

            std::thread::sleep(Duration::from_secs(self.options.poll_interval_secs));
        }
    }

    fn print_alert(alert: &SpikeAlert) {
        // Audible terminal bell for notification
        print!("\x07");

        println!(
            "\n  \x1b[38;2;244;63;94m╭───\x1b[0m \x1b[1;38;2;244;63;94m🚨 LIVE LAG SPIKE DETECTED\x1b[0m \x1b[38;2;100;116;139m[{}]\x1b[0m \x1b[38;2;244;63;94m───────────────────────────────────────────────╮\x1b[0m",
            alert.timestamp
        );
        println!(
            "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[1;38;2;248;250;252mChunk [{}, {}]\x1b[0m \x1b[38;2;148;163;184m({}) • Center: ~{}, ~\x1b[0m \x1b[1;38;2;56;189;248m{}\x1b[0m",
            alert.chunk_x, alert.chunk_z, alert.dimension, alert.block_x, alert.block_z
        );
        let purge_items =
            generate_purge_command(&alert.dimension, alert.chunk_x, alert.chunk_z, "item");
        let purge_carts =
            generate_purge_command(&alert.dimension, alert.chunk_x, alert.chunk_z, "minecart");
        let purge_all =
            generate_purge_command(&alert.dimension, alert.chunk_x, alert.chunk_z, "all");
        println!(
            "  [38;2;244;63;94m│[0m  [38;2;148;163;184mPurge Drop:   [0m [38;2;245;158;11m/{}[0m",
            purge_items
        );
        if alert.delta_minecarts > 0 || alert.reasons.iter().any(|r| r.contains("minecart")) {
            println!(
                "  [38;2;244;63;94m│[0m  [38;2;148;163;184mPurge Carts:  [0m [38;2;245;158;11m/{}[0m",
                purge_carts
            );
        }
        println!(
            "  [38;2;244;63;94m│[0m  [38;2;148;163;184mPurge All:    [0m [38;2;244;63;94m/{}[0m",
            purge_all
        );
        println!(
            "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[38;2;148;163;184mScore Jump:   \x1b[0m \x1b[1;38;2;245;158;11m+{:.1}\x1b[0m  ({:.1} ➔ \x1b[1;38;2;244;63;94m{:.1}\x1b[0m)",
            alert.delta_score, alert.old_score, alert.new_score
        );

        let mut changes = Vec::new();
        if alert.delta_hoppers != 0 {
            changes.push(format!("hoppers: {:+}", alert.delta_hoppers));
        }
        if alert.delta_villagers != 0 {
            changes.push(format!("villagers: {:+}", alert.delta_villagers));
        }
        if alert.delta_entities != 0 {
            changes.push(format!("entities: {:+}", alert.delta_entities));
        }
        if alert.delta_minecarts != 0 {
            changes.push(format!("minecarts: {:+}", alert.delta_minecarts));
        }
        if alert.delta_payload_kb != 0 {
            changes.push(format!("payload: {:+} KiB", alert.delta_payload_kb));
        }
        if !changes.is_empty() {
            println!(
                "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[38;2;148;163;184mDeltas:       \x1b[0m \x1b[38;2;226;232;240m{}\x1b[0m",
                changes.join(", ")
            );
        }

        println!(
            "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[38;2;148;163;184mTriggers:     \x1b[0m \x1b[38;2;251;146;60m{}\x1b[0m",
            alert.reasons.join(" • ")
        );

        if !alert.suspects.is_empty() {
            let suspect_strs: Vec<String> = alert
                .suspects
                .iter()
                .take(3)
                .map(|(name, dist)| {
                    if *dist <= 16.0 {
                        format!(
                            "\x1b[1;38;2;244;63;94m{} (IN CHUNK, {:.0}m)\x1b[0m",
                            name, dist
                        )
                    } else {
                        format!("\x1b[38;2;245;158;11m{} ({:.0}m away)\x1b[0m", name, dist)
                    }
                })
                .collect();
            println!(
                "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[1;38;2;248;250;252mSuspect(s):   \x1b[0m {}",
                suspect_strs.join(", ")
            );
        }

        if let (Some(mspt), Some(tps)) = (alert.server_mspt, alert.server_tps) {
            println!(
                "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[38;2;148;163;184mServer State: \x1b[0m MSPT: \x1b[1m{:.1} ms\x1b[0m • TPS: \x1b[1m{:.1}\x1b[0m",
                mspt, tps
            );
        }

        println!(
            "  \x1b[38;2;244;63;94m│\x1b[0m  \x1b[38;2;148;163;184mTeleport:     \x1b[0m \x1b[1;38;2;56;189;248m/tp @s {} ~ {}\x1b[0m",
            alert.block_x, alert.block_z
        );
        println!(
            "  \x1b[38;2;244;63;94m╰─────────────────────────────────────────────────────────────────────────────╯\x1b[0m\n"
        );
    }
}

pub fn format_discord_webhook(alert: &SpikeAlert) -> String {
    let tp_cmd = format!("/tp @s {} ~ {}", alert.block_x, alert.block_z);
    let kill_cmd = generate_purge_command(&alert.dimension, alert.chunk_x, alert.chunk_z, "item");
    let perf_str = match (alert.server_mspt, alert.server_tps) {
        (Some(mspt), Some(tps)) => format!("{mspt:.1} ms ({tps:.1} TPS)"),
        _ => "N/A".to_string(),
    };
    let suspect_str = if alert.suspects.is_empty() {
        "None within 250 blocks".to_string()
    } else {
        alert
            .suspects
            .iter()
            .take(3)
            .map(|(name, dist)| format!("{name} (~{dist:.0}b)"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let reasons_str = alert.reasons.join("\n• ");

    let json = serde_json::json!({
        "username": "ChunkLens Sentinel",
        "embeds": [
            {
                "title": "🚨 Minecraft Lag Spike Detected",
                "color": 15158332,
                "fields": [
                    {
                        "name": "Location",
                        "value": format!("Chunk `[{}, {}]` (Dimension: `{}`)\nBlock `~[{}, {}]`", alert.chunk_x, alert.chunk_z, alert.dimension, alert.block_x, alert.block_z),
                        "inline": true
                    },
                    {
                        "name": "Server Performance",
                        "value": format!("MSPT: `{perf_str}`"),
                        "inline": true
                    },
                    {
                        "name": "Detected Spikes",
                        "value": format!("• {reasons_str}"),
                        "inline": false
                    },
                    {
                        "name": "Nearby Players",
                        "value": suspect_str,
                        "inline": false
                    },
                    {
                        "name": "Quick Fix Commands",
                        "value": format!("Teleport: `{tp_cmd}`\nPurge items: `{kill_cmd}`"),
                        "inline": false
                    }
                ],
                "footer": {
                    "text": format!("ChunkLens Sentinel • {}", alert.timestamp)
                }
            }
        ]
    });
    json.to_string()
}

pub fn send_webhook(url: &str, alert: &SpikeAlert) {
    let payload = if url.contains("telegram") {
        let text = format!(
            "🚨 *Minecraft Lag Spike Detected*\n\nChunk: `[{}, {}]` ({})\nMSPT: `{:?}`\nDelta: `+{:.1}`\n\nFix: `/tp @s {} ~ {}`",
            alert.chunk_x,
            alert.chunk_z,
            alert.dimension,
            alert.server_mspt,
            alert.delta_score,
            alert.block_x,
            alert.block_z
        );
        serde_json::json!({
            "text": text,
            "parse_mode": "Markdown"
        })
        .to_string()
    } else {
        format_discord_webhook(alert)
    };

    let url = url.to_string();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("curl")
            .arg("-s")
            .arg("-m")
            .arg("5")
            .arg("-X")
            .arg("POST")
            .arg("-H")
            .arg("Content-Type: application/json")
            .arg("-d")
            .arg(&payload)
            .arg(&url)
            .output();
    });
}

fn chrono_lite_time_string() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    format!("{:02}:{:02}:{:02} UTC", hours, mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formats_discord_webhook_embed_correctly() {
        let alert = SpikeAlert {
            timestamp: "12:00:00 UTC".to_string(),
            dimension: "overworld".to_string(),
            chunk_x: 10,
            chunk_z: -20,
            block_x: 168,
            block_z: -312,
            old_score: 10.0,
            new_score: 35.0,
            delta_score: 25.0,
            delta_entities: 50,
            delta_villagers: 10,
            delta_hoppers: 25,
            delta_minecarts: 0,
            delta_payload_kb: 200,
            suspects: vec![("Notch".to_string(), 15.0)],
            server_mspt: Some(48.5),
            server_tps: Some(18.2),
            reasons: vec!["load score jumped by +25.0".to_string()],
        };
        let payload = format_discord_webhook(&alert);
        assert!(payload.contains("ChunkLens Sentinel"));
        assert!(payload.contains("Chunk `[10, -20]`"));
        assert!(payload.contains("48.5 ms"));
        assert!(payload.contains("Notch (~15b)"));
    }

    #[test]
    fn generates_accurate_purge_commands() {
        let cmd_item = generate_purge_command("world", -164, 74, "item");
        assert_eq!(
            cmd_item,
            "execute in minecraft:overworld run kill @e[type=item,x=-2624,y=-64,z=1184,dx=15,dy=384,dz=15]"
        );

        let cmd_all = generate_purge_command("the_nether", 10, -5, "all");
        assert_eq!(
            cmd_all,
            "execute in minecraft:the_nether run kill @e[type=!player,x=160,y=-64,z=-80,dx=15,dy=384,dz=15]"
        );

        assert_eq!(parse_kill_count("Killed 304 entities"), Some(304));
        assert_eq!(parse_kill_count("Killed 1 entity"), Some(1));
        assert_eq!(parse_kill_count("No entity was found"), Some(0));
    }

    #[test]
    fn parses_execute_entity_counts() {
        assert_eq!(parse_execute_count("Test passed. Count: 17"), Some(17));
        assert_eq!(parse_execute_count("Test passed, count: 42"), Some(42));
        assert_eq!(parse_execute_count("Test failed"), Some(0));
    }

    #[test]
    fn parses_player_coordinates_from_minecraft_data_get() {
        let sample = "\
Notch has the following entity data: [123.45d, 64.0d, -789.01d]
Alex has the following entity data: [-40.0d, 70.0d, 120.5d]
Steve has the following entity data: [0.5, 64.0, 10.2]
";
        let players = parse_player_positions(sample);
        assert_eq!(players.len(), 3);
        assert_eq!(players[0].name, "Notch");
        assert_eq!(players[0].chunk_x, 7);
        assert_eq!(players[0].chunk_z, -50);

        assert_eq!(players[1].name, "Alex");
        assert_eq!(players[1].chunk_x, -3);
        assert_eq!(players[1].chunk_z, 7);

        assert_eq!(players[2].name, "Steve");
        assert_eq!(players[2].chunk_x, 0);
        assert_eq!(players[2].chunk_z, 0);
    }

    #[test]
    fn parses_spark_and_vanilla_tps_and_mspt() {
        let spark_sample = "\
TPS from last 5s, 10s, 1m, 5m, 15m:
20.0, 20.0, 19.85, 20.0, 20.0
Tick durations (min/med/95%ile/max) from last 10s, 1m:
24.1/32.5/41.2/54.8, 22.3/31.1/38.9/62.1 ms
";
        let (mspt, tps) = parse_server_perf(spark_sample);
        assert_eq!(tps, Some(20.0));
        assert_eq!(mspt, Some(32.5));

        let vanilla_sample = "TPS from last 1m, 5m, 15m: 18.5, 19.2, 19.8";
        let (v_mspt, v_tps) = parse_server_perf(vanilla_sample);
        assert_eq!(v_tps, Some(18.5));
        assert_eq!(v_mspt, None);
    }

    #[test]
    fn spike_evaluator_triggers_on_large_deltas() {
        let options = LiveOptions::default();
        let base = ChunkMetrics {
            villagers: 2,
            hoppers: 5,
            entity_count: 10,
            payload_size: 20_000,
            ..Default::default()
        };

        let spiked = ChunkMetrics {
            villagers: 18,
            hoppers: 40,
            entity_count: 80,
            payload_size: 250_000,
            ..Default::default()
        };

        let reasons = evaluate_spike(Some(&base), &spiked, 5.0, 35.0, &options);
        assert!(reasons.is_some());
        let r = reasons.unwrap();
        assert!(r.iter().any(|s| s.contains("score jumped")));
        assert!(r.iter().any(|s| s.contains("villagers surged")));
        assert!(r.iter().any(|s| s.contains("hoppers increased")));
        assert!(r.iter().any(|s| s.contains("NBT payload grew")));
    }
}
