use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::{
    model::{ChunkMetrics, Cluster, ScanResult},
    nbt, region,
};

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub threads: Option<usize>,
    pub fast: bool,
    pub eco: bool,
    pub min_score: f64,
}
impl ScanOptions {
    pub fn new(threads: Option<usize>, fast: bool, eco: bool, min_score: f64) -> Self {
        Self {
            threads,
            fast,
            eco,
            min_score,
        }
    }
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self::new(None, false, false, 0.1)
    }
}

#[derive(Debug, Clone)]
pub struct ScoreWeights {
    pub villager: f64,
    pub hopper: f64,
    pub hopper_minecart: f64,
    pub generic_entity: f64,
    pub armor_stand: f64,
    pub redstone_component: f64,
    pub block_entity: f64,
    pub item_frame: f64,
}
impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            villager: 1.2,
            hopper: 0.16,
            hopper_minecart: 1.5,
            generic_entity: 0.035,
            armor_stand: 0.25,
            redstone_component: 0.015,
            block_entity: 0.02,
            item_frame: 0.10,
        }
    }
}

pub fn score(metric: &ChunkMetrics, weights: &ScoreWeights) -> f64 {
    let redstone = metric.redstone_wire
        + metric.repeaters
        + metric.comparators
        + metric.observers
        + metric.pistons;
    let falling_blocks = metric
        .entity_types
        .get("minecraft:falling_block")
        .copied()
        .unwrap_or(0);
    let falling_penalty = if falling_blocks > 30 {
        (falling_blocks as f64 - 30.0) * 0.15
    } else {
        0.0
    };
    let loose_items = metric.dropped_items + metric.exp_orbs;
    let loose_penalty = if loose_items > 40 {
        ((loose_items as f64 - 40.0) * 0.12).min(50.0)
    } else {
        0.0
    };
    let bees = metric
        .entity_types
        .get("minecraft:bee")
        .copied()
        .unwrap_or(0);
    let allays = metric
        .entity_types
        .get("minecraft:allay")
        .copied()
        .unwrap_or(0);
    let complex_mobs = (bees + allays) as f64 * 0.8;

    (metric.villagers as f64 * weights.villager
        + metric.hoppers as f64 * weights.hopper
        + metric.hopper_minecarts as f64 * weights.hopper_minecart
        + metric.entity_count as f64 * weights.generic_entity
        + metric.armor_stands as f64 * weights.armor_stand
        + metric.item_frames as f64 * weights.item_frame
        + redstone as f64 * weights.redstone_component
        + metric.block_entity_count as f64 * weights.block_entity
        + falling_penalty
        + loose_penalty
        + complex_mobs)
        .min(100.0)
}

