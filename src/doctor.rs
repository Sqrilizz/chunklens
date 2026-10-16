use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RuleSeverity {
    Pass,
    Tip,
    Warn,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorFinding {
    pub severity: RuleSeverity,
    pub title: String,
    pub file: String,
    pub parameter: String,
    pub current_value: String,
    pub recommended_value: String,
    pub description: String,
}

pub struct DoctorReport {
    pub server_path: PathBuf,
    pub software: Option<String>,
    pub findings: Vec<DoctorFinding>,
}

impl DoctorReport {
    pub fn pass_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == RuleSeverity::Pass)
            .count()
    }

    pub fn warn_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == RuleSeverity::Warn)
            .count()
    }

    pub fn tip_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == RuleSeverity::Tip)
            .count()
    }
}

pub fn diagnose_server(server_dir: &Path) -> Result<DoctorReport> {
    let root = server_dir
        .canonicalize()
        .unwrap_or_else(|_| server_dir.to_path_buf());
    let mut findings = Vec::new();

    // 1. Inspect server.properties
    let prop_path = root.join("server.properties");
    if let Ok(props) = crate::discovery::parse_properties(&prop_path) {
        audit_server_properties(&props, &mut findings);
    } else {
        findings.push(DoctorFinding {
            severity: RuleSeverity::Tip,
            title: "Missing server.properties".to_string(),
            file: "server.properties".to_string(),
            parameter: "N/A".to_string(),
            current_value: "missing".to_string(),
            recommended_value: "present".to_string(),
            description: "No server.properties found in the inspected directory.".to_string(),
        });
    }

    // 2. Inspect Paper configuration
    let paper_paths = [
        root.join("config/paper-world-defaults.yml"),
        root.join("config/paper-global.yml"),
        root.join("paper.yml"),
    ];
    let mut paper_found = false;
    for path in &paper_paths {
        if let Ok(content) = fs::read_to_string(path) {
            audit_paper_config(
                &content,
                path.file_name().unwrap().to_str().unwrap(),
                &mut findings,
            );
            paper_found = true;
            break;
        }
    }

    // 3. Inspect Purpur configuration
    let purpur_path = root.join("purpur.yml");
    if let Ok(content) = fs::read_to_string(&purpur_path) {
        audit_purpur_config(&content, &mut findings);
    }

    // Determine detected software
    let server_meta = crate::discovery::server_at(&root).ok();
    let software = server_meta.and_then(|s| s.software).or_else(|| {
        if purpur_path.is_file() {
            Some("purpur".to_string())
        } else if paper_found {
            Some("paper".to_string())
        } else {
            None
        }
    });

    Ok(DoctorReport {
        server_path: root,
        software,
        findings,
    })
}

fn audit_server_properties(props: &HashMap<String, String>, findings: &mut Vec<DoctorFinding>) {
    // simulation-distance
    if let Some(val) = props
        .get("simulation-distance")
        .and_then(|s| s.parse::<u32>().ok())
    {
        if val > 6 {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Warn,
                title: "High Simulation Distance".to_string(),
                file: "server.properties".to_string(),
                parameter: "simulation-distance".to_string(),
                current_value: val.to_string(),
                recommended_value: "4 - 6".to_string(),
                description: "Simulation distance controls the radius of ticked chunks. Values > 6 exponentially increase mob AI and redstone load.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Optimal Simulation Distance".to_string(),
                file: "server.properties".to_string(),
                parameter: "simulation-distance".to_string(),
                current_value: val.to_string(),
                recommended_value: "4 - 6".to_string(),
                description: "Simulation distance is in the recommended performance range."
                    .to_string(),
            });
        }
    }

    // view-distance
    if let Some(val) = props
        .get("view-distance")
        .and_then(|s| s.parse::<u32>().ok())
    {
        if val > 10 {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Warn,
                title: "High View Distance".to_string(),
                file: "server.properties".to_string(),
                parameter: "view-distance".to_string(),
                current_value: val.to_string(),
                recommended_value: "8 - 10".to_string(),
                description: "High view distances consume substantial bandwidth and server RAM. Consider pairing with a chunk pre-generator.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Optimal View Distance".to_string(),
                file: "server.properties".to_string(),
                parameter: "view-distance".to_string(),
                current_value: val.to_string(),
                recommended_value: "8 - 10".to_string(),
                description: "View distance is within balanced limits.".to_string(),
            });
        }
    }

    // sync-chunk-writes
    if let Some(sync) = props.get("sync-chunk-writes") {
        if sync.trim().eq_ignore_ascii_case("true") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Tip,
                title: "Synchronous Chunk Writes Enabled".to_string(),
                file: "server.properties".to_string(),
                parameter: "sync-chunk-writes".to_string(),
                current_value: "true".to_string(),
                recommended_value: "false".to_string(),
                description: "Synchronous chunk writes block the main server thread waiting for disk flush. Disabling is safe on modern SSD/NVMe drives.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Asynchronous Chunk Writes".to_string(),
                file: "server.properties".to_string(),
                parameter: "sync-chunk-writes".to_string(),
                current_value: "false".to_string(),
                recommended_value: "false".to_string(),
                description: "Non-blocking chunk writes prevent disk I/O lag spikes.".to_string(),
            });
        }
    }

    // network-compression-threshold
    if let Some(val) = props
        .get("network-compression-threshold")
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|&v| (1..256).contains(&v))
    {
        findings.push(DoctorFinding {
            severity: RuleSeverity::Tip,
            title: "Aggressive Packet Compression".to_string(),
            file: "server.properties".to_string(),
            parameter: "network-compression-threshold".to_string(),
            current_value: val.to_string(),
            recommended_value: "256 or 512".to_string(),
            description: "Compressing small network packets wastes CPU cycles. Values 256 - 512 strike the ideal balance.".to_string(),
        });
    }
}

