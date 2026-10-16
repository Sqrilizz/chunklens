use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table,
        Wrap,
    },
};

use super::app::{AppState, Tab};
use crate::model::ChunkMetrics;

// Industrial Terminal Palette (inspired by lazygit & btop)
const COLOR_PRIMARY: Color = Color::Rgb(56, 189, 248); // Cyan 400 (Active focus / highlights)
const COLOR_ACCENT: Color = Color::Rgb(14, 165, 233); // Sky 500
const COLOR_SUCCESS: Color = Color::Rgb(34, 197, 94); // Green 500 (Healthy / Connected)
const COLOR_WARNING: Color = Color::Rgb(245, 158, 11); // Amber 500 (Elevated load)
const COLOR_DANGER: Color = Color::Rgb(239, 68, 68); // Red 500 (Critical load / Errors)
const COLOR_MUTED: Color = Color::Rgb(148, 163, 184); // Slate 400 (Secondary text)
const COLOR_BORDER: Color = Color::Rgb(71, 85, 105); // Slate 600 (Dim borders)
const COLOR_BORDER_ACTIVE: Color = Color::Rgb(56, 189, 248); // Cyan 400 (Focused panel)
const COLOR_SELECTION_BG: Color = Color::Rgb(30, 41, 59); // Slate 800 (Row selection)