pub fn scan_world(world: &Path, options: ScanOptions) -> Result<ScanResult> {
    anyhow::ensure!(
        options.min_score.is_finite() && (0.0..=100.0).contains(&options.min_score),
        "min-score must be between 0 and 100"
    );
    anyhow::ensure!(
        options.threads != Some(0),
        "threads must be greater than zero"
    );
    anyhow::ensure!(
        !(options.fast && options.eco),
        "--fast and --eco cannot be combined"
    );
    let regions = find_regions(&world.join("region"))?;
    if regions.is_empty() {
        anyhow::bail!(
            "no .mca files found under {}",
            world.join("region").display()
        );
    }
    let region_count = regions.len();
    let entity_files = find_regions(&world.join("entities"))?;
    let total_files = region_count + entity_files.len();
    let mut pairs: BTreeMap<_, Vec<(PathBuf, bool)>> = BTreeMap::new();
    for (files, entities_only) in [(regions, false), (entity_files, true)] {
        for path in files {
            pairs
                .entry(path.file_name().unwrap().to_owned())
                .or_default()
                .push((path, entities_only));
        }
    }
    let num_cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let requested = options.threads.unwrap_or_else(|| {
        if options.fast {
            num_cpus
        } else if options.eco {
            (num_cpus / 2).max(1)
        } else {
            num_cpus.saturating_sub(1).max(1)
        }
    });
    let worker_threads = requested.min(pairs.len().max(1));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_threads)
        .build()?;
    let progress = indicatif::ProgressBar::new(total_files as u64);
    progress.set_style(indicatif::ProgressStyle::with_template("{spinner:.cyan} Scanning [{bar:24.cyan/blue}] {pos}/{len} files • {elapsed_precise} • ETA {eta_precise}")?.progress_chars("━╸─"));
    let started = Instant::now();
    let weights = ScoreWeights::default();
    let groups: Vec<_> = pairs.into_values().collect();
    let mut result = pool.install(|| {
        groups
            .par_iter()
            .map_init(
                || {
                    (
                        Vec::with_capacity(1024 * 1024),
                        Vec::with_capacity(512 * 1024),
                    )
                },
                |(file_buf, decompressed), files| {
                    let mut combined = HashMap::<(i32, i32), ChunkMetrics>::new();
                    let mut result = FileScan {
                        metrics: Vec::new(),
                        skipped: 0,
                        warnings: Vec::new(),
                        total_scanned: 0,
                    };
                    for (path, entities_only) in files {
                        match scan_file(path, world, *entities_only, file_buf, decompressed) {
                            Ok(scan) => {
                                for metric in scan.metrics {
                                    merge_metric(
                                        combined
                                            .entry((metric.chunk_x, metric.chunk_z))
                                            .or_default(),
                                        metric,
                                    );
                                }
                                result.skipped += scan.skipped;
                                result.warnings.extend(scan.warnings);
                            }
                            Err(error) => {
                                result.skipped += 1;
                                result
                                    .warnings
                                    .push(format!("{}: {error:#}", path.display()));
                            }
                        }
                        progress.inc(1);
                    }
                    result.total_scanned = combined.len() as u64;
                    for mut metric in combined.into_values() {
                        metric.score = score(&metric, &weights);
                        if metric.score >= options.min_score.min(20.0) || metric.is_bloat_hazard() {
                            result.metrics.push(metric);
                        }
                    }
                    result
                },
            )
            .reduce(FileScan::default, |mut left, mut right| {
                left.metrics.append(&mut right.metrics);
                left.skipped += right.skipped;
                left.warnings.append(&mut right.warnings);
                left.total_scanned += right.total_scanned;
                left
            })
    });
    progress.finish_and_clear();
    let mut chunks = std::mem::take(&mut result.metrics);
    let skipped = result.skipped;
    let warnings = result.warnings;
    let total_scanned = result.total_scanned;
    chunks.sort_by_key(|chunk| (chunk.chunk_x, chunk.chunk_z));
    let clusters = detect_clusters(&chunks, 20.0);
    chunks.retain(|chunk| chunk.score >= options.min_score || chunk.is_bloat_hazard());
    let elapsed = started.elapsed().as_secs_f64();
    let speed = if elapsed > 0.0 {
        (total_scanned as f64 / elapsed) as u64
    } else {
        total_scanned
    };
    eprintln!(
        "  \x1b[1;92m✔\x1b[0m Scanned \x1b[1m{total_scanned}\x1b[0m chunks in \x1b[1m{:.2}s\x1b[0m (\x1b[36m{speed}\x1b[0m chunks/s) • {worker_threads} workers • {} retained",
        elapsed,
        chunks.len()
    );
    Ok(ScanResult {
        world: world.display().to_string(),
        chunks,
        clusters,
        skipped,
        warnings,
        total_scanned,
        format_version: 1,
        min_score: options.min_score,
        scanned_at_unix: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        ),
    })
}

fn find_regions(directory: &Path) -> Result<Vec<PathBuf>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "mca"))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

#[derive(Default)]
struct FileScan {
    metrics: Vec<ChunkMetrics>,
    skipped: u64,
    warnings: Vec<String>,
    total_scanned: u64,
}

