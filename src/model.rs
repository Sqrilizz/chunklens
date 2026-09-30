use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use flate2::{Compression, bufread::GzDecoder, write::GzEncoder};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChunkMetrics {
    pub dimension: String,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub data_version: Option<i32>,
    pub entity_count: u64,
    pub block_entity_count: u64,
    pub villagers: u64,
    pub armor_stands: u64,
    pub dropped_items: u64,
    pub minecarts: u64,
    pub hopper_minecarts: u64,
    pub hoppers: u64,
    pub furnaces: u64,
    pub chests: u64,
    pub spawners: u64,
    pub redstone_wire: u64,
    pub repeaters: u64,
    pub comparators: u64,
    pub observers: u64,
    pub pistons: u64,
    pub entity_types: BTreeMap<String, u64>,
    pub block_entity_types: BTreeMap<String, u64>,
    pub block_types: BTreeMap<String, u64>,
    pub score: f64,
    #[serde(default)]
    pub payload_size: u64,
    #[serde(default)]
    pub is_oversized: bool,
    #[serde(default)]
    pub item_frames: u64,
    #[serde(default)]
    pub exp_orbs: u64,
    #[serde(default)]
    pub scheduled_ticks: u64,
    #[serde(default)]
    pub shulkers: u64,
    #[serde(default)]
    pub signs: u64,
    #[serde(default)]
    pub skulls: u64,
    #[serde(default)]
    pub stored_items: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cluster {
    pub chunks: Vec<(i32, i32)>,
    pub center: (i32, i32),
    pub total_score: f64,
    pub entities: u64,
    pub villagers: u64,
    pub hoppers: u64,
    pub hopper_minecarts: u64,
    pub redstone_components: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanResult {
    pub world: String,
    pub chunks: Vec<ChunkMetrics>,
    pub clusters: Vec<Cluster>,
    pub skipped: u64,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub total_scanned: u64,
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub min_score: f64,
    #[serde(default)]
    pub scanned_at_unix: Option<u64>,
}

impl ChunkMetrics {
    pub fn block_x(&self) -> i64 {
        i64::from(self.chunk_x) * 16 + 8
    }

    pub fn block_z(&self) -> i64 {
        i64::from(self.chunk_z) * 16 + 8
    }

    pub fn tp_command(&self) -> String {
        format!("/tp @s {} ~ {}", self.block_x(), self.block_z())
    }

    #[allow(dead_code)]
    pub fn severity(&self) -> &'static str {
        if self.is_oversized {
            return "LARGE NBT";
        }
        match self.score {
            s if s >= 80.0 => "SCORE 80+",
            s if s >= 60.0 => "SCORE 60+",
            s if s >= 40.0 => "SCORE 40+",
            s if s >= 20.0 => "SCORE 20+",
            _ => "SCORE <20",
        }
    }

    pub fn severity_colored(&self) -> String {
        let color = if self.is_oversized {
            "1;95"
        } else {
            match self.score {
                s if s >= 80.0 => "1;91",
                s if s >= 60.0 => "1;93",
                s if s >= 40.0 => "93",
                s if s >= 20.0 => "92",
                _ => "90",
            }
        };
        format!("\x1b[{color}m{}\x1b[0m", self.severity())
    }

    pub fn culprits_summary(&self) -> String {
        if self.is_oversized {
            return format!("LARGE NBT ({} KB)", self.payload_size / 1024);
        }

        let mut parts = Vec::new();

        if self.villagers > 0 {
            let v_str = if self.villagers == 1 {
                "villager"
            } else {
                "villagers"
            };
            parts.push(format!("{} {}", self.villagers, v_str));
        }

        if self.hoppers > 0 || self.hopper_minecarts > 0 {
            let h_str = if self.hoppers == 1 {
                "hopper"
            } else {
                "hoppers"
            };
            let c_str = if self.hopper_minecarts == 1 {
                "cart"
            } else {
                "carts"
            };
            if self.hopper_minecarts > 0 && self.hoppers > 0 {
                parts.push(format!(
                    "{} {} ({} {})",
                    self.hoppers, h_str, self.hopper_minecarts, c_str
                ));
            } else if self.hoppers > 0 {
                parts.push(format!("{} {}", self.hoppers, h_str));
            } else {
                parts.push(format!("{} hopper {}", self.hopper_minecarts, c_str));
            }
        }

        if self.dropped_items > 30 {
            parts.push(format!("{} dropped items", self.dropped_items));
        }

        if self.exp_orbs > 20 {
            parts.push(format!("{} xp orbs", self.exp_orbs));
        }

        if self.item_frames > 20 {
            parts.push(format!("{} item frames", self.item_frames));
        }

        if self.armor_stands > 20 {
            parts.push(format!("{} armor stands", self.armor_stands));
        }

        let active_redstone = self.repeaters + self.comparators + self.observers + self.pistons;
        if active_redstone > 15 {
            parts.push(format!("{} redstone", active_redstone));
        }

        let other_entities = self.entity_count.saturating_sub(self.villagers);
        if other_entities > 0 && (parts.is_empty() || other_entities > 30) {
            let e_str = if self.entity_count == 1 {
                "entity"
            } else {
                "entities"
            };
            parts.push(format!("{} {}", self.entity_count, e_str));
        }

        if parts.is_empty() {
            if self.block_entity_count > 0 {
                format!(
                    "{} block entities, {} entities",
                    self.block_entity_count, self.entity_count
                )
            } else {
                let e_str = if self.entity_count == 1 {
                    "entity"
                } else {
                    "entities"
                };
                format!("{} {}", self.entity_count, e_str)
            }
        } else {
            parts.join(", ")
        }
    }

    pub fn bloat_score(&self) -> f64 {
        let size_score = (self.payload_size as f64 / 1024.0) * 0.15;
        let container_score = (self.shulkers as f64 * 2.0)
            + (self.chests as f64 * 0.4)
            + (self.stored_items as f64 * 0.01)
            + (self.block_entity_count as f64 * 0.05);
        let entity_score = (self.dropped_items as f64 * 0.4)
            + (self.item_frames as f64 * 0.5)
            + (self.skulls as f64 * 0.5)
            + (self.armor_stands as f64 * 0.3);
        let tick_score = (self.scheduled_ticks as f64 / 100.0) * 0.5;

        size_score + container_score + entity_score + tick_score
    }

    pub fn is_bloat_hazard(&self) -> bool {
        self.is_oversized
            || self.payload_size >= 400 * 1024
            || self.shulkers >= 30
            || (self.chests + self.shulkers) >= 150
            || self.stored_items >= 1500
            || self.dropped_items >= 200
            || self.item_frames >= 50
            || self.skulls >= 50
            || self.scheduled_ticks >= 1500
    }

    pub fn bloat_hazard_badge(&self) -> (&'static str, &'static str) {
        if self.is_oversized || self.payload_size >= 1_048_576 {
            ("OVERSIZED", "\x1b[1;95mOVERSIZED\x1b[0m")
        } else if self.shulkers >= 30
            || (self.chests + self.shulkers) >= 150
            || self.stored_items >= 1500
        {
            ("DENSE STORAGE", "\x1b[1;93mDENSE STORAGE\x1b[0m")
        } else if self.scheduled_ticks >= 1500 {
            ("TICK FLOOD", "\x1b[1;91mTICK FLOOD\x1b[0m")
        } else if self.dropped_items >= 200 || self.entity_count >= 400 {
            ("ITEM FLOOD", "\x1b[1;91mITEM FLOOD\x1b[0m")
        } else if self.item_frames >= 50 || self.skulls >= 50 {
            ("RENDER LAG", "\x1b[1;96mRENDER LAG\x1b[0m")
        } else if self.payload_size >= 400 * 1024 {
            ("HEAVY NBT", "\x1b[93mHEAVY NBT\x1b[0m")
        } else {
            ("RISK", "\x1b[90mRISK\x1b[0m")
        }
    }

    pub fn bloat_details(&self) -> String {
        let mut parts = Vec::new();
        let size_kb = self.payload_size / 1024;
        if size_kb >= 1024 {
            parts.push(format!("{:.2} MiB NBT", size_kb as f64 / 1024.0));
        } else if size_kb >= 200 {
            parts.push(format!("{} KiB NBT", size_kb));
        }

        if self.shulkers > 0 {
            let s_str = if self.shulkers == 1 {
                "shulker"
            } else {
                "shulkers"
            };
            parts.push(format!("{} {}", self.shulkers, s_str));
        }
        if self.chests > 30 {
            parts.push(format!("{} chests/barrels", self.chests));
        }
        if self.stored_items > 300 {
            parts.push(format!("{} items in containers", self.stored_items));
        }
        if self.item_frames > 20 {
            parts.push(format!("{} item frames", self.item_frames));
        }
        if self.skulls > 20 {
            parts.push(format!("{} skulls", self.skulls));
        }
        if self.signs > 30 {
            parts.push(format!("{} signs", self.signs));
        }
        if self.dropped_items > 50 {
            parts.push(format!("{} dropped items", self.dropped_items));
        }
        if self.scheduled_ticks > 500 {
            parts.push(format!("{} scheduled ticks", self.scheduled_ticks));
        }

        if parts.is_empty() {
            format!(
                "{} KiB, {} block entities",
                size_kb, self.block_entity_count
            )
        } else {
            parts.join(", ")
        }
    }
}

impl Cluster {
    pub fn culprits_summary(&self) -> String {
        let mut parts = Vec::new();
        if self.villagers > 0 {
            let v_str = if self.villagers == 1 {
                "villager"
            } else {
                "villagers"
            };
            parts.push(format!("{} {}", self.villagers, v_str));
        }
        if self.hoppers > 0 || self.hopper_minecarts > 0 {
            let h_str = if self.hoppers == 1 {
                "hopper"
            } else {
                "hoppers"
            };
            let c_str = if self.hopper_minecarts == 1 {
                "cart"
            } else {
                "carts"
            };
            if self.hopper_minecarts > 0 && self.hoppers > 0 {
                parts.push(format!(
                    "{} {} ({} {})",
                    self.hoppers, h_str, self.hopper_minecarts, c_str
                ));
            } else if self.hoppers > 0 {
                parts.push(format!("{} {}", self.hoppers, h_str));
            } else {
                parts.push(format!("{} hopper {}", self.hopper_minecarts, c_str));
            }
        }
        let other_entities = self.entities.saturating_sub(self.villagers);
        if other_entities > 30 {
            let e_str = if self.entities == 1 {
                "entity"
            } else {
                "entities"
            };
            parts.push(format!("{} {}", self.entities, e_str));
        }
        if self.redstone_components > 20 {
            parts.push(format!("{} redstone", self.redstone_components));
        }
        if parts.is_empty() {
            let e_str = if self.entities == 1 {
                "entity"
            } else {
                "entities"
            };
            format!("{} {}", self.entities, e_str)
        } else {
            parts.join(", ")
        }
    }
}

impl ScanResult {
    pub fn read_json(path: &Path) -> Result<Self> {
        const MAX_REPORT_BYTES: u64 = 256 * 1024 * 1024;
        let mut input = BufReader::with_capacity(
            256 * 1024,
            File::open(path).with_context(|| format!("opening {}", path.display()))?,
        );
        let compressed = input.fill_buf()?.starts_with(&[0x1f, 0x8b]);
        ensure!(
            compressed || input.get_ref().metadata()?.len() <= MAX_REPORT_BYTES,
            "report exceeds 256 MiB decoded size limit; rescan with --min-score or export a filtered report"
        );
        let mut limited: Box<dyn Read> = if compressed {
            Box::new(GzDecoder::new(input).take(MAX_REPORT_BYTES + 1))
        } else {
            Box::new(input.take(MAX_REPORT_BYTES + 1))
        };
        let result: Self = serde_json::from_reader(&mut limited)
            .with_context(|| format!("reading report {} (maximum decoded size: 256 MiB; rescan with --min-score or export a filtered report)", path.display()))?;
        ensure!(
            result.format_version <= 1,
            "report format {} is newer than this ChunkLens build",
            result.format_version
        );
        ensure!(
            result
                .chunks
                .iter()
                .all(|c| c.score.is_finite() && (0.0..=100.0).contains(&c.score)),
            "report contains an invalid chunk score"
        );
        Ok(result)
    }

    pub fn total_chunks(&self) -> u64 {
        self.total_scanned.max(self.chunks.len() as u64)
    }

    pub fn top_by(&self, category: &str, count: usize) -> Vec<&ChunkMetrics> {
        if count == 0 {
            return Vec::new();
        }
        let mut chunks: Vec<_> = self.chunks.iter().collect();
        let compare = |a: &&ChunkMetrics, b: &&ChunkMetrics| {
            metric_value(b, category)
                .total_cmp(&metric_value(a, category))
                .then_with(|| {
                    (&a.dimension, a.chunk_x, a.chunk_z).cmp(&(&b.dimension, b.chunk_x, b.chunk_z))
                })
        };
        if count < chunks.len() {
            chunks.select_nth_unstable_by(count, compare);
            chunks.truncate(count);
        }
        chunks.sort_unstable_by(compare);
        chunks
    }

    pub fn find(&self, x: i32, z: i32) -> Option<&ChunkMetrics> {
        self.chunks
            .iter()
            .find(|chunk| chunk.chunk_x == x && chunk.chunk_z == z)
    }

    pub fn write_json(&self, path: &Path, min_score: f64) -> Result<()> {
        #[derive(Serialize)]
        struct Report<'a> {
            format_version: u32,
            world: &'a str,
            scanned_at_unix: Option<u64>,
            min_score: f64,
            total_scanned: u64,
            chunks: FilteredChunks<'a>,
            clusters: &'a [Cluster],
            skipped: u64,
            warnings: &'a [String],
        }
        ensure!(
            min_score.is_finite() && (0.0..=100.0).contains(&min_score),
            "invalid export score threshold"
        );
        let report = Report {
            format_version: 1,
            world: &self.world,
            scanned_at_unix: self.scanned_at_unix,
            min_score: self.min_score.max(min_score),
            total_scanned: self.total_chunks(),
            chunks: FilteredChunks(&self.chunks, min_score),
            clusters: &self.clusters,
            skipped: self.skipped,
            warnings: &self.warnings,
        };
        write_report(path, "json", |writer| {
            serde_json::to_writer(writer, &report)?;
            Ok(())
        })
    }

    pub fn write_csv(&self, path: &Path, min_score: f64) -> Result<()> {
        ensure!(
            min_score.is_finite() && (0.0..=100.0).contains(&min_score),
            "invalid export score threshold"
        );
        write_report(path, "csv", |writer| {
            writeln!(
                writer,
                "dimension,chunk_x,chunk_z,block_x,block_z,tp_command,data_version,entity_count,block_entity_count,villagers,armor_stands,dropped_items,item_frames,exp_orbs,minecarts,hopper_minecarts,hoppers,furnaces,chests,spawners,redstone_wire,repeaters,comparators,observers,pistons,scheduled_ticks,score,payload_size,is_oversized,shulkers,signs,skulls,stored_items"
            )?;
            for c in self
                .chunks
                .iter()
                .filter(|c| c.score >= min_score || c.is_bloat_hazard())
            {
                writeln!(
                    writer,
                    "\"{}\",{},{},{},{},\"{}\",{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.3},{},{},{},{},{},{}",
                    c.dimension.replace('"', "\"\""),
                    c.chunk_x,
                    c.chunk_z,
                    c.block_x(),
                    c.block_z(),
                    c.tp_command(),
                    c.data_version.map_or(String::new(), |v| v.to_string()),
                    c.entity_count,
                    c.block_entity_count,
                    c.villagers,
                    c.armor_stands,
                    c.dropped_items,
                    c.item_frames,
                    c.exp_orbs,
                    c.minecarts,
                    c.hopper_minecarts,
                    c.hoppers,
                    c.furnaces,
                    c.chests,
                    c.spawners,
                    c.redstone_wire,
                    c.repeaters,
                    c.comparators,
                    c.observers,
                    c.pistons,
                    c.scheduled_ticks,
                    c.score,
                    c.payload_size,
                    c.is_oversized,
                    c.shulkers,
                    c.signs,
                    c.skulls,
                    c.stored_items
                )?;
            }
            Ok(())
        })
    }
}

