use super::{
    app::{AppState, Tab},
    ui,
};
use crate::model::{ChunkMetrics, ScanResult};
use ratatui::{Terminal, backend::TestBackend};
use std::{path::PathBuf, time::Instant};

#[test]
#[ignore]
fn benchmark_dashboard() {
    let count = std::env::var("CHUNKLENS_BENCH_CHUNKS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000);
    let scan = ScanResult {
        chunks: (0..count)
            .map(|i| ChunkMetrics {
                dimension: "world".into(),
                chunk_x: i % 1000,
                chunk_z: i / 1000,
                score: (i % 100) as f64,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    let mut state = AppState::new(scan, None, None, None, None);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for tab in [Tab::Hotspots, Tab::Heatmap] {
        state.active_tab = tab;
        terminal
            .draw(|frame| ui::render(frame, &mut state))
            .unwrap();
        let mut times = Vec::new();
        for _ in 0..15 {
            let started = Instant::now();
            terminal
                .draw(|frame| ui::render(frame, &mut state))
                .unwrap();
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "RENDER_BENCH tab={tab:?} chunks={count} median_ms={:.4}",
            times[times.len() / 2]
        );
    }
}

#[test]
#[ignore]
fn snapshot_dashboard() {
    let report = PathBuf::from(
        std::env::var("CHUNKLENS_SNAPSHOT_REPORT").expect("set CHUNKLENS_SNAPSHOT_REPORT"),
    );
    let output =
        PathBuf::from(std::env::var("CHUNKLENS_SNAPSHOT_DIR").expect("set CHUNKLENS_SNAPSHOT_DIR"));
    std::fs::create_dir_all(&output).unwrap();
    let scan = ScanResult::read_json(&report).unwrap();
    let mut state = AppState::new(scan, None, None, None, None);
    for (width, height) in [(60, 18), (80, 24), (120, 40), (180, 50)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        for tab in [Tab::Hotspots, Tab::Heatmap, Tab::Diagnostic, Tab::Monitor] {
            state.active_tab = tab;
            terminal
                .draw(|frame| ui::render(frame, &mut state))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = buffer
                .content
                .chunks(width as usize)
                .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            let cells=buffer.content.iter().map(|cell|serde_json::json!({"text":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg)})).collect::<Vec<_>>();
            std::fs::write(output.join(format!("{tab:?}-{width}.txt")), text).unwrap();
            std::fs::write(
                output.join(format!("{tab:?}-{width}.json")),
                serde_json::to_vec(
                    &serde_json::json!({"width":width,"height":height,"cells":cells}),
                )
                .unwrap(),
            )
            .unwrap();
        }
    }
}
