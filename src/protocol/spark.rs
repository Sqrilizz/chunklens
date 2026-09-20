use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, ensure};
use prost::Message;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SparkProfileSummary {
    pub server_version: String,
    pub number_of_ticks: u64,
    pub duration_seconds: f64,
    pub average_mspt: Option<f64>,
    pub top_consumers: Vec<SparkMethodMetric>,
    pub category_breakdown: SparkCategoryBreakdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SparkMethodMetric {
    pub class_name: String,
    pub method_name: String,
    pub percentage: f64,
    pub self_time_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SparkCategoryBreakdown {
    pub entities_pct: f64,
    pub tile_entities_pct: f64,
    pub redstone_pct: f64,
    pub chunk_loading_pct: f64,
    pub plugins_pct: f64,
    pub physics_pct: f64,
    pub other_pct: f64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct RawReport {
    metadata: Metadata,
    threads: Vec<Thread>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Metadata {
    #[serde(alias = "platformMetadata")]
    platform_metadata: Platform,
    #[serde(alias = "platformStatistics")]
    platform_statistics: Statistics,
    #[serde(alias = "numberOfTicks")]
    number_of_ticks: u64,
    #[serde(alias = "startTime")]
    start_time: Option<i64>,
    #[serde(alias = "endTime")]
    end_time: Option<i64>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Platform {
    name: String,
    version: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Statistics {
    mspt: Mspt,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Mspt {
    last1m: Average,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Average {
    mean: Option<f64>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Thread {
    name: String,
    children: Vec<Node>,
    #[serde(alias = "childrenRefs")]
    children_refs: Vec<i32>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Node {
    #[serde(alias = "className")]
    class_name: String,
    #[serde(alias = "methodName")]
    method_name: String,
    time: Option<f64>,
    times: Vec<f64>,
    children: Vec<Node>,
    #[serde(alias = "childrenRefs")]
    children_refs: Vec<i32>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoReport {
    #[prost(message, optional, tag = "1")]
    metadata: Option<ProtoMetadata>,
    #[prost(message, repeated, tag = "2")]
    threads: Vec<ProtoThread>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoMetadata {
    #[prost(int64, tag = "2")]
    start_time: i64,
    #[prost(message, optional, tag = "7")]
    platform: Option<ProtoPlatform>,
    #[prost(message, optional, tag = "8")]
    statistics: Option<ProtoStatistics>,
    #[prost(int64, tag = "11")]
    end_time: i64,
    #[prost(int32, tag = "12")]
    number_of_ticks: i32,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoPlatform {
    #[prost(string, tag = "2")]
    name: String,
    #[prost(string, tag = "3")]
    version: String,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoStatistics {
    #[prost(message, optional, tag = "5")]
    mspt: Option<ProtoMspt>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoMspt {
    #[prost(message, optional, tag = "1")]
    last_1m: Option<ProtoAverage>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoAverage {
    #[prost(double, tag = "1")]
    mean: f64,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoThread {
    #[prost(string, tag = "1")]
    name: String,
    #[prost(message, repeated, tag = "3")]
    children: Vec<ProtoNode>,
    #[prost(double, repeated, tag = "4")]
    times: Vec<f64>,
    #[prost(int32, repeated, tag = "5")]
    children_refs: Vec<i32>,
}

#[derive(Clone, PartialEq, Message)]
struct ProtoNode {
    #[prost(string, tag = "3")]
    class_name: String,
    #[prost(string, tag = "4")]
    method_name: String,
    #[prost(double, repeated, tag = "8")]
    times: Vec<f64>,
    #[prost(int32, repeated, tag = "9")]
    children_refs: Vec<i32>,
}

impl Node {
    fn weight(&self) -> Result<f64> {
        ensure!(
            self.children_refs.is_empty(),
            "indexed Spark stacks are not supported; export a nested JSON call tree"
        );
        let weight = self.time.unwrap_or_else(|| self.times.iter().sum());
        ensure!(
            weight.is_finite()
                && weight >= 0.0
                && self.times.iter().all(|v| v.is_finite() && *v >= 0.0),
            "invalid Spark sample weight"
        );
        Ok(weight)
    }
}

pub fn parse_spark_report(path: &Path) -> Result<SparkProfileSummary> {
    let input =
        std::fs::read(path).with_context(|| format!("opening Spark report {}", path.display()))?;
    ensure!(
        input.len() <= 128 * 1024 * 1024,
        "Spark report exceeds 128 MiB limit"
    );
    if input
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'{')
    {
        let report: RawReport =
            serde_json::from_slice(&input).context("expected a nested Spark JSON report")?;
        summarize_json(report)
    } else {
        summarize_protobuf(
            ProtoReport::decode(input.as_slice()).context("invalid Spark protobuf report")?,
        )
    }
}

fn summarize_json(report: RawReport) -> Result<SparkProfileSummary> {
    let mut summary = SparkProfileSummary {
        server_version: format!(
            "{} {}",
            report.metadata.platform_metadata.name, report.metadata.platform_metadata.version
        )
        .trim()
        .to_owned(),
        number_of_ticks: report.metadata.number_of_ticks,
        average_mspt: report.metadata.platform_statistics.mspt.last1m.mean,
        ..Default::default()
    };
    ensure!(
        summary
            .average_mspt
            .is_none_or(|v| v.is_finite() && v >= 0.0),
        "invalid measured MSPT"
    );
    if let (Some(start), Some(end)) = (report.metadata.start_time, report.metadata.end_time) {
        ensure!(end >= start, "Spark report ends before it starts");
        summary.duration_seconds = (end as f64 - start as f64) / 1000.0;
    }
    let mut methods = BTreeMap::<(String, String), f64>::new();
    let mut total = 0.0;
    for thread in report
        .threads
        .iter()
        .filter(|thread| thread.name.eq_ignore_ascii_case("Server thread"))
    {
        ensure!(
            thread.children_refs.is_empty(),
            "indexed Spark stacks are not supported; export a nested JSON call tree"
        );
        for node in &thread.children {
            total += node.weight()?;
            collect_self_time(node, &mut methods)?;
        }
    }
    finish_summary(summary, methods, total)
}

fn summarize_protobuf(report: ProtoReport) -> Result<SparkProfileSummary> {
    let metadata = report
        .metadata
        .context("Spark protobuf is missing metadata")?;
    let platform = metadata.platform.unwrap_or_default();
    let average_mspt = metadata
        .statistics
        .and_then(|statistics| statistics.mspt)
        .and_then(|mspt| mspt.last_1m)
        .map(|average| average.mean);
    ensure!(
        average_mspt.is_none_or(|value| value.is_finite() && value >= 0.0),
        "invalid measured MSPT"
    );
    ensure!(
        metadata.end_time >= metadata.start_time,
        "Spark report ends before it starts"
    );
    let summary = SparkProfileSummary {
        server_version: format!("{} {}", platform.name, platform.version)
            .trim()
            .to_owned(),
        number_of_ticks: metadata.number_of_ticks.max(0) as u64,
        duration_seconds: (metadata.end_time - metadata.start_time) as f64 / 1000.0,
        average_mspt,
        ..Default::default()
    };
    let mut methods = BTreeMap::new();
    let mut total = 0.0;
    for thread in report
        .threads
        .iter()
        .filter(|thread| thread.name.eq_ignore_ascii_case("Server thread"))
    {
        let roots = if thread.children_refs.is_empty() {
            (0..thread.children.len() as i32).collect::<Vec<_>>()
        } else {
            thread.children_refs.clone()
        };
        for index in roots {
            let index = usize::try_from(index).context("negative Spark node reference")?;
            let root = thread
                .children
                .get(index)
                .context("Spark node reference outside call tree")?;
            total += proto_weight(&root.times)?;
            collect_proto_self_time(&thread.children, index, &mut methods, 0)?;
        }
    }
    finish_summary(summary, methods, total)
}

fn proto_weight(times: &[f64]) -> Result<f64> {
    ensure!(
        times.iter().all(|time| time.is_finite() && *time >= 0.0),
        "invalid Spark sample weight"
    );
    Ok(times.iter().copied().fold(0.0, f64::max))
}

fn collect_proto_self_time(
    nodes: &[ProtoNode],
    index: usize,
    methods: &mut BTreeMap<(String, String), f64>,
    depth: usize,
) -> Result<()> {
    ensure!(depth < 1024, "Spark call tree exceeds depth limit");
    let node = nodes
        .get(index)
        .context("Spark node reference outside call tree")?;
    let inclusive = proto_weight(&node.times)?;
    let mut children = 0.0;
    for reference in &node.children_refs {
        let child = usize::try_from(*reference).context("negative Spark node reference")?;
        ensure!(
            child < index,
            "Spark call tree contains an invalid or cyclic reference"
        );
        children += proto_weight(&nodes[child].times)?;
        collect_proto_self_time(nodes, child, methods, depth + 1)?;
    }
    ensure!(
        children <= inclusive + inclusive.max(1.0) * 1e-6,
        "Spark child samples exceed their parent"
    );
    let exclusive = (inclusive - children).max(0.0);
    if exclusive > 0.0 {
        let class = if node.class_name.is_empty() {
            "(unknown)"
        } else {
            &node.class_name
        };
        *methods
            .entry((class.to_owned(), node.method_name.clone()))
            .or_default() += exclusive;
    }
    Ok(())
}

fn finish_summary(
    mut summary: SparkProfileSummary,
    methods: BTreeMap<(String, String), f64>,
    total: f64,
) -> Result<SparkProfileSummary> {
    ensure!(
        total.is_finite() && total > 0.0,
        "no supported Server thread samples found in the Spark report"
    );
    let mut consumers = Vec::new();
    for ((class_name, method_name), self_time_ms) in methods {
        let percentage = self_time_ms / total * 100.0;
        let name = format!("{class_name}.{method_name}").to_lowercase();
        let category = if name.contains("blockentity")
            || name.contains("block.entity")
            || name.contains("tileentity")
            || name.contains("hopper")
        {
            &mut summary.category_breakdown.tile_entities_pct
        } else if name.contains("entity") {
            &mut summary.category_breakdown.entities_pct
        } else if name.contains("redstone") || name.contains("piston") || name.contains("wire") {
            &mut summary.category_breakdown.redstone_pct
        } else if name.contains("chunk") || name.contains("region") {
            &mut summary.category_breakdown.chunk_loading_pct
        } else {
            &mut summary.category_breakdown.other_pct
        };
        *category += percentage;
        consumers.push(SparkMethodMetric {
            class_name,
            method_name,
            percentage,
            self_time_ms,
        });
    }
    consumers.sort_by(|a, b| {
        b.percentage
            .total_cmp(&a.percentage)
            .then_with(|| a.class_name.cmp(&b.class_name))
            .then_with(|| a.method_name.cmp(&b.method_name))
    });
    consumers.truncate(8);
    summary.top_consumers = consumers;
    Ok(summary)
}

fn collect_self_time(node: &Node, methods: &mut BTreeMap<(String, String), f64>) -> Result<()> {
    let inclusive = node.weight()?;
    let children = node
        .children
        .iter()
        .map(Node::weight)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .sum::<f64>();
    ensure!(
        children <= inclusive + inclusive.abs().max(1.0) * 1e-6,
        "Spark child samples exceed their parent"
    );
    let exclusive = (inclusive - children).max(0.0);
    if exclusive > 0.0 {
        let class = if node.class_name.is_empty() {
            "(unknown)"
        } else {
            &node.class_name
        };
        *methods
            .entry((class.to_owned(), node.method_name.clone()))
            .or_default() += exclusive;
    }
    for child in &node.children {
        collect_self_time(child, methods)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: serde_json::Value) -> Result<SparkProfileSummary> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), serde_json::to_vec(&value)?)?;
        parse_spark_report(file.path())
    }

    #[test]
    fn uses_exclusive_samples_and_does_not_invent_mspt() {
        let report = parse(serde_json::json!({
            "metadata":{"start_time":0,"end_time":50_000,"number_of_ticks":1000},
            "threads":[{"name":"Server thread","children":[{"class_name":"Entity","method_name":"tick","time":100.0,
                "children":[{"class_name":"HopperBlockEntity","method_name":"tick","time":40.0}]}]}]
        })).unwrap();
        assert_eq!(report.average_mspt, None);
        assert_eq!(report.category_breakdown.entities_pct, 60.0);
        assert_eq!(report.category_breakdown.tile_entities_pct, 40.0);
    }

    #[test]
    fn reads_measured_mspt_and_all_sample_windows() {
        let report = parse(serde_json::json!({
            "metadata":{"platformStatistics":{"mspt":{"last1m":{"mean":12.5}}}},
            "threads":[{"name":"Server thread","children":[{"className":"Entity","methodName":"tick","times":[10.0,20.0]}]}]
        })).unwrap();
        assert_eq!(report.average_mspt, Some(12.5));
        assert_eq!(report.top_consumers[0].self_time_ms, 30.0);
        assert_eq!(report.top_consumers[0].percentage, 100.0);
    }

    #[test]
    fn unsupported_report_is_an_error() {
        assert!(parse(serde_json::json!({})).is_err());
        assert!(
            parse(serde_json::json!({"threads":[{"name":"Server thread","children_refs":[0]}]}))
                .is_err()
        );
    }

    #[test]
    fn reads_indexed_protobuf_call_tree() {
        let report = ProtoReport {
            metadata: Some(ProtoMetadata {
                start_time: 1_000,
                end_time: 3_000,
                platform: Some(ProtoPlatform {
                    name: "Paper".into(),
                    version: "1.21".into(),
                }),
                number_of_ticks: 40,
                ..Default::default()
            }),
            threads: vec![ProtoThread {
                name: "Server thread".into(),
                children: vec![
                    ProtoNode {
                        class_name: "HopperBlockEntity".into(),
                        method_name: "tick".into(),
                        times: vec![30.0],
                        children_refs: vec![],
                    },
                    ProtoNode {
                        class_name: "Entity".into(),
                        method_name: "tick".into(),
                        times: vec![100.0],
                        children_refs: vec![0],
                    },
                ],
                times: vec![100.0],
                children_refs: vec![1],
            }],
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), report.encode_to_vec()).unwrap();
        let result = parse_spark_report(file.path()).unwrap();
        assert_eq!(result.server_version, "Paper 1.21");
        assert_eq!(result.duration_seconds, 2.0);
        assert_eq!(result.category_breakdown.entities_pct, 70.0);
        assert_eq!(result.category_breakdown.tile_entities_pct, 30.0);
    }

    #[test]
    fn rejects_invalid_protobuf_reference() {
        let report = ProtoReport {
            metadata: Some(ProtoMetadata {
                end_time: 1,
                ..Default::default()
            }),
            threads: vec![ProtoThread {
                name: "Server thread".into(),
                children: vec![ProtoNode {
                    times: vec![1.0],
                    children_refs: vec![0],
                    ..Default::default()
                }],
                children_refs: vec![0],
                ..Default::default()
            }],
        };
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), report.encode_to_vec()).unwrap();
        assert!(parse_spark_report(file.path()).is_err());
    }
}
