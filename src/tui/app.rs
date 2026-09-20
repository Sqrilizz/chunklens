use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant, SystemTime},
};

use base64::Engine;
use ratatui::widgets::{ListState, TableState};

use crate::{
    model::{ChunkMetrics, Cluster, ScanResult},
    protocol::{slp::ServerPingResponse, spark::SparkProfileSummary},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Monitor = 0,
    Hotspots = 1,
    Heatmap = 2,
    Clusters = 3,
    Diagnostic = 4,
    Spark = 5,
    Rcon = 6,
}

impl Tab {
    pub fn titles() -> [&'static str; 7] {
        [
            " 1 Monitor ",
            " 2 Chunks ",
            " 3 Heatmap ",
            " 4 Clusters ",
            " 5 Inspector ",
            " 6 Spark ",
            " 7 RCON ",
        ]
    }

    pub fn from_index(index: usize) -> Self {
        match index % 7 {
            0 => Tab::Monitor,
            1 => Tab::Hotspots,
            2 => Tab::Heatmap,
            3 => Tab::Clusters,
            4 => Tab::Diagnostic,
            5 => Tab::Spark,
            6 => Tab::Rcon,
            _ => Tab::Monitor,
        }
    }

    pub fn to_index(self) -> usize {
        self as usize
    }

    pub fn next(self) -> Self {
        Self::from_index(self.to_index() + 1)
    }

    pub fn prev(self) -> Self {
        Self::from_index((self.to_index() + 6) % 7)
    }
}

enum BackgroundEvent {
    Ping(Result<(SocketAddr, ServerPingResponse), String>),
    Rcon(Result<String, String>),
}

pub struct AppState {
    pub spatial_index: HashMap<(i32, i32), usize>,
    pub critical_chunks: usize,
    pub table_scroll: usize,
    pub page_size: usize,
    pub help_visible: bool,
    pub search_before_edit: String,
    pub poll_pending: bool,
    pub monitor_error: Option<String>,
    pub rcon_pending: bool,
    pub rcon_history_cursor: Option<usize>,
    background_tx: Sender<BackgroundEvent>,
    background_rx: Receiver<BackgroundEvent>,
    pub scan: ScanResult,
    pub active_tab: Tab,
    pub table_state: TableState,
    pub cluster_state: ListState,
    pub search_query: String,
    pub is_searching: bool,
    pub selected_chunk_index: usize,
    pub notification: Option<(String, Instant)>,
    pub should_quit: bool,
    // Heatmap navigation
    pub heatmap_offset_x: f64,
    pub heatmap_offset_z: f64,
    pub heatmap_scale: f64,
    // Filtered chunk indices
    pub filtered_chunk_indices: Vec<usize>,
    // Live Server Monitor
    pub server_host: String,
    pub server_addr: Option<SocketAddr>,
    pub latest_ping: Option<ServerPingResponse>,
    pub latency_history: Vec<u64>,
    pub monitor_events: Vec<(String, String)>,
    pub last_poll: Instant,
    pub poll_interval: Duration,
    // Spark Profiler
    pub spark_summary: Option<SparkProfileSummary>,
    // RCON Console
    pub rcon_addr: Option<SocketAddr>,
    pub rcon_password: Option<String>,
    pub rcon_input: String,
    pub rcon_history: Vec<String>,
    pub rcon_logs: Vec<(String, String, bool)>, // (time, text, is_command)
}