fn scan_file(
    path: &Path,
    world: &Path,
    entities_only: bool,
    file_buf: &mut Vec<u8>,
    decompress_buf: &mut Vec<u8>,
) -> Result<FileScan> {
    let (region_x, region_z) =
        region::region_coordinates(path).context("invalid region filename")?;
    let dimension = world
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("world")
        .to_string();
    let mut output = Vec::with_capacity(512);
    let mut warnings = Vec::new();
    let mut skipped = 0;

    region::for_each_chunk_with_buffers(path, file_buf, decompress_buf, |slot, chunk_res| {
        let bytes = match chunk_res {
            Ok(value) => value,
            Err(error) => {
                skipped += 1;
                warnings.push(format!("WARN {}: {error:#}", path.display()));
                return;
            }
        };
        let local_x = (slot % 32) as i32;
        let local_z = (slot / 32) as i32;
        let Some(x) = region_x
            .checked_mul(32)
            .and_then(|v| v.checked_add(local_x))
        else {
            skipped += 1;
            warnings.push(format!("{}: chunk X coordinate overflow", path.display()));
            return;
        };
        let Some(z) = region_z
            .checked_mul(32)
            .and_then(|v| v.checked_add(local_z))
        else {
            skipped += 1;
            warnings.push(format!("{}: chunk Z coordinate overflow", path.display()));
            return;
        };
        let metric = match nbt::extract(bytes, &dimension, x, z, entities_only) {
            Ok(metric) => metric,
            Err(error) => {
                skipped += 1;
                warnings.push(format!(
                    "{} chunk slot {slot}: invalid NBT: {error:#}",
                    path.display()
                ));
                return;
            }
        };
        if metric.is_oversized {
            warnings.push(format!(
                "{} chunk [{}, {}]: large saved NBT ({:.2} MiB); inspect its contents",
                path.display(),
                metric.chunk_x,
                metric.chunk_z,
                metric.payload_size as f64 / 1_048_576.0
            ));
        }

        output.push(metric);
    })?;

    Ok(FileScan {
        total_scanned: output.len() as u64,
        metrics: output,
        skipped,
        warnings,
    })
}

fn merge_metric(target: &mut ChunkMetrics, incoming: ChunkMetrics) {
    if target.dimension.is_empty() {
        target.dimension = incoming.dimension;
        target.chunk_x = incoming.chunk_x;
        target.chunk_z = incoming.chunk_z;
        target.data_version = incoming.data_version;
    }
    target.payload_size = target.payload_size.max(incoming.payload_size);
    target.is_oversized |= incoming.is_oversized;
    target.entity_count += incoming.entity_count;
    target.block_entity_count += incoming.block_entity_count;
    target.villagers += incoming.villagers;
    target.armor_stands += incoming.armor_stands;
    target.dropped_items += incoming.dropped_items;
    target.item_frames += incoming.item_frames;
    target.exp_orbs += incoming.exp_orbs;
    target.scheduled_ticks += incoming.scheduled_ticks;
    target.minecarts += incoming.minecarts;
    target.hopper_minecarts += incoming.hopper_minecarts;
    target.hoppers += incoming.hoppers;
    target.furnaces += incoming.furnaces;
    target.chests += incoming.chests;
    target.shulkers += incoming.shulkers;
    target.signs += incoming.signs;
    target.skulls += incoming.skulls;
    target.stored_items += incoming.stored_items;
    target.spawners += incoming.spawners;
    target.redstone_wire += incoming.redstone_wire;
    target.repeaters += incoming.repeaters;
    target.comparators += incoming.comparators;
    target.observers += incoming.observers;
    target.pistons += incoming.pistons;
    for (id, count) in incoming.entity_types {
        *target.entity_types.entry(id).or_default() += count;
    }
    for (id, count) in incoming.block_entity_types {
        *target.block_entity_types.entry(id).or_default() += count;
    }
    for (id, count) in incoming.block_types {
        *target.block_types.entry(id).or_default() += count;
    }
}