struct FilteredChunks<'a>(&'a [ChunkMetrics], f64);

impl Serialize for FilteredChunks<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for chunk in self
            .0
            .iter()
            .filter(|c| c.score >= self.1 || c.is_bloat_hazard())
        {
            seq.serialize_element(chunk)?;
        }
        seq.end()
    }
}

fn write_report(
    path: &Path,
    format: &str,
    write: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<()> {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    ensure!(
        name.ends_with(&format!(".{format}")) || name.ends_with(&format!(".{format}.gz")),
        "report path must end in .{format} or .{format}.gz"
    );
    let parent = path
        .parent()
        .context("invalid destination path: missing parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary export in {}", parent.display()))?;
    {
        let mut buffered = BufWriter::with_capacity(256 * 1024, temporary.as_file_mut());
        if path.extension().is_some_and(|e| e == "gz") {
            let mut compressed = GzEncoder::new(&mut buffered, Compression::default());
            write(&mut compressed)?;
            compressed.finish()?;
        } else {
            write(&mut buffered)?;
        }
        buffered.flush()?;
    }
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .with_context(|| format!("saving report {}", path.display()))?;
    Ok(())
}

pub fn metric_value(chunk: &ChunkMetrics, category: &str) -> f64 {
    match category {
        "bloat" => chunk.bloat_score(),
        "size" | "nbt" => chunk.payload_size as f64,
        "shulkers" => chunk.shulkers as f64,
        "chests" => chunk.chests as f64,
        "items" => chunk.stored_items as f64,
        "entities" => chunk.entity_count as f64,
        "villagers" => chunk.villagers as f64,
        "hoppers" => chunk.hoppers as f64,
        "minecarts" => chunk.minecarts as f64,
        "redstone" => {
            (chunk.redstone_wire
                + chunk.repeaters
                + chunk.comparators
                + chunk.observers
                + chunk.pistons) as f64
        }
        _ => chunk.score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_plain_and_compressed_reports_and_retains_large_nbt() {
        let dir = tempfile::tempdir().unwrap();
        let report = ScanResult {
            world: "world".into(),
            total_scanned: 9000,
            scanned_at_unix: Some(123),
            chunks: vec![
                ChunkMetrics {
                    dimension: "world".into(),
                    score: 0.0,
                    is_oversized: true,
                    payload_size: 2_000_000,
                    ..Default::default()
                },
                ChunkMetrics {
                    score: 10.0,
                    ..Default::default()
                },
                ChunkMetrics {
                    score: 0.0,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        for name in ["report.json", "report.json.gz"] {
            let path = dir.path().join(name);
            report.write_json(&path, 1.0).unwrap();
            let loaded = ScanResult::read_json(&path).unwrap();
            assert_eq!(loaded.chunks.len(), 2);
            assert_eq!(loaded.total_chunks(), 9000);
            assert_eq!(loaded.scanned_at_unix, Some(123));
            assert!(loaded.chunks[0].is_oversized);
        }
        report
            .write_csv(&dir.path().join("report.csv.gz"), 1.0)
            .unwrap();
        assert!(
            report
                .write_json(&dir.path().join("level.dat"), 0.0)
                .is_err()
        );
    }

    #[test]
    fn failed_export_preserves_existing_report() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, b"previous").unwrap();
        let result = write_report(&path, "json", |writer| {
            writer.write_all(b"partial")?;
            anyhow::bail!("interrupted");
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
    }

    #[test]
    fn rejects_oversized_report_before_deserializing() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(256 * 1024 * 1024 + 1).unwrap();
        let error = ScanResult::read_json(file.path()).unwrap_err();
        assert!(error.to_string().contains("256 MiB"));
    }

    #[test]
    fn top_selection_is_deterministic_and_handles_limits() {
        let report = ScanResult {
            chunks: (0..100)
                .rev()
                .map(|x| ChunkMetrics {
                    chunk_x: x,
                    score: (x % 5) as f64,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        assert!(report.top_by("score", 0).is_empty());
        assert_eq!(
            report
                .top_by("score", 3)
                .iter()
                .map(|c| c.chunk_x)
                .collect::<Vec<_>>(),
            vec![4, 9, 14]
        );
        assert_eq!(report.top_by("score", 200).len(), 100);
        assert_eq!(
            ChunkMetrics {
                chunk_x: i32::MAX,
                ..Default::default()
            }
            .block_x(),
            34_359_738_360
        );
    }

    #[test]
    fn bloat_hazards_and_dupe_stash_detection() {
        let oversized = ChunkMetrics {
            chunk_x: 1,
            chunk_z: 1,
            payload_size: 1_500_000,
            is_oversized: true,
            ..Default::default()
        };
        assert!(oversized.is_bloat_hazard());
        assert_eq!(oversized.bloat_hazard_badge().0, "OVERSIZED");

        let stash = ChunkMetrics {
            chunk_x: 2,
            chunk_z: 2,
            shulkers: 45,
            chests: 120,
            stored_items: 2500,
            payload_size: 600_000,
            ..Default::default()
        };
        assert!(stash.is_bloat_hazard());
        assert_eq!(stash.bloat_hazard_badge().0, "DENSE STORAGE");
        assert!(stash.bloat_score() > 100.0);

        let report = ScanResult {
            chunks: vec![oversized, stash],
            ..Default::default()
        };
        let top_bloat = report.top_by("bloat", 2);
        assert_eq!(top_bloat.len(), 2);
    }
}