impl AppState {
    pub fn new(
        mut scan: ScanResult,
        server_host: Option<String>,
        spark_summary: Option<SparkProfileSummary>,
        rcon_addr: Option<SocketAddr>,
        rcon_password: Option<String>,
    ) -> Self {
        scan.chunks.sort_by(|a, b| b.score.total_cmp(&a.score));
        scan.clusters
            .sort_by(|a, b| b.total_score.total_cmp(&a.total_score));

        let chunk_count = scan.chunks.len();
        let mut table_state = TableState::default();
        if chunk_count > 0 {
            table_state.select(Some(0));
        }

        let mut cluster_state = ListState::default();
        if !scan.clusters.is_empty() {
            cluster_state.select(Some(0));
        }

        let filtered_chunk_indices: Vec<usize> = (0..chunk_count).collect();

        let (center_x, center_z) = if let Some(c) = scan.clusters.first() {
            (c.center.0 as f64, c.center.1 as f64)
        } else if let Some(first) = scan.chunks.first() {
            (first.chunk_x as f64, first.chunk_z as f64)
        } else {
            (0.0, 0.0)
        };

        // Resolve socket address for server monitor
        let (background_tx, background_rx) = mpsc::channel();
        let spatial_index = scan
            .chunks
            .iter()
            .enumerate()
            .map(|(i, c)| ((c.chunk_x, c.chunk_z), i))
            .collect();
        let critical_chunks = scan.chunks.iter().filter(|c| c.score >= 80.0).count();

        let initial_tab = if chunk_count > 0 {
            Tab::Hotspots
        } else {
            Tab::Monitor
        };

        Self {
            spatial_index,
            critical_chunks,
            table_scroll: 0,
            page_size: 20,
            help_visible: false,
            search_before_edit: String::new(),
            poll_pending: false,
            monitor_error: None,
            rcon_pending: false,
            rcon_history_cursor: None,
            background_tx,
            background_rx,
            scan,
            active_tab: initial_tab,
            table_state,
            cluster_state,
            search_query: String::new(),
            is_searching: false,
            selected_chunk_index: 0,
            notification: None,
            should_quit: false,
            heatmap_offset_x: center_x,
            heatmap_offset_z: center_z,
            heatmap_scale: 1.0,
            filtered_chunk_indices,
            server_host: server_host.unwrap_or_default(),
            server_addr: None,
            latest_ping: None,
            latency_history: Vec::new(),
            monitor_events: Vec::new(),
            last_poll: Instant::now() - Duration::from_secs(10), // trigger immediate poll
            poll_interval: Duration::from_secs(3),
            spark_summary,
            rcon_addr,
            rcon_password,
            rcon_input: String::new(),
            rcon_history: Vec::new(),
            rcon_logs: Vec::new(),
        }
    }

    pub fn poll_server(&mut self) {
        if self.server_host.is_empty() || self.poll_pending {
            return;
        }
        self.last_poll = Instant::now();
        self.poll_pending = true;
        let address = self.server_host.clone();
        let tx = self.background_tx.clone();
        std::thread::spawn(move || {
            let result = crate::protocol::slp::resolve_server(&address, 25565, true)
                .and_then(|(host, addr)| {
                    crate::protocol::slp::ping_server(&host, addr, Duration::from_secs(2))
                        .map(|ping| (addr, ping))
                })
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(BackgroundEvent::Ping(result));
        });
    }

    pub fn process_background(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.background_rx.try_recv() {
            changed = true;
            match event {
                BackgroundEvent::Ping(result) => {
                    self.poll_pending = false;
                    match result {
                        Ok((addr, ping)) => {
                            if self.monitor_error.take().is_some() || self.latest_ping.is_none() {
                                self.add_monitor_event("Server connected".into());
                            }
                            if let Some(previous) = &self.latest_ping
                                && previous.online_players != ping.online_players
                            {
                                self.add_monitor_event(format!(
                                    "Players: {} → {}",
                                    previous.online_players, ping.online_players
                                ));
                            }
                            self.server_addr = Some(addr);
                            self.latency_history.push(ping.latency_ms);
                            if self.latency_history.len() > 60 {
                                self.latency_history.remove(0);
                            }
                            self.latest_ping = Some(ping);
                        }
                        Err(error) => {
                            if self.monitor_error.as_ref() != Some(&error) {
                                self.add_monitor_event(format!("Connection failed: {error}"));
                            }
                            self.monitor_error = Some(error);
                        }
                    }
                }
                BackgroundEvent::Rcon(result) => {
                    self.rcon_pending = false;
                    let response = result.unwrap_or_else(|error| format!("RCON failed: {error}"));
                    for line in response.lines() {
                        self.rcon_logs
                            .push((current_time_str(), line.to_owned(), false));
                    }
                    if self.rcon_logs.len() > 200 {
                        self.rcon_logs.drain(..self.rcon_logs.len() - 200);
                    }
                }
            }
        }
        changed
    }