pub fn mini_progress_bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio * width as f64).round() as usize).min(width);
    let empty = width.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let size = frame.area();
    if size.width < 60 || size.height < 18 {
        frame.render_widget(
            Paragraph::new(
                "ChunkLens needs a terminal of at least 60 × 18.\nResize the window, or press Q to quit.",
            )
            .wrap(Wrap { trim: true }),
            size,
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(size);

    render_header(frame, state, chunks[0]);
    render_tabs(frame, state, chunks[1]);

    match state.active_tab {
        Tab::Monitor => render_monitor(frame, state, chunks[2]),
        Tab::Hotspots => render_hotspots(frame, state, chunks[2]),
        Tab::Heatmap => render_heatmap(frame, state, chunks[2]),
        Tab::Clusters => render_clusters(frame, state, chunks[2]),
        Tab::Diagnostic => render_diagnostic(frame, state, chunks[2]),
        Tab::Spark => render_spark(frame, state, chunks[2]),
        Tab::Rcon => render_rcon(frame, state, chunks[2]),
    }

    render_footer(frame, state, chunks[3]);

    if state.help_visible {
        render_help(frame, size);
    }

    if state.purge_modal_open {
        render_purge_modal(frame, state, size);
    }
}

fn render_header(frame: &mut Frame, state: &AppState, area: Rect) {
    let world = std::path::Path::new(&state.scan.world)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("No report");

    const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spinner = SPINNER_FRAMES[state.tick_count % SPINNER_FRAMES.len()];

    let (conn_info, conn_color) = if state.server_host.is_empty() {
        ("Offline analysis".to_string(), COLOR_MUTED)
    } else if state.monitor_error.is_some() {
        (
            format!("{} [Disconnected]", state.server_host),
            COLOR_DANGER,
        )
    } else if let Some(ping) = &state.latest_ping {
        let poll_ratio = (state.last_poll.elapsed().as_secs_f64()
            / state.poll_interval.as_secs_f64())
        .clamp(0.0, 1.0);
        let poll_bar = mini_progress_bar(poll_ratio, 6);
        let ping_state = if state.poll_pending {
            format!("[{spinner} pinging]")
        } else {
            format!("[loop: {poll_bar}]")
        };
        (
            format!(
                "{} · {}ms · {}/{} online  {}",
                state.server_host,
                ping.latency_ms,
                ping.online_players,
                ping.max_players,
                ping_state
            ),
            COLOR_SUCCESS,
        )
    } else {
        (
            format!("{spinner} Connecting to {}...", state.server_host),
            COLOR_WARNING,
        )
    };

    let spans = vec![
        Span::styled(
            " CHUNKLENS ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {world} · {} / {} chunks  ",
                state.scan.chunks.len(),
                state.scan.total_chunks()
            ),
            Style::default().fg(Color::White),
        ),
        Span::styled("│ ", Style::default().fg(COLOR_BORDER)),
        Span::styled(conn_info, Style::default().fg(conn_color)),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_tabs(frame: &mut Frame, state: &AppState, area: Rect) {
    let labels = ["1: Chunks", "2: Clusters", "3: Live Monitor", "4: Console"];
    let mut spans = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let is_active = state.active_tab.primary_index() == Some(index);
        spans.push(Span::styled(
            format!(" {label} "),
            if is_active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLOR_MUTED)
            },
        ));
        spans.push(Span::raw(" "));
    }

    if state.active_tab == Tab::Hotspots {
        spans.push(Span::styled(
            "│  Filter: ",
            Style::default().fg(COLOR_BORDER),
        ));
        spans.push(Span::styled(
            format!(
                "[f] {} ({})",
                state.filter_category.label(),
                state.filtered_chunk_indices.len()
            ),
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let advanced = match state.active_tab {
        Tab::Heatmap => Some("Heatmap"),
        Tab::Diagnostic => Some("Inspector"),
        Tab::Spark => Some("Spark"),
        _ => None,
    };
    if let Some(name) = advanced {
        spans.push(Span::styled(
            "  │  View: ",
            Style::default().fg(COLOR_BORDER),
        ));
        spans.push(Span::styled(
            name,
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub struct ChunkDiagnosis {
    pub tag: &'static str,
    pub severity: &'static str,
    pub severity_color: Color,
    pub summary: String,
}

pub fn diagnose_chunk(c: &crate::model::ChunkMetrics) -> ChunkDiagnosis {
    let redstone_total = c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons;
    if c.payload_size > 1_000_000 || c.shulkers > 50 {
        ChunkDiagnosis {
            tag: "[NBT-STASH]",
            severity: "[CRIT]",
            severity_color: COLOR_DANGER,
            summary: format!(
                "Oversized NBT payload ({:.1} MB, {} shulkers) — potential dupe stash or crash hazard",
                c.payload_size as f64 / 1_048_576.0,
                c.shulkers
            ),
        }
    } else if c.villagers >= 30 {
        ChunkDiagnosis {
            tag: "[VILLAGER]",
            severity: "[CRIT]",
            severity_color: COLOR_DANGER,
            summary: format!(
                "Dense villager cluster ({} villagers) — heavy Brain AI pathfinding and POI scan cost",
                c.villagers
            ),
        }
    } else if c.hoppers >= 60 || c.hopper_minecarts >= 10 {
        ChunkDiagnosis {
            tag: "[HOPPERS]",
            severity: if c.hoppers >= 100 { "[CRIT]" } else { "[WARN]" },
            severity_color: if c.hoppers >= 100 {
                COLOR_DANGER
            } else {
                COLOR_WARNING
            },
            summary: format!(
                "High hopper density ({} hoppers, {} minecarts) — continuous inventory polling",
                c.hoppers, c.hopper_minecarts
            ),
        }
    } else if redstone_total >= 80 || c.comparators >= 25 {
        ChunkDiagnosis {
            tag: "[REDSTONE]",
            severity: "[WARN]",
            severity_color: COLOR_WARNING,
            summary: format!(
                "Active redstone clockwork ({} total components, {} comparators)",
                redstone_total, c.comparators
            ),
        }
    } else if c.dropped_items >= 50 || c.exp_orbs >= 40 {
        ChunkDiagnosis {
            tag: "[DROPPED]",
            severity: "[WARN]",
            severity_color: COLOR_WARNING,
            summary: format!(
                "Loose entity accumulation ({} items, {} XP orbs)",
                c.dropped_items, c.exp_orbs
            ),
        }
    } else if c.entity_count >= 50 {
        ChunkDiagnosis {
            tag: "[MOB-FARM]",
            severity: "[WARN]",
            severity_color: COLOR_WARNING,
            summary: format!(
                "Dense mob concentration ({} total entities)",
                c.entity_count
            ),
        }
    } else if c.score >= 50.0 {
        ChunkDiagnosis {
            tag: "[HEAVY]",
            severity: "[WARN]",
            severity_color: COLOR_WARNING,
            summary: format!("High cumulative load score ({:.1} / 100)", c.score),
        }
    } else {
        ChunkDiagnosis {
            tag: "[NORMAL]",
            severity: "[OK]",
            severity_color: COLOR_SUCCESS,
            summary: format!("Low load score ({:.1} / 100)", c.score),
        }
    }
}

fn spark_correlation_note(
    c: &crate::model::ChunkMetrics,
    spark: Option<&crate::protocol::spark::SparkProfileSummary>,
) -> Option<String> {
    let s = spark?;
    let top = s.top_consumers.first()?;
    let redstone_total = c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons;
    let short_class = top
        .class_name
        .split('.')
        .next_back()
        .unwrap_or(&top.class_name);

    if c.villagers >= 20 && top.method_name.to_lowercase().contains("entity") {
        Some(format!(
            "Spark matches: {short_class}.{} ({:.1}%) aligns with {} villagers in this chunk",
            top.method_name, top.percentage, c.villagers
        ))
    } else if c.hoppers >= 30
        && (top.method_name.to_lowercase().contains("blockentity")
            || top.method_name.to_lowercase().contains("hopper"))
    {
        Some(format!(
            "Spark matches: {short_class}.{} ({:.1}%) aligns with {} hoppers in this chunk",
            top.method_name, top.percentage, c.hoppers
        ))
    } else if redstone_total >= 40 && top.method_name.to_lowercase().contains("redstone") {
        Some(format!(
            "Spark matches: {short_class}.{} ({:.1}%) aligns with {} redstone components",
            top.method_name, top.percentage, redstone_total
        ))
    } else {
        Some(format!(
            "Spark top consumer: {short_class}.{} ({:.1}% self-time)",
            top.method_name, top.percentage
        ))
    }
}

fn render_hotspots(frame: &mut Frame, state: &mut AppState, area: Rect) {
    if state.filtered_chunk_indices.is_empty() {
        let message = if state.scan.total_chunks() == 0 {
            "Open a world or report to inspect chunks.\n\nchunklens tui /path/to/world\nchunklens tui report.json.gz\n\nPress ? for keyboard shortcuts."
        } else if !state.search_query.is_empty() {
            "No matching chunks.\nPress / to change the search filter."
        } else {
            "No chunks meet the score threshold in this category.\nPress 'f' to change category or scan with --min-score 0."
        };
        frame.render_widget(
            Paragraph::new(message).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(COLOR_BORDER))
                    .title(" Chunks "),
            ),
            area,
        );
        return;
    }

    let capacity = area.height.saturating_sub(3).max(1) as usize;
    state.page_size = capacity;
    let selected = state.table_state.selected().unwrap_or(0);
    if selected < state.table_scroll {
        state.table_scroll = selected;
    }
    if selected >= state.table_scroll + capacity {
        state.table_scroll = selected + 1 - capacity;
    }

    if area.width >= 96 {
        // Master-Detail Split View (lazygit / k9s style)
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(45), // Left: Offenders list
                Constraint::Percentage(55), // Right: Instant inspector
            ])
            .split(area);

        render_offenders_split(frame, state, split[0], capacity, selected);
        render_instant_inspector(frame, state, split[1]);
    } else {
        // Compact single-column view for narrow terminals (< 96 columns)
        render_offenders_compact(frame, state, area, capacity, selected);
    }
}

fn render_offenders_split(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    capacity: usize,
    selected: usize,
) {
    let header = Row::new([
        Span::styled(
            "#",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Chunk X,Z",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Score",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Tag",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "Offense",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .height(1);

    let rows: Vec<Row> = state
        .filtered_chunk_indices
        .iter()
        .enumerate()
        .skip(state.table_scroll)
        .take(capacity)
        .map(|(rank, &index)| {
            let c = &state.scan.chunks[index];
            let diag = diagnose_chunk(c);
            let is_selected = rank == selected;
            let rank_str = format!("{:<3}", rank + 1);
            let coord_str = format!("{}, {}", c.chunk_x, c.chunk_z);
            let score_str = format!("{:>5.1}", c.score);
            let offense_str = c.culprits_summary();

            Row::new(vec![
                Span::styled(
                    rank_str,
                    if is_selected {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(COLOR_MUTED)
                    },
                ),
                Span::styled(
                    coord_str,
                    if is_selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
                Span::styled(score_str, score_to_style(c.score)),
                Span::styled(
                    diag.tag,
                    Style::default()
                        .fg(diag.severity_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    offense_str,
                    if is_selected {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(COLOR_MUTED)
                    },
                ),
            ])
        })
        .collect();

    let title = format!(
        " [1: Top Offenders · {} matches] ",
        state.filtered_chunk_indices.len()
    );

    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Length(12),
            Constraint::Length(7),
            Constraint::Length(13),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER_ACTIVE))
            .title(Span::styled(
                title,
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
    )
    .row_highlight_style(
        Style::default()
            .bg(COLOR_SELECTION_BG)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    let mut local_state = ratatui::widgets::TableState::default();
    local_state.select(Some(selected.saturating_sub(state.table_scroll)));
    frame.render_stateful_widget(table, area, &mut local_state);
}

fn render_instant_inspector(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER));

    if let Some(c) = state.selected_chunk() {
        let diag = diagnose_chunk(c);
        let title = format!(" [2: Inspector · Chunk {}, {}] ", c.chunk_x, c.chunk_z);
        let block = block.title(Span::styled(
            title,
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let show_radar = state.show_minimap && inner.height >= 20;
        let layout = if show_radar {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2), // Location & Score Meta
                    Constraint::Length(2), // Diagnosis Banner
                    Constraint::Length(7), // Entity & Block Breakdown
                    Constraint::Length(2), // Spark Correlation
                    Constraint::Min(5),    // Topographic Radar
                    Constraint::Length(1), // Actions Strip
                ])
                .split(inner)
        } else {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2), // Location & Score Meta
                    Constraint::Length(2), // Diagnosis Banner
                    Constraint::Min(6),    // Entity & Block Breakdown
                    Constraint::Length(2), // Spark Correlation
                    Constraint::Length(1), // Actions Strip
                ])
                .split(inner)
        };

        // 1. Location & Meta
        let meta_line = Line::from(vec![
            Span::styled("Block: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("[{}, ~ , {}]", c.block_x(), c.block_z()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  Dim: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(short_dim(&c.dimension), Style::default().fg(Color::White)),
            Span::styled("  │  Score: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("{:.1} / 100", c.score),
                score_to_style(c.score).add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(Paragraph::new(meta_line), layout[0]);

        // 2. Diagnosis Banner
        let diag_line = Line::from(vec![
            Span::styled(
                format!(" {} ", diag.severity),
                Style::default()
                    .fg(Color::Black)
                    .bg(diag.severity_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                diag.tag,
                Style::default()
                    .fg(diag.severity_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(&diag.summary, Style::default().fg(Color::White)),
        ]);
        frame.render_widget(
            Paragraph::new(diag_line).wrap(Wrap { trim: true }),
            layout[1],
        );

        // 3. Entity & Block Breakdown (2 side-by-side columns)
        let breakdown_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(layout[2]);

        // Left Breakdown: Entities
        let mut ent_lines = vec![
            Line::from(Span::styled(
                format!("Entities ({})", c.entity_count),
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("─────────────────────────────"),
        ];
        let mut sorted_ent: Vec<_> = c.entity_types.iter().collect();
        sorted_ent.sort_by(|a, b| b.1.cmp(a.1));
        if sorted_ent.is_empty() {
            ent_lines.push(Line::from(Span::styled(
                "None",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (name, count) in sorted_ent.into_iter().take(6) {
                let clean = name.strip_prefix("minecraft:").unwrap_or(name);
                ent_lines.push(Line::from(vec![
                    Span::styled(format!(" {:<16}", clean), Style::default().fg(Color::White)),
                    Span::styled(format!("{:>5}", count), Style::default().fg(COLOR_WARNING)),
                ]));
            }
        }
        frame.render_widget(Paragraph::new(ent_lines), breakdown_split[0]);

        // Right Breakdown: Tile Entities & Redstone
        let redstone_total =
            c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons;
        let block_lines = vec![
            Line::from(Span::styled(
                format!(
                    "Blocks & Redstone ({})",
                    c.block_entity_count + redstone_total
                ),
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("─────────────────────────────"),
            Line::from(vec![
                Span::styled(" Hoppers:        ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>5}", c.hoppers),
                    Style::default().fg(if c.hoppers > 30 {
                        COLOR_DANGER
                    } else {
                        COLOR_WARNING
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Chests:         ", Style::default().fg(Color::White)),
                Span::styled(format!("{:>5}", c.chests), Style::default().fg(COLOR_MUTED)),
            ]),
            Line::from(vec![
                Span::styled(" Shulker Boxes:  ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>5}", c.shulkers),
                    Style::default().fg(if c.shulkers > 20 {
                        COLOR_DANGER
                    } else {
                        COLOR_MUTED
                    }),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Redstone Dust:  ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>5}", c.redstone_wire),
                    Style::default().fg(COLOR_MUTED),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Comparators:    ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>5}", c.comparators),
                    Style::default().fg(if c.comparators > 10 {
                        COLOR_WARNING
                    } else {
                        COLOR_MUTED
                    }),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(block_lines), breakdown_split[1]);

        // 4. Spark Correlation
        let spark_text = if let Some(note) = spark_correlation_note(c, state.spark_summary.as_ref())
        {
            Line::from(vec![
                Span::styled(
                    "SPARK: ",
                    Style::default()
                        .fg(COLOR_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(note, Style::default().fg(Color::White)),
            ])
        } else {
            Line::from(Span::styled(
                "SPARK: Profile not loaded (use --spark profile.json)",
                Style::default().fg(COLOR_MUTED),
            ))
        };
        frame.render_widget(Paragraph::new(spark_text), layout[3]);

        let action_purge_label = if state.rcon_addr.is_some() {
            "RCON Purge  │ "
        } else {
            "Copy /kill  │ "
        };

        if show_radar {
            render_minimap_radar(frame, state, c, layout[4]);
        }

        let action_idx = if show_radar { 5 } else { 4 };

        // 5. Actions Strip
        let actions = Line::from(vec![
            Span::styled("Actions: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                "[y/c] ",
                Style::default()
                    .fg(COLOR_SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Copy /tp  │ ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                "[x/p] ",
                Style::default()
                    .fg(COLOR_DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(action_purge_label, Style::default().fg(COLOR_MUTED)),
            Span::styled(
                "[m] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Radar  │ ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Inspector", Style::default().fg(COLOR_MUTED)),
        ]);
        frame.render_widget(Paragraph::new(actions), layout[action_idx]);
    } else {
        frame.render_widget(
            Paragraph::new("No chunk selected")
                .style(Style::default().fg(COLOR_MUTED))
                .block(block),
            area,
        );
    }
}

fn render_minimap_radar(frame: &mut Frame, state: &AppState, c: &ChunkMetrics, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Topographic Radar (Surrounding Chunks) ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 || inner.width < 15 {
        return;
    }

    let rows_radius = ((inner.height.saturating_sub(1) / 2) as i32).clamp(1, 2);
    let cols_radius = ((inner.width.saturating_sub(2) / 6) as i32).clamp(2, 5);

    let mut lines = Vec::new();
    for dz in -rows_radius..=rows_radius {
        let mut spans = Vec::new();
        for dx in -cols_radius..=cols_radius {
            let tx = c.chunk_x + dx;
            let tz = c.chunk_z + dz;

            if dx == 0 && dz == 0 {
                spans.push(Span::styled(
                    " [★]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                let score = state.chunk_score_at(tx, tz);
                let (sym, style) = match score {
                    Some(s) if s >= 80.0 => ("  █ ", Style::default().fg(COLOR_DANGER)),
                    Some(s) if s >= 50.0 => ("  ▓ ", Style::default().fg(COLOR_WARNING)),
                    Some(s) if s >= 20.0 => ("  ▒ ", Style::default().fg(Color::Rgb(245, 158, 11))),
                    Some(s) if s >= 5.0 => ("  ░ ", Style::default().fg(Color::Rgb(56, 189, 248))),
                    _ => ("  · ", Style::default().fg(COLOR_BORDER)),
                };
                spans.push(Span::styled(sym, style));
            }
        }
        lines.push(Line::from(spans));
    }

    let legend = Line::from(vec![
        Span::styled(
            "★ Target  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█ 80+  ", Style::default().fg(COLOR_DANGER)),
        Span::styled("▓ 50+  ", Style::default().fg(COLOR_WARNING)),
        Span::styled("▒ 20+  ", Style::default().fg(Color::Rgb(245, 158, 11))),
        Span::styled("░ 5+  ", Style::default().fg(Color::Rgb(56, 189, 248))),
        Span::styled("· Light", Style::default().fg(COLOR_BORDER)),
    ]);
    lines.push(legend);

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_purge_modal(frame: &mut Frame, state: &AppState, area: Rect) {
    let width = 64.min(area.width.saturating_sub(4));
    let height = 11.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let modal_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_DANGER))
        .title(Span::styled(
            " 🚨 Live RCON Action: Purge Offending Entities ",
            Style::default()
                .fg(COLOR_DANGER)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let chunk = state.selected_chunk();
    let (coord_str, dim_str, score_str, cmd_str) = match chunk {
        Some(c) => {
            let entity_type = if c.villagers > 15 {
                "villager"
            } else if c.dropped_items > 30 {
                "item"
            } else {
                "all"
            };
            (
                format!("[{}, {}]", c.chunk_x, c.chunk_z),
                c.dimension.clone(),
                format!("{:.1}", c.score),
                crate::live::generate_purge_command(
                    &c.dimension,
                    c.chunk_x,
                    c.chunk_z,
                    entity_type,
                ),
            )
        }
        None => (
            "N/A".to_string(),
            "unknown".to_string(),
            "0.0".to_string(),
            "N/A".to_string(),
        ),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Target Chunk: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                coord_str,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({dim_str}) • Score: "),
                Style::default().fg(COLOR_MUTED),
            ),
            Span::styled(
                score_str,
                Style::default()
                    .fg(COLOR_DANGER)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Remote Server: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                state
                    .rcon_addr
                    .map_or("None".to_string(), |a| a.to_string()),
                Style::default().fg(Color::Green),
            ),
        ]),
        Line::from(vec![
            Span::styled("Command: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("/{cmd_str}"),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                " [y / Enter] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Execute via RCON   ", Style::default().fg(Color::White)),
            Span::styled(
                " [c] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Copy Command   ", Style::default().fg(Color::White)),
            Span::styled(
                " [Esc] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Cancel", Style::default().fg(Color::White)),
        ]),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_offenders_compact(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    capacity: usize,
    selected: usize,
) {
    let visible_columns: &[usize] = if area.width >= 80 {
        &[0, 2, 3, 4, 5, 6, 7, 8]
    } else {
        &[2, 4, 5, 6, 7, 8]
    };
    let headings = [
        "#",
        "Dim",
        "Chunk X,Z",
        "Block X,Z",
        "Score",
        "Ent",
        "Vil",
        "Hop",
        "Red",
        "Signals",
    ];
    let widths = [4, 10, 14, 15, 7, 6, 6, 6, 7, 15];
    let header = Row::new(visible_columns.iter().map(|&i| {
        Span::styled(
            headings[i],
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        )
    }));
    let rows: Vec<Row> = state
        .filtered_chunk_indices
        .iter()
        .enumerate()
        .skip(state.table_scroll)
        .take(capacity)
        .map(|(rank, &index)| {
            let c = &state.scan.chunks[index];
            let values = [
                format!("{}", rank + 1),
                short_dim(&c.dimension),
                format!("{}, {}", c.chunk_x, c.chunk_z),
                format!("{}, {}", c.block_x(), c.block_z()),
                format!("{:.1}", c.score),
                c.entity_count.to_string(),
                c.villagers.to_string(),
                c.hoppers.to_string(),
                (c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons)
                    .to_string(),
                c.culprits_summary(),
            ];
            Row::new(
                visible_columns
                    .iter()
                    .map(|&i| {
                        Span::styled(
                            values[i].clone(),
                            if i == 4 {
                                score_to_style(c.score)
                            } else {
                                Style::default().fg(Color::White)
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    let title = format!(
        " Chunks · {} matches · {} score 80+ ",
        state.filtered_chunk_indices.len(),
        state.high_score_chunks
    );
    let table = Table::new(
        rows,
        visible_columns.iter().map(|&i| {
            if i == 9 {
                Constraint::Min(widths[i])
            } else {
                Constraint::Length(widths[i])
            }
        }),
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER))
            .title(title),
    )
    .row_highlight_style(
        Style::default()
            .bg(COLOR_SELECTION_BG)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    let mut local_state = ratatui::widgets::TableState::default();
    local_state.select(Some(selected.saturating_sub(state.table_scroll)));
    frame.render_stateful_widget(table, area, &mut local_state);
}

fn render_monitor(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.server_host.is_empty() {
        let message = "No live server selected.\n\nRun: chunklens tui report.json.gz --host example.com\nOr:  chunklens watch example.com\n\nPress 1 for chunks, or ? for keys.";
        frame.render_widget(
            Paragraph::new(message).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(COLOR_BORDER))
                    .title(" Monitor "),
            ),
            area,
        );
        return;
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10), // Top 3 status cards
            Constraint::Min(8),     // Event & Health Log
        ])
        .split(area);

    let top_cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Server metadata
            Constraint::Percentage(30), // Players gauge
            Constraint::Percentage(30), // Latency sparkline
        ])
        .split(layout[0]);

    // Card 1: Server Metadata
    let (ip_str, ver_str, motd_str, lat_str) = if let Some(p) = &state.latest_ping {
        (
            state
                .server_addr
                .map_or("Unknown".to_string(), |a| a.to_string()),
            format!("{} (p{})", p.version_name, p.protocol_version),
            p.description_clean.clone(),
            format!("{}ms", p.latency_ms),
        )
    } else {
        (
            state
                .server_addr
                .map_or("Resolving...".to_string(), |a| a.to_string()),
            "Awaiting response...".to_string(),
            "—".to_string(),
            "—".to_string(),
        )
    };

    let meta_lines = vec![
        Line::from(vec![
            Span::styled("Host:     ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                &state.server_host,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Socket:   ", Style::default().fg(COLOR_MUTED)),
            Span::styled(ip_str, Style::default().fg(Color::Cyan)),
            Span::styled("  Latency: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                lat_str,
                Style::default()
                    .fg(COLOR_SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Version:  ", Style::default().fg(COLOR_MUTED)),
            Span::styled(ver_str, Style::default().fg(COLOR_ACCENT)),
        ]),
        Line::from(vec![
            Span::styled("MOTD:     ", Style::default().fg(COLOR_MUTED)),
            Span::styled(motd_str, Style::default().fg(Color::White)),
        ]),
    ];

    let b1 = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Server Metadata ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(Paragraph::new(meta_lines).block(b1), top_cards[0]);

    // Card 2: Players Gauge
    let (online, max_players, ratio) = if let Some(p) = &state.latest_ping {
        let r = if p.max_players > 0 {
            (p.online_players as f64 / p.max_players as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (p.online_players, p.max_players, r)
    } else {
        (0, 0, 0.0)
    };

    let player_gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_BORDER))
                .title(Span::styled(
                    " Player Capacity ",
                    Style::default()
                        .fg(COLOR_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .gauge_style(
            Style::default()
                .fg(COLOR_SUCCESS)
                .bg(Color::Rgb(30, 41, 59)),
        )
        .label(Span::styled(
            format!("{online} / {max_players} Online"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ))
        .ratio(ratio);
    frame.render_widget(player_gauge, top_cards[1]);

    // Card 3: Latency Sparkline
    let sparkline_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Ping Latency History ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    let avg_lat = if !state.latency_history.is_empty() {
        state.latency_history.iter().sum::<u64>() / state.latency_history.len() as u64
    } else {
        0
    };

    let inner_spark = sparkline_block.inner(top_cards[2]);
    frame.render_widget(sparkline_block, top_cards[2]);

    let spark_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(inner_spark);

    let poll_ratio = (state.last_poll.elapsed().as_secs_f64() / state.poll_interval.as_secs_f64())
        .clamp(0.0, 1.0);
    let poll_bar = mini_progress_bar(poll_ratio, 6);
    let poll_label = if state.poll_pending {
        "syncing...".to_string()
    } else {
        format!(
            "{poll_bar} {:.1}s",
            state
                .last_poll
                .elapsed()
                .as_secs_f64()
                .min(state.poll_interval.as_secs_f64())
        )
    };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Avg: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(format!("{avg_lat}ms"), Style::default().fg(COLOR_SUCCESS)),
            Span::styled("  │ Poll: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(poll_label, Style::default().fg(COLOR_PRIMARY)),
        ])),
        spark_layout[0],
    );

    let sparkline = Sparkline::default()
        .data(&state.latency_history)
        .style(Style::default().fg(COLOR_PRIMARY))
        .max(300);
    frame.render_widget(sparkline, spark_layout[1]);

    // Bottom Card: Event & Health Log
    let event_items: Vec<ListItem> = state
        .monitor_events
        .iter()
        .rev()
        .take(50)
        .map(|(time, msg)| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{time}] "), Style::default().fg(COLOR_MUTED)),
                Span::styled(msg, Style::default().fg(Color::White)),
            ]))
        })
        .collect();

    let event_list = List::new(event_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER))
            .title(Span::styled(
                " Event & Incident Log ",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(event_list, layout[1]);
}

fn render_heatmap(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Potential Load Heatmap (Pan: Arrow Keys / Zoom: +/-) ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let grid_w = inner.width as usize;
    let grid_h = inner.height as usize;

    if grid_w == 0 || grid_h == 0 {
        return;
    }

    let scale = state.heatmap_scale.max(0.1);
    let center_x = state.heatmap_offset_x;
    let center_z = state.heatmap_offset_z;

    let sel_coord = state.selected_chunk().map(|c| (c.chunk_x, c.chunk_z));

    let mut lines = Vec::with_capacity(grid_h);

    for row in 0..grid_h {
        let mut line_spans = Vec::with_capacity(grid_w);
        let dz = (row as f64 - grid_h as f64 / 2.0) * scale;
        let chunk_z = (center_z + dz).round() as i32;

        for col in 0..grid_w {
            let dx = (col as f64 - grid_w as f64 / 2.0) * scale;
            let chunk_x = (center_x + dx).round() as i32;

            let is_sel = sel_coord.is_some_and(|(sx, sz)| sx == chunk_x && sz == chunk_z);

            if is_sel {
                line_spans.push(Span::styled(
                    "★",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else if let Some(&index) = state.spatial_index.get(&(chunk_x, chunk_z)) {
                let m = &state.scan.chunks[index];
                let sym = "■";
                let color = match m.score {
                    s if s >= 80.0 => COLOR_DANGER,
                    s if s >= 60.0 => Color::Rgb(244, 63, 94),
                    s if s >= 40.0 => COLOR_WARNING,
                    s if s >= 20.0 => COLOR_SUCCESS,
                    _ => Color::Rgb(71, 85, 105),
                };
                line_spans.push(Span::styled(sym, Style::default().fg(color)));
            } else {
                line_spans.push(Span::styled(
                    "·",
                    Style::default().fg(Color::Rgb(30, 41, 59)),
                ));
            }
        }
        lines.push(Line::from(line_spans));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_clusters(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(area);

    let cluster_items: Vec<ListItem> = state
        .scan
        .clusters
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let bx = c.center.0 * 16 + 8;
            let bz = c.center.1 * 16 + 8;
            let title = format!(
                "Cluster #{:<2} │ Center: [{}, {}] (Block {}, {})",
                i + 1,
                c.center.0,
                c.center.1,
                bx,
                bz
            );
            let stats = format!(
                "   Chunks: {:<2} │ Score: {:<6.1} │ Entities: {:<4} │ Hoppers: {}",
                c.chunks.len(),
                c.total_score,
                c.entities,
                c.hoppers + c.hopper_minecarts
            );
            let lines = vec![
                Line::from(Span::styled(
                    title,
                    Style::default()
                        .fg(COLOR_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(stats, Style::default().fg(COLOR_MUTED))),
            ];
            ListItem::new(lines)
        })
        .collect();

    let cluster_list = List::new(cluster_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_BORDER))
                .title(Span::styled(
                    format!(" Lag Clusters ({}) ", state.scan.clusters.len()),
                    Style::default()
                        .fg(COLOR_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(COLOR_SELECTION_BG)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    frame.render_stateful_widget(cluster_list, layout[0], &mut state.cluster_state);

    let detail_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Cluster Diagnostics ",
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));

    if let Some(c) = state.selected_cluster() {
        let inner = detail_block.inner(layout[1]);
        frame.render_widget(detail_block, layout[1]);

        let bx = c.center.0 * 16 + 8;
        let bz = c.center.1 * 16 + 8;
        let tp_cmd = format!("/tp @s {} ~ {}", bx, bz);

        let mut lines = vec![
            Line::from(vec![
                Span::styled("Center: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!(
                        "Chunk [{}, {}]  │  Block [{}, {}]",
                        c.center.0, c.center.1, bx, bz
                    ),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Teleport: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    &tp_cmd,
                    Style::default()
                        .fg(COLOR_SUCCESS)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Score: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}", c.total_score),
                    score_to_style(c.total_score / c.chunks.len().max(1) as f64),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Components:",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("  • Entities:          {}", c.entities)),
            Line::from(format!(
                "  • Hoppers & Carts:   {}",
                c.hoppers + c.hopper_minecarts
            )),
            Line::from(format!("  • Villagers:         {}", c.villagers)),
            Line::from(format!("  • Redstone Active:   {}", c.redstone_components)),
            Line::from(""),
            Line::from(Span::styled(
                "Connected Chunks:",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
        ];

        for (cx, cz) in &c.chunks {
            let score_str = if let Some(m) = state.scan.find(*cx, *cz) {
                format!("score {:.1}", m.score)
            } else {
                "scanned".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  • Chunk [{}, {}]", cx, cz),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" (Block {}, {}) ", cx * 16 + 8, cz * 16 + 8),
                    Style::default().fg(COLOR_MUTED),
                ),
                Span::styled(
                    format!("[{}]", score_str),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]));
        }

        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    } else {
        frame.render_widget(
            Paragraph::new("No cluster selected")
                .style(Style::default().fg(COLOR_MUTED))
                .block(detail_block),
            layout[1],
        );
    }
}

fn render_diagnostic(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Deep Diagnostic Inspector ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    if let Some(c) = state.selected_chunk() {
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if area.width < 72 {
            let top_entities = c
                .entity_types
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, count)| {
                    format!("{} ×{}", name.trim_start_matches("minecraft:"), count)
                })
                .unwrap_or_else(|| "none".to_owned());
            let top_blocks = c
                .block_entity_types
                .iter()
                .max_by_key(|(_, count)| *count)
                .map(|(name, count)| {
                    format!("{} ×{}", name.trim_start_matches("minecraft:"), count)
                })
                .unwrap_or_else(|| "none".to_owned());
            let lines = vec![
                Line::from(format!(
                    "{}  chunk {}, {}",
                    c.dimension, c.chunk_x, c.chunk_z
                )),
                Line::from(format!(
                    "Block {}, {}  ·  {}",
                    c.block_x(),
                    c.block_z(),
                    c.tp_command()
                )),
                Line::from(format!("Score {:.1}/100  ·  {}", c.score, c.severity())),
                Line::from(""),
                Line::from(format!(
                    "Entities {:>6}  ·  villagers {}",
                    c.entity_count, c.villagers
                )),
                Line::from(format!("Top entity    {top_entities}")),
                Line::from(format!(
                    "Block ent {:>5}  ·  hoppers {}",
                    c.block_entity_count, c.hoppers
                )),
                Line::from(format!("Top block     {top_blocks}")),
                Line::from(format!(
                    "Redstone      wire {}  repeater {}  comparator {}",
                    c.redstone_wire, c.repeaters, c.comparators
                )),
                Line::from(format!(
                    "              observer {}  piston {}",
                    c.observers, c.pistons
                )),
                Line::from(format!(
                    "Scheduled ticks {}  ·  items {}  ·  XP {}",
                    c.scheduled_ticks, c.dropped_items, c.exp_orbs
                )),
            ];
            frame.render_widget(Paragraph::new(lines), inner);
            return;
        }

        let diag_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Overview
                Constraint::Length(3), // Gauge
                Constraint::Min(10),   // 3 columns
                Constraint::Length(5), // Recommendations
            ])
            .split(inner);

        let overview = vec![
            Line::from(vec![
                Span::styled("Dimension: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    &c.dimension,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  │  Chunk: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("[{}, {}]", c.chunk_x, c.chunk_z),
                    Style::default()
                        .fg(COLOR_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  │  Block: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("[{}, ~ , {}]", c.block_x(), c.block_z()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Teleport:  ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    c.tp_command(),
                    Style::default()
                        .fg(COLOR_SUCCESS)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  [Press 'Y' or 'C' to copy]",
                    Style::default().fg(COLOR_MUTED),
                ),
            ]),
            Line::from(vec![
                Span::styled("Severity:  ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    c.severity(),
                    score_to_style(c.score).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" (Potential Load Score: {:.1} / 100)", c.score),
                    Style::default().fg(COLOR_MUTED),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(overview), diag_layout[0]);

        let ratio = (c.score / 100.0).clamp(0.0, 1.0);
        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::NONE)
                    .title("Offline potential score:"),
            )
            .gauge_style(score_to_style(c.score))
            .ratio(ratio);
        frame.render_widget(gauge, diag_layout[1]);

        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(diag_layout[2]);

        // Entities
        let mut ent_lines = vec![
            Line::from(Span::styled(
                format!("Entities ({})", c.entity_count),
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("────────────────────────────"),
        ];
        let mut sorted_entities: Vec<_> = c.entity_types.iter().collect();
        sorted_entities.sort_by(|a, b| b.1.cmp(a.1));
        if sorted_entities.is_empty() {
            ent_lines.push(Line::from(Span::styled(
                "None",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (name, count) in sorted_entities.into_iter().take(8) {
                let clean = name.strip_prefix("minecraft:").unwrap_or(name);
                ent_lines.push(Line::from(vec![
                    Span::styled(format!(" {:<18}", clean), Style::default().fg(Color::White)),
                    Span::styled(format!("{:>6}", count), Style::default().fg(COLOR_WARNING)),
                ]));
            }
        }
        let b1 = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER));
        frame.render_widget(Paragraph::new(ent_lines).block(b1), cols[0]);

        // Block Entities
        let mut be_lines = vec![
            Line::from(Span::styled(
                format!("Block Entities ({})", c.block_entity_count),
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("────────────────────────────"),
        ];
        let mut sorted_be: Vec<_> = c.block_entity_types.iter().collect();
        sorted_be.sort_by(|a, b| b.1.cmp(a.1));
        if sorted_be.is_empty() {
            be_lines.push(Line::from(Span::styled(
                "None",
                Style::default().fg(COLOR_MUTED),
            )));
        } else {
            for (name, count) in sorted_be.into_iter().take(8) {
                let clean = name.strip_prefix("minecraft:").unwrap_or(name);
                be_lines.push(Line::from(vec![
                    Span::styled(format!(" {:<18}", clean), Style::default().fg(Color::White)),
                    Span::styled(format!("{:>6}", count), Style::default().fg(COLOR_WARNING)),
                ]));
            }
        }
        let b2 = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER));
        frame.render_widget(Paragraph::new(be_lines).block(b2), cols[1]);

        // Redstone
        let redstone_total =
            c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons;
        let red_lines = vec![
            Line::from(Span::styled(
                format!("Redstone ({redstone_total})"),
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from("────────────────────────────"),
            Line::from(vec![
                Span::styled(" Redstone Dust:   ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.redstone_wire),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Repeaters:       ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.repeaters),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Comparators:     ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.comparators),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Observers:       ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.observers),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Pistons:         ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.pistons),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
            Line::from(vec![
                Span::styled(" Hoppers:         ", Style::default().fg(Color::White)),
                Span::styled(
                    format!("{:>6}", c.hoppers),
                    Style::default().fg(COLOR_DANGER),
                ),
            ]),
        ];
        let b3 = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER));
        frame.render_widget(Paragraph::new(red_lines).block(b3), cols[2]);

        // Recommendations
        let mut recs = Vec::new();
        if c.hoppers > 50 {
            recs.push(
                "High hopper density: lock idle hoppers with composters or full redstone power.",
            );
        }
        if c.villagers > 20 {
            recs.push("Large villager cluster: check live profiler data for pathfinding cost.");
        }
        if c.redstone_wire > 100 {
            recs.push("Heavy redstone dust updates: replace extensive wire runs with packed ice or target blocks.");
        }
        if c.dropped_items > 50 {
            recs.push(
                "Accumulated dropped items: inspect water stream alignment or collection hoppers.",
            );
        }
        if c.exp_orbs > 30 {
            recs.push("XP orb cluster: check live profiler data before clearing orbs.");
        }
        if c.scheduled_ticks > 50 {
            recs.push("Many saved scheduled ticks: inspect live block and fluid activity.");
        }
        if c.item_frames > 40 {
            recs.push("Many item frames: check client rendering and entity tracking under load.");
        }
        if recs.is_empty() {
            recs.push("No high counts in the tracked categories; live profiling is still needed.");
        }

        let rec_lines: Vec<Line> = recs
            .into_iter()
            .map(|r| {
                Line::from(Span::styled(
                    format!("  • {}", r),
                    Style::default().fg(COLOR_WARNING),
                ))
            })
            .collect();

        let rec_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_BORDER))
            .title(Span::styled(
                " Optimization Advice ",
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ));
        frame.render_widget(Paragraph::new(rec_lines).block(rec_block), diag_layout[3]);
    } else {
        frame.render_widget(
            Paragraph::new("No chunk selected")
                .style(Style::default().fg(COLOR_MUTED))
                .block(block),
            area,
        );
    }
}

fn render_spark(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Spark Profiler Analysis ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    if let Some(s) = &state.spark_summary {
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Platform & MSPT Gauge
                Constraint::Length(4), // Category Breakdown
                Constraint::Min(8),    // Top Methods Table
            ])
            .split(inner);

        let mspt_col = match s.average_mspt {
            Some(m) if m < 20.0 => COLOR_SUCCESS,
            Some(m) if m < 40.0 => COLOR_WARNING,
            Some(_) => COLOR_DANGER,
            None => COLOR_MUTED,
        };

        let meta = vec![
            Line::from(vec![
                Span::styled("Platform: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    &s.server_version,
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  │  Ticks Profiled: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{}", s.number_of_ticks),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled("  │  Duration: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}s", s.duration_seconds),
                    Style::default().fg(COLOR_MUTED),
                ),
            ]),
            Line::from(vec![
                Span::styled("Average MSPT: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    s.average_mspt
                        .map_or_else(|| "Not recorded".into(), |value| format!("{value:.1} ms")),
                    Style::default().fg(mspt_col).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    if s.average_mspt.is_some() {
                        " (Target: <= 50.0 ms for 20 TPS)"
                    } else {
                        ""
                    },
                    Style::default().fg(COLOR_MUTED),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(meta), layout[0]);

        // Categories
        let cb = &s.category_breakdown;
        let cat_line = Line::from(vec![
            Span::styled("Entities: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("{:.1}%  ", cb.entities_pct),
                if cb.entities_pct > 30.0 {
                    COLOR_DANGER
                } else {
                    COLOR_PRIMARY
                },
            ),
            Span::styled("Tile Entities: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("{:.1}%  ", cb.tile_entities_pct),
                if cb.tile_entities_pct > 25.0 {
                    COLOR_DANGER
                } else {
                    COLOR_PRIMARY
                },
            ),
            Span::styled("Redstone: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(
                format!("{:.1}%  ", cb.redstone_pct),
                if cb.redstone_pct > 15.0 {
                    COLOR_DANGER
                } else {
                    COLOR_PRIMARY
                },
            ),
            Span::styled("Chunk Load: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(format!("{:.1}%  ", cb.chunk_loading_pct), COLOR_MUTED),
            Span::styled("Plugins: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(format!("{:.1}%", cb.plugins_pct), COLOR_MUTED),
        ]);
        frame.render_widget(
            Paragraph::new(vec![Line::from("Category Share:"), cat_line]),
            layout[1],
        );

        // Top Methods
        let header = Row::new([
            Span::styled(
                " # ",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Self %",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Class & Method",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
        .height(1)
        .bottom_margin(1);

        let rows: Vec<Row> = s
            .top_consumers
            .iter()
            .enumerate()
            .map(|(i, m)| {
                Row::new(vec![
                    Span::raw(format!("{:2}", i + 1)),
                    Span::styled(
                        format!("{:>5.1}%", m.percentage),
                        if m.percentage > 20.0 {
                            Style::default()
                                .fg(COLOR_DANGER)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(COLOR_WARNING)
                        },
                    ),
                    Span::raw(format!("{}.{}", m.class_name, m.method_name)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(5),
                Constraint::Length(10),
                Constraint::Min(40),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_BORDER))
                .title(Span::styled(
                    " Top Methods by Self Sample Share ",
                    Style::default().fg(COLOR_PRIMARY),
                )),
        );

        frame.render_widget(table, layout[2]);
    } else {
        let msg = vec![
            Line::from(Span::styled(
                "No Spark profile report currently loaded.",
                Style::default().fg(COLOR_MUTED),
            )),
            Line::from(""),
            Line::from("To load and analyze a Spark profiler dump:"),
            Line::from("  chunklens tui report.json.gz --spark profile.json"),
            Line::from("  chunklens spark profile.json (print a summary)"),
        ];
        frame.render_widget(
            Paragraph::new(msg)
                .block(block)
                .alignment(Alignment::Center),
            area,
        );
    }
}

fn render_rcon(frame: &mut Frame, state: &AppState, area: Rect) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // Output logs
            Constraint::Length(3), // Input box
        ])
        .split(area);

    let log_items: Vec<ListItem> = state
        .rcon_logs
        .iter()
        .map(|(time, text, is_cmd)| {
            let style = if *is_cmd {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if text.starts_with("Error") || text.starts_with("Failed") {
                Style::default().fg(COLOR_DANGER)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("[{time}] "), Style::default().fg(COLOR_MUTED)),
                Span::styled(text, style),
            ]))
        })
        .collect();

    let logs_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " RCON Interactive Console ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    let logs_list = List::new(log_items).block(logs_block);
    frame.render_widget(logs_list, layout[0]);

    // Input line
    let input_line = Line::from(vec![
        Span::styled(
            "rcon ❯ ",
            Style::default()
                .fg(COLOR_SUCCESS)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &state.rcon_input,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("█", Style::default().fg(COLOR_PRIMARY)),
    ]);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_PRIMARY))
        .title(Span::styled(
            " Execute Command (Enter to send, Esc to clear) ",
            Style::default().fg(COLOR_MUTED),
        ));

    frame.render_widget(Paragraph::new(input_line).block(input_block), layout[1]);
}

fn render_footer(frame: &mut Frame, state: &AppState, area: Rect) {
    const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spinner = SPINNER_FRAMES[state.tick_count % SPINNER_FRAMES.len()];

    let text = if state.is_searching {
        format!(" /{}█  ·  Enter apply · Esc cancel", state.search_query)
    } else if let Some((message, _)) = &state.notification {
        format!(" {message}")
    } else if state.rcon_pending {
        format!(" {spinner} Executing RCON command in background...")
    } else if state.poll_pending && state.latest_ping.is_none() {
        format!(" {spinner} Connecting to {}...", state.server_host)
    } else {
        let shortcuts = match state.active_tab {
            Tab::Rcon => "Enter send · ↑↓ history · Tab switch · F1 help · Ctrl-C quit",
            Tab::Heatmap => "Arrows pan · +/- zoom · Esc chunks · ? help · Q quit",
            Tab::Monitor => "R refresh · S Spark · Tab switch · ? help · Q quit",
            Tab::Diagnostic | Tab::Spark => "Esc chunks · Tab switch · ? help · Q quit",
            _ => {
                "j/k move · f filter · y/c copy tp · x/p copy kill · / search · 1-4 tab · ? help · q quit"
            }
        };
        if state.search_query.is_empty() {
            format!(" {shortcuts}")
        } else {
            format!(" /{}  ·  {shortcuts}", state.search_query)
        }
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(COLOR_MUTED)),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(84, 84, area);
    frame.render_widget(Clear, popup);
    let text = "NAVIGATION & VIEWS:\n  1 Chunks · 2 Clusters · 3 Live Monitor · 4 Console\n  Tab / Shift-Tab    Cycle primary views\n  ↑/↓ or J/K         Navigate rows / offenders\n  PgUp / PgDn        Scroll table page\n  Enter or D         Open deep diagnostic inspector\n  /                  Instant search (coords, dimension, score)\n  F                  Cycle category filter (All, Mobs, Hoppers, Redstone, NBT)\n  Esc                Back to chunks; quit from chunks\n  Q or Ctrl-C        Quit\n\nACTIONS & TOOLS:\n  M                  Toggle Topographic 2D Minimap Radar\n  Y or C             Copy teleport command (/tp @s X ~ Z) to clipboard\n  X or P             Open live RCON entity purge modal (or copy /kill)\n  H                  Open 2D potential load heatmap\n  S                  Open Spark profiler report\n  R                  Refresh live ping & RCON status\n\nABOUT CHUNKLENS:\n  High-throughput Minecraft Anvil & Linear region scanner, split-screen\n  lag investigator, Spark correlator, and server configuration auditor.\n  Version 0.2.0 • Built in Rust with Ratatui";
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_PRIMARY))
                .title(" ChunkLens Help & Shortcuts "),
        ),
        popup,
    );
}

fn score_to_style(score: f64) -> Style {
    match score {
        s if s >= 80.0 => Style::default()
            .fg(COLOR_DANGER)
            .add_modifier(Modifier::BOLD),
        s if s >= 60.0 => Style::default().fg(COLOR_DANGER),
        s if s >= 40.0 => Style::default().fg(COLOR_WARNING),
        s if s >= 20.0 => Style::default().fg(COLOR_SUCCESS),
        _ => Style::default().fg(COLOR_MUTED),
    }
}

fn short_dim(dim: &str) -> String {
    if dim.contains("overworld") {
        "Overworld".to_string()
    } else if dim.contains("nether") {
        "The Nether".to_string()
    } else if dim.contains("end") {
        "The End".to_string()
    } else {
        dim.to_string()
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChunkMetrics, ScanResult};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn renders_all_tabs_at_common_sizes() {
        for (width, height) in [(40, 12), (60, 18), (80, 24), (120, 40), (180, 50)] {
            let scan = ScanResult {
                chunks: vec![ChunkMetrics {
                    dimension: "world".into(),
                    score: 42.0,
                    ..Default::default()
                }],
                ..Default::default()
            };
            let mut app = AppState::new(scan, None, None, None, None);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            for tab in [
                Tab::Hotspots,
                Tab::Clusters,
                Tab::Monitor,
                Tab::Rcon,
                Tab::Heatmap,
                Tab::Diagnostic,
                Tab::Spark,
            ] {
                app.active_tab = tab;
                terminal.draw(|frame| render(frame, &mut app)).unwrap();
            }
            app.help_visible = true;
            terminal.draw(|frame| render(frame, &mut app)).unwrap();
        }
    }

    #[test]
    fn viewport_tracks_selection_in_a_large_report() {
        let scan = ScanResult {
            chunks: (0..100_000)
                .map(|i| ChunkMetrics {
                    chunk_x: i,
                    dimension: "world".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut app = AppState::new(scan, None, None, None, None);
        app.table_state.select(Some(99_999));
        app.selected_chunk_index = 99_999;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        assert!(app.table_scroll > 99_900);
        assert!(app.page_size < 24);
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("99999, 0"));
    }
    #[test]
    fn test_diagnose_chunk_heuristics() {
        let vil_chunk = ChunkMetrics {
            villagers: 50,
            score: 75.0,
            ..Default::default()
        };
        let d1 = diagnose_chunk(&vil_chunk);
        assert_eq!(d1.tag, "[VILLAGER]");
        assert_eq!(d1.severity, "[CRIT]");

        let nbt_chunk = ChunkMetrics {
            payload_size: 2_000_000,
            shulkers: 60,
            ..Default::default()
        };
        let d2 = diagnose_chunk(&nbt_chunk);
        assert_eq!(d2.tag, "[NBT-STASH]");

        let hop_chunk = ChunkMetrics {
            hoppers: 120,
            ..Default::default()
        };
        let d3 = diagnose_chunk(&hop_chunk);
        assert_eq!(d3.tag, "[HOPPERS]");
    }
    #[test]
    fn test_render_split_preview() {
        let mut types = std::collections::BTreeMap::new();
        types.insert("minecraft:villager".to_string(), 142);
        types.insert("minecraft:iron_golem".to_string(), 4);
        let scan = ScanResult {
            chunks: vec![
                ChunkMetrics {
                    dimension: "minecraft:overworld".into(),
                    chunk_x: 14,
                    chunk_z: -8,
                    villagers: 142,
                    entity_count: 146,
                    hoppers: 88,
                    chests: 34,
                    comparators: 12,
                    repeaters: 8,
                    observers: 4,
                    score: 94.2,
                    entity_types: types,
                    ..Default::default()
                },
                ChunkMetrics {
                    dimension: "minecraft:overworld".into(),
                    chunk_x: -3,
                    chunk_z: 41,
                    score: 82.0,
                    payload_size: 1_800_000,
                    shulkers: 340,
                    ..Default::default()
                },
            ],
            world: "world".into(),
            ..Default::default()
        };
        let mut app = AppState::new(scan, Some("play.example.com".into()), None, None, None);
        app.latest_ping = Some(crate::protocol::slp::ServerPingResponse {
            latency_ms: 18,
            online_players: 34,
            max_players: 50,
            version_name: "Paper 1.21.1".into(),
            protocol_version: 767,
            description_clean: "Survival SMP".into(),
            raw_json: String::new(),
        });
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..24 {
            let row: String = (0..120).map(|x| buffer[(x, y)].symbol()).collect();
            println!("{}", row);
        }
    }
}