fn audit_paper_config(content: &str, filename: &str, findings: &mut Vec<DoctorFinding>) {
    let lower = content.to_ascii_lowercase();

    // hopper.disable-move-event
    if lower.contains("disable-move-event") {
        if lower.contains("disable-move-event: true") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Hopper Move Event Disabled".to_string(),
                file: filename.to_string(),
                parameter: "hopper.disable-move-event".to_string(),
                current_value: "true".to_string(),
                recommended_value: "true".to_string(),
                description: "InventoryMoveItemEvent overhead on hoppers is disabled.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Warn,
                title: "Hopper Move Event Enabled".to_string(),
                file: filename.to_string(),
                parameter: "hopper.disable-move-event".to_string(),
                current_value: "false".to_string(),
                recommended_value: "true".to_string(),
                description: "Hoppers fire InventoryMoveItemEvent every tick. Setting to true dramatically reduces MSPT on hopper-heavy bases.".to_string(),
            });
        }
    }

    // redstone-implementation
    if lower.contains("redstone-implementation") {
        if lower.contains("alternate-current") || lower.contains("eigen-redstone") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Optimized Redstone Engine".to_string(),
                file: filename.to_string(),
                parameter: "redstone-implementation".to_string(),
                current_value: "alternate-current".to_string(),
                recommended_value: "alternate-current".to_string(),
                description: "Modern redstone implementation reduces recursive block updates."
                    .to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Tip,
                title: "Vanilla Redstone Implementation".to_string(),
                file: filename.to_string(),
                parameter: "redstone-implementation".to_string(),
                current_value: "vanilla".to_string(),
                recommended_value: "alternate-current".to_string(),
                description: "Setting redstone-implementation to 'alternate-current' eliminates redstone lag cascades.".to_string(),
            });
        }
    }

    // alt-item-despawn-rate
    if lower.contains("alt-item-despawn-rate") {
        if lower.contains("enabled: true") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Alternative Item Despawn Rate".to_string(),
                file: filename.to_string(),
                parameter: "alt-item-despawn-rate.enabled".to_string(),
                current_value: "true".to_string(),
                recommended_value: "true".to_string(),
                description: "Junk items despawn faster, saving entity tick time.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Tip,
                title: "Junk Items Despawn at Normal Speed".to_string(),
                file: filename.to_string(),
                parameter: "alt-item-despawn-rate.enabled".to_string(),
                current_value: "false".to_string(),
                recommended_value: "true".to_string(),
                description: "Enable alt-item-despawn-rate to despawn cobble/arrows faster and prevent mob farm lag.".to_string(),
            });
        }
    }
}

fn audit_purpur_config(content: &str, findings: &mut Vec<DoctorFinding>) {
    let lower = content.to_ascii_lowercase();

    // villager.brain-ticks
    if lower.contains("brain-ticks") {
        if lower.contains("brain-ticks: 1") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Tip,
                title: "Frequent Villager Brain Ticking".to_string(),
                file: "purpur.yml".to_string(),
                parameter: "mobs.villager.brain-ticks".to_string(),
                current_value: "1".to_string(),
                recommended_value: "2 or 4".to_string(),
                description: "Villager AI pathfinding is calculated every single tick. Setting to 2 or 4 cuts villager CPU usage in half.".to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Optimized Villager Brain Ticks".to_string(),
                file: "purpur.yml".to_string(),
                parameter: "mobs.villager.brain-ticks".to_string(),
                current_value: "optimized".to_string(),
                recommended_value: "2 or 4".to_string(),
                description: "Villager AI ticking is spaced across multiple ticks.".to_string(),
            });
        }
    }

    // villager.lobotomize
    if lower.contains("lobotomize") {
        if lower.contains("enabled: true") {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Pass,
                title: "Villager Lobotomization Enabled".to_string(),
                file: "purpur.yml".to_string(),
                parameter: "mobs.villager.lobotomize.enabled".to_string(),
                current_value: "true".to_string(),
                recommended_value: "true".to_string(),
                description: "Trapped trading hall villagers are put into low-CPU idle state."
                    .to_string(),
            });
        } else {
            findings.push(DoctorFinding {
                severity: RuleSeverity::Tip,
                title: "Villager Lobotomization Disabled".to_string(),
                file: "purpur.yml".to_string(),
                parameter: "mobs.villager.lobotomize.enabled".to_string(),
                current_value: "false".to_string(),
                recommended_value: "true".to_string(),
                description: "Enable villager lobotomization to freeze AI for boxed-in trading hall villagers.".to_string(),
            });
        }
    }
}