pub fn detect_clusters(chunks: &[ChunkMetrics], threshold: f64) -> Vec<Cluster> {
    let heavy: HashMap<_, _> = chunks
        .iter()
        .filter(|c| c.score >= threshold)
        .map(|c| ((c.chunk_x, c.chunk_z), c))
        .collect();
    let mut seen = HashSet::new();
    let mut clusters = Vec::new();
    let mut starts: Vec<_> = heavy.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        if !seen.insert(start) {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut group = Vec::new();
        while let Some(pos) = queue.pop_front() {
            let chunk = heavy[&pos];
            group.push(chunk);
            for next in [
                (pos.0 + 1, pos.1),
                (pos.0 - 1, pos.1),
                (pos.0, pos.1 + 1),
                (pos.0, pos.1 - 1),
            ] {
                if heavy.contains_key(&next) && seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        if group.len() > 1 {
            let n = group.len() as i64;
            clusters.push(Cluster {
                center: (
                    (group.iter().map(|c| c.chunk_x as i64).sum::<i64>() / n) as i32,
                    (group.iter().map(|c| c.chunk_z as i64).sum::<i64>() / n) as i32,
                ),
                chunks: group.iter().map(|c| (c.chunk_x, c.chunk_z)).collect(),
                total_score: group.iter().map(|c| c.score).sum(),
                entities: group.iter().map(|c| c.entity_count).sum(),
                villagers: group.iter().map(|c| c.villagers).sum(),
                hoppers: group.iter().map(|c| c.hoppers).sum(),
                hopper_minecarts: group.iter().map(|c| c.hopper_minecarts).sum(),
                redstone_components: group
                    .iter()
                    .map(|c| {
                        c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons
                    })
                    .sum(),
            });
        }
    }
    clusters.sort_by(|a, b| {
        b.total_score
            .total_cmp(&a.total_score)
            .then_with(|| a.center.cmp(&b.center))
    });
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::ZlibEncoder};
    use serde_json::json;
    use std::io::Write;
    #[test]
    fn score_is_bounded() {
        let m = ChunkMetrics {
            villagers: 1000,
            ..Default::default()
        };
        assert_eq!(score(&m, &ScoreWeights::default()), 100.0);
    }
    #[test]
    fn adjacent_heavy_chunks_cluster() {
        let chunks = vec![
            ChunkMetrics {
                chunk_x: 1,
                chunk_z: 1,
                score: 30.,
                ..Default::default()
            },
            ChunkMetrics {
                chunk_x: 2,
                chunk_z: 1,
                score: 30.,
                ..Default::default()
            },
        ];
        assert_eq!(detect_clusters(&chunks, 20.).len(), 1);
    }

    #[test]
    fn scans_a_minimal_anvil_region() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let region_dir = temp.path().join("region");
        std::fs::create_dir(&region_dir).expect("region directory");
        let nbt = fastnbt::to_bytes(&json!({"DataVersion": 3700, "xPos": 2, "zPos": -3, "Entities": [{"id":"minecraft:villager"}], "block_entities": [{"id":"minecraft:hopper"}]})).expect("serialize nbt");
        let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(&nbt).expect("compress nbt");
        let compressed = compressed.finish().expect("finish compression");
        let length = (compressed.len() + 1) as u32;
        let sectors = ((length as usize + 4).div_ceil(crate::region::SECTOR)) as u8;
        let mut region = vec![0_u8; crate::region::SECTOR * (2 + sectors as usize)];
        region[0..4].copy_from_slice(&[0, 0, 2, sectors]);
        let offset = crate::region::SECTOR * 2;
        region[offset..offset + 4].copy_from_slice(&length.to_be_bytes());
        region[offset + 4] = 2;
        region[offset + 5..offset + 5 + compressed.len()].copy_from_slice(&compressed);
        let region_path = region_dir.join("r.0.0.mca");
        std::fs::write(&region_path, region).expect("region fixture");
        let before = std::fs::read(&region_path).expect("fixture before scan");
        let result = scan_world(temp.path(), ScanOptions::new(Some(1), false, false, 0.0))
            .expect("scan fixture");
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].villagers, 1);
        assert_eq!(result.chunks[0].hoppers, 1);
        assert_eq!(
            (result.chunks[0].chunk_x, result.chunks[0].chunk_z),
            (2, -3)
        );
        assert_eq!(
            std::fs::read(region_path).expect("fixture after scan"),
            before
        );
    }

    #[test]
    fn corrupt_chunk_does_not_abort_its_region() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let region_dir = temp.path().join("region");
        std::fs::create_dir(&region_dir).expect("region directory");
        let nbt = fastnbt::to_bytes(&json!({"xPos":0,"zPos":0})).expect("serialize nbt");
        let mut compressed = ZlibEncoder::new(Vec::new(), Compression::default());
        compressed.write_all(&nbt).expect("compress nbt");
        let compressed = compressed.finish().expect("finish compression");
        let length = (compressed.len() + 1) as u32;
        let sectors = ((length as usize + 4).div_ceil(crate::region::SECTOR)) as u8;
        let mut region = vec![0_u8; crate::region::SECTOR * (2 + sectors as usize)];
        region[0..4].copy_from_slice(&[0, 0, 2, sectors]);
        region[4..8].copy_from_slice(&[0, 0, 99, 1]);
        let offset = crate::region::SECTOR * 2;
        region[offset..offset + 4].copy_from_slice(&length.to_be_bytes());
        region[offset + 4] = 2;
        region[offset + 5..offset + 5 + compressed.len()].copy_from_slice(&compressed);
        std::fs::write(region_dir.join("r.0.0.mca"), region).expect("region fixture");
        let result = scan_world(temp.path(), ScanOptions::new(Some(1), false, false, 0.0))
            .expect("scan fixture");
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.warnings.len(), 1);
    }
    fn write_fixture(path: &std::path::Path, payload: &[u8]) {
        let mut compressor = ZlibEncoder::new(Vec::new(), Compression::fast());
        compressor.write_all(payload).unwrap();
        let data = compressor.finish().unwrap();
        let size = data.len() + 1;
        let sectors = (size + 4).div_ceil(crate::region::SECTOR);
        let mut bytes = vec![0; crate::region::SECTOR * (2 + sectors)];
        bytes[..4].copy_from_slice(&[0, 0, 2, sectors as u8]);
        bytes[8192..8196].copy_from_slice(&(size as u32).to_be_bytes());
        bytes[8196] = 2;
        bytes[8197..8197 + data.len()].copy_from_slice(&data);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn filters_only_after_merging_entity_regions() {
        let dir = tempfile::tempdir().unwrap();
        write_fixture(
            &dir.path().join("region/r.0.0.mca"),
            &fastnbt::to_bytes(&json!({"xPos":0,"zPos":0})).unwrap(),
        );
        write_fixture(
            &dir.path().join("entities/r.0.0.mca"),
            &fastnbt::to_bytes(&json!({"Entities":[{"id":"minecraft:villager"}]})).unwrap(),
        );
        let result = scan_world(dir.path(), ScanOptions::new(Some(2), false, false, 1.0)).unwrap();
        assert_eq!(result.total_scanned, 1);
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].villagers, 1);
        assert!(result.chunks[0].payload_size > 0);
    }

    #[test]
    fn retains_oversized_chunks_even_with_zero_load_score() {
        let dir = tempfile::tempdir().unwrap();
        let mut fields = std::collections::HashMap::new();
        fields.insert(
            "UnusedLighting".to_owned(),
            fastnbt::Value::ByteArray(fastnbt::ByteArray::new(vec![0; 1_048_576])),
        );
        let payload = fastnbt::to_bytes(&fastnbt::Value::Compound(fields)).unwrap();
        write_fixture(&dir.path().join("region/r.0.0.mca"), &payload);
        let result = scan_world(dir.path(), ScanOptions::default()).unwrap();
        assert_eq!(result.chunks.len(), 1);
        assert!(result.chunks[0].is_oversized);
        assert!(result.chunks[0].payload_size >= 1_048_576);
        assert_eq!(result.chunks[0].score, 0.0);
        assert_eq!(result.warnings.len(), 1);
    }
}