    pub fn add_monitor_event(&mut self, text: String) {
        let time_str = current_time_str();
        self.monitor_events.push((time_str, text));
        if self.monitor_events.len() > 100 {
            self.monitor_events.remove(0);
        }
    }

    pub fn send_rcon_command(&mut self) {
        let command = self.rcon_input.trim().to_string();
        if command.is_empty() {
            return;
        }
        if self.rcon_pending {
            self.notify("Waiting for the current command".into());
            return;
        }
        let Some(password) = self.rcon_password.clone().filter(|p| !p.is_empty()) else {
            self.notify("Set CHUNKLENS_RCON_PASSWORD before opening the dashboard".into());
            return;
        };
        let Some(addr) = self.rcon_addr else {
            self.notify("Set --rcon-address when opening the dashboard".into());
            return;
        };
        self.rcon_input.clear();
        self.rcon_history.push(command.clone());
        if self.rcon_history.len() > 100 {
            self.rcon_history.remove(0);
        }
        self.rcon_history_cursor = None;
        self.rcon_logs
            .push((current_time_str(), format!("> {command}"), true));
        self.rcon_pending = true;
        let tx = self.background_tx.clone();
        std::thread::spawn(move || {
            let result =
                crate::protocol::rcon::RconClient::connect(addr, &password, Duration::from_secs(3))
                    .and_then(|mut client| client.execute(&command))
                    .map_err(|e| format!("{e:#}"));
            let _ = tx.send(BackgroundEvent::Rcon(result));
        });
    }

    pub fn recall_rcon(&mut self, previous: bool) {
        if self.rcon_history.is_empty() {
            return;
        }
        let next = match (self.rcon_history_cursor, previous) {
            (None, true) => Some(self.rcon_history.len() - 1),
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.rcon_history.len() => Some(i + 1),
            _ => None,
        };
        self.rcon_history_cursor = next;
        self.rcon_input = next.map_or(String::new(), |i| self.rcon_history[i].clone());
    }

    pub fn refresh_filter(&mut self) {
        self.table_scroll = 0;
        let q = self.search_query.trim().to_lowercase().replace(", ", ",");
        if q.is_empty() {
            self.filtered_chunk_indices = (0..self.scan.chunks.len()).collect();
        } else {
            self.filtered_chunk_indices = self
                .scan
                .chunks
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    let dim = c.dimension.to_lowercase();
                    let coords = format!("{},{}", c.chunk_x, c.chunk_z);
                    let block_coords = format!("{},{}", c.block_x(), c.block_z());
                    let score_str = format!("{:.0}", c.score);
                    dim.contains(&q)
                        || coords.contains(&q)
                        || block_coords.contains(&q)
                        || score_str.contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }

        if self.filtered_chunk_indices.is_empty() {
            self.table_state.select(None);
            self.selected_chunk_index = 0;
        } else {
            let cur = self.table_state.selected().unwrap_or(0);
            let next = cur.min(self.filtered_chunk_indices.len() - 1);
            self.table_state.select(Some(next));
            self.selected_chunk_index = self.filtered_chunk_indices[next];
        }
    }

    pub fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    pub fn prev_tab(&mut self) {
        self.active_tab = self.active_tab.prev();
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.active_tab = tab;
    }

    pub fn selected_chunk(&self) -> Option<&ChunkMetrics> {
        if self.filtered_chunk_indices.is_empty() {
            return None;
        }
        let index = self
            .selected_chunk_index
            .min(self.scan.chunks.len().saturating_sub(1));
        self.scan.chunks.get(index)
    }

    pub fn selected_cluster(&self) -> Option<&Cluster> {
        let idx = self.cluster_state.selected().unwrap_or(0);
        self.scan.clusters.get(idx)
    }

    pub fn next_row(&mut self) {
        if self.filtered_chunk_indices.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.filtered_chunk_indices.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
        self.selected_chunk_index = self.filtered_chunk_indices[i];
    }

    pub fn prev_row(&mut self) {
        if self.filtered_chunk_indices.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_chunk_indices.len().saturating_sub(1)
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
        self.selected_chunk_index = self.filtered_chunk_indices[i];
    }