pub fn print_doctor_report(report: &DoctorReport) {
    println!("\n  \x1b[1;37mChunkLens Doctor\x1b[0m \x1b[90m— Server Configuration Audit\x1b[0m");
    println!(
        "  \x1b[38;2;71;85;105m────────────────────────────────────────────────────────────────────────────\x1b[0m"
    );
    println!(
        "  \x1b[90mTarget Server:\x1b[0m \x1b[1m{}\x1b[0m (software: \x1b[36m{}\x1b[0m)",
        report.server_path.display(),
        report.software.as_deref().unwrap_or("unknown")
    );
    println!(
        "  \x1b[38;2;71;85;105m────────────────────────────────────────────────────────────────────────────\x1b[0m\n"
    );

    for finding in &report.findings {
        let (badge, color) = match finding.severity {
            RuleSeverity::Pass => (" PASS ", "\x1b[1;30;42m"),
            RuleSeverity::Tip => (" TIP  ", "\x1b[1;30;46m"),
            RuleSeverity::Warn => (" WARN ", "\x1b[1;30;43m"),
        };

        println!(
            "  {color}{badge}\x1b[0m \x1b[1m{}\x1b[0m \x1b[90m({}:{})\x1b[0m",
            finding.title, finding.file, finding.parameter
        );
        if finding.severity != RuleSeverity::Pass {
            println!(
                "         \x1b[90mCurrent: \x1b[0m\x1b[31m{}\x1b[0m  \x1b[90m➔  Recommended: \x1b[0m\x1b[32m{}\x1b[0m",
                finding.current_value, finding.recommended_value
            );
        }
        println!(
            "         \x1b[38;2;148;163;184m{}\x1b[0m\n",
            finding.description
        );
    }

    println!(
        "  \x1b[38;2;71;85;105m────────────────────────────────────────────────────────────────────────────\x1b[0m"
    );
    println!(
        "  \x1b[1mSummary:\x1b[0m \x1b[32m✔ {} passed\x1b[0m \x1b[90m•\x1b[0m \x1b[33m⚠ {} warnings\x1b[0m \x1b[90m•\x1b[0m \x1b[36mℹ {} tips\x1b[0m\n",
        report.pass_count(),
        report.warn_count(),
        report.tip_count()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audits_server_properties_flags() {
        let mut props = HashMap::new();
        props.insert("simulation-distance".to_string(), "12".to_string());
        props.insert("view-distance".to_string(), "16".to_string());
        props.insert("sync-chunk-writes".to_string(), "true".to_string());
        let mut findings = Vec::new();
        audit_server_properties(&props, &mut findings);

        assert_eq!(findings.len(), 3);
        assert!(
            findings
                .iter()
                .any(|f| f.parameter == "simulation-distance" && f.severity == RuleSeverity::Warn)
        );
        assert!(
            findings
                .iter()
                .any(|f| f.parameter == "view-distance" && f.severity == RuleSeverity::Warn)
        );
        assert!(
            findings
                .iter()
                .any(|f| f.parameter == "sync-chunk-writes" && f.severity == RuleSeverity::Tip)
        );
    }

    #[test]
    fn audits_paper_and_purpur_optimizations() {
        let paper = r#"
        hopper:
          disable-move-event: false
        redstone-implementation: alternate-current
        "#;
        let mut findings = Vec::new();
        audit_paper_config(paper, "paper.yml", &mut findings);
        assert!(findings.iter().any(
            |f| f.parameter == "hopper.disable-move-event" && f.severity == RuleSeverity::Warn
        ));
        assert!(
            findings
                .iter()
                .any(|f| f.parameter == "redstone-implementation"
                    && f.severity == RuleSeverity::Pass)
        );

        let purpur = r#"
        mobs:
          villager:
            brain-ticks: 1
            lobotomize:
              enabled: true
        "#;
        let mut purpur_findings = Vec::new();
        audit_purpur_config(purpur, &mut purpur_findings);
        assert!(
            purpur_findings
                .iter()
                .any(|f| f.parameter == "mobs.villager.brain-ticks"
                    && f.severity == RuleSeverity::Tip)
        );
        assert!(
            purpur_findings
                .iter()
                .any(|f| f.parameter == "mobs.villager.lobotomize.enabled"
                    && f.severity == RuleSeverity::Pass)
        );
    }
}