    pub fn page_down(&mut self) {
        if self.filtered_chunk_indices.is_empty() {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0);
        let next = (cur + self.page_size).min(self.filtered_chunk_indices.len().saturating_sub(1));
        self.table_state.select(Some(next));
        self.selected_chunk_index = self.filtered_chunk_indices[next];
    }

    pub fn page_up(&mut self) {
        if self.filtered_chunk_indices.is_empty() {
            return;
        }
        let cur = self.table_state.selected().unwrap_or(0);
        let prev = cur.saturating_sub(self.page_size);
        self.table_state.select(Some(prev));
        self.selected_chunk_index = self.filtered_chunk_indices[prev];
    }

    pub fn next_cluster(&mut self) {
        if self.scan.clusters.is_empty() {
            return;
        }
        let i = match self.cluster_state.selected() {
            Some(i) => {
                if i >= self.scan.clusters.len().saturating_sub(1) {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.cluster_state.select(Some(i));
    }

    pub fn prev_cluster(&mut self) {
        if self.scan.clusters.is_empty() {
            return;
        }
        let i = match self.cluster_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.scan.clusters.len().saturating_sub(1)
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.cluster_state.select(Some(i));
    }

    pub fn pan_heatmap(&mut self, dx: f64, dz: f64) {
        self.heatmap_offset_x += dx * self.heatmap_scale;
        self.heatmap_offset_z += dz * self.heatmap_scale;
    }

    pub fn zoom_heatmap(&mut self, factor: f64) {
        self.heatmap_scale = (self.heatmap_scale * factor).clamp(0.1, 50.0);
    }

    pub fn copy_teleport_command(&mut self) {
        let command = if self.active_tab == Tab::Clusters {
            self.selected_cluster().map(|c| {
                format!(
                    "/tp @s {} ~ {}",
                    i64::from(c.center.0) * 16 + 8,
                    i64::from(c.center.1) * 16 + 8
                )
            })
        } else {
            self.selected_chunk().map(ChunkMetrics::tp_command)
        };
        if let Some(command) = command {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&command);
            print!("\x1b]52;c;{encoded}\x07");
            use std::io::Write;
            let _ = std::io::stdout().flush();
            self.notify(format!("Clipboard: {command}"));
        }
    }

    pub fn notify(&mut self, msg: String) {
        self.notification = Some((msg, Instant::now()));
    }

    pub fn clear_expired_notification(&mut self) {
        if let Some((_, created_at)) = self.notification
            && created_at.elapsed() > Duration::from_secs(3)
        {
            self.notification = None;
        }
    }
}

fn current_time_str() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs = now % 60;
    let mins = (now / 60) % 60;
    let hours = (now / 3600) % 24;
    format!("{:02}:{:02}:{:02}", hours, mins, secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_dashboard_does_not_start_network_requests() {
        let mut app = AppState::new(ScanResult::default(), None, None, None, None);
        app.poll_server();
        assert!(!app.poll_pending);
        assert!(app.server_addr.is_none());
    }

    #[test]
    fn failed_ping_is_not_presented_as_a_fresh_connection() {
        let mut app = AppState::new(ScanResult::default(), None, None, None, None);
        app.background_tx
            .send(BackgroundEvent::Ping(Err("offline".into())))
            .unwrap();
        assert!(app.process_background());
        assert_eq!(app.monitor_error.as_deref(), Some("offline"));
        assert!(!app.poll_pending);
    }

    #[test]
    fn filters_and_pages_without_losing_selection() {
        let scan = ScanResult {
            chunks: (0..100)
                .map(|i| ChunkMetrics {
                    chunk_x: i,
                    dimension: "world".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut app = AppState::new(scan, None, None, None, None);
        app.page_size = 7;
        app.page_down();
        assert_eq!(app.selected_chunk().unwrap().chunk_x, 7);
        app.search_query = "99,0".into();
        app.refresh_filter();
        assert_eq!(app.selected_chunk().unwrap().chunk_x, 99);
        app.search_query = "nothing".into();
        app.refresh_filter();
        assert!(app.selected_chunk().is_none());
    }
}
