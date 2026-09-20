use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table,
        Tabs, Wrap,
    },
};

use super::app::{AppState, Tab};

// Professional Nordic/Catppuccin Palette
const COLOR_PRIMARY: Color = Color::Rgb(99, 102, 241); // Indigo
const COLOR_ACCENT: Color = Color::Rgb(168, 85, 247); // Purple
const COLOR_SUCCESS: Color = Color::Rgb(52, 211, 153); // Emerald
const COLOR_WARNING: Color = Color::Rgb(251, 191, 36); // Amber
const COLOR_DANGER: Color = Color::Rgb(244, 63, 94); // Rose
const COLOR_MUTED: Color = Color::Rgb(100, 116, 139); // Slate
const COLOR_BORDER: Color = Color::Rgb(71, 85, 105); // Dark Slate

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let size = frame.area();
    if size.width < 60 || size.height < 18 {
        frame.render_widget(Paragraph::new("ChunkLens needs a terminal of at least 60 × 18.\nResize the window, or press Q to quit.").wrap(Wrap {trim:true}),size);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Tabs
            Constraint::Min(10),   // Main View
            Constraint::Length(3), // Footer
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
    } else if state.is_searching {
        render_search_modal(frame, state, size);
    } else if let Some((msg, _)) = &state.notification {
        render_toast(frame, msg, size);
    }
}

fn render_header(frame: &mut Frame, state: &AppState, area: Rect) {
    let world = std::path::Path::new(&state.scan.world)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("No report");
    let (connection, color) = if state.server_host.is_empty() {
        ("Offline analysis".to_owned(), COLOR_MUTED)
    } else if state.monitor_error.is_some() {
        ("Disconnected · last known values".to_owned(), COLOR_DANGER)
    } else if let Some(ping) = &state.latest_ping {
        (
            format!("{} · {}ms", state.server_host, ping.latency_ms),
            COLOR_SUCCESS,
        )
    } else {
        (
            format!("Connecting to {}", state.server_host),
            COLOR_WARNING,
        )
    };
    let spans = vec![
        Span::styled(
            " CHUNKLENS ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {world} · {} / {} chunks  ",
                state.scan.chunks.len(),
                state.scan.total_chunks()
            ),
            Style::default().fg(Color::White),
        ),
        Span::styled(connection, Style::default().fg(color)),
    ];
    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_BORDER)),
        ),
        area,
    );
}

fn render_tabs(frame: &mut Frame, state: &AppState, area: Rect) {
    let titles = Tab::titles()
        .iter()
        .map(|t| Line::from(Span::raw(*t)))
        .collect::<Vec<_>>();

    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_BORDER)),
        )
        .select(state.active_tab.to_index())
        .style(Style::default().fg(COLOR_MUTED))
        .highlight_style(
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD)
                .bg(Color::Rgb(30, 27, 75)),
        )
        .divider(Span::styled("│", Style::default().fg(COLOR_BORDER)));

    frame.render_widget(tabs, area);
}

fn render_monitor(frame: &mut Frame, state: &AppState, area: Rect) {
    if state.server_host.is_empty() {
        let message = "Live monitoring is optional.\n\nchunklens watch localhost:25565\nchunklens tui report.json.gz --host localhost:25565\n\nSLP shows server status and response time.\nA saved world score estimates potential load; it does not measure MSPT.\n\nPress Tab to explore a loaded report, or ? for help.";
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

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Avg: ", Style::default().fg(COLOR_MUTED)),
            Span::styled(format!("{avg_lat}ms"), Style::default().fg(COLOR_SUCCESS)),
            Span::styled("  │ Polling: ", Style::default().fg(COLOR_MUTED)),
            Span::styled("every 3s", Style::default().fg(COLOR_MUTED)),
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

fn render_hotspots(frame: &mut Frame, state: &mut AppState, area: Rect) {
    if state.filtered_chunk_indices.is_empty() {
        let message = if state.scan.total_chunks() == 0 {
            "Open a world or report to inspect chunks.\n\nchunklens tui /path/to/world\nchunklens tui report.json.gz\n\nPress ? for keyboard shortcuts."
        } else if !state.search_query.is_empty() {
            "No matching chunks.\nPress / to change the filter."
        } else {
            "No chunks meet the score threshold.\nScan with --min-score 0 to retain every chunk."
        };
        frame.render_widget(
            Paragraph::new(message)
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(" Chunks ")),
            area,
        );
        return;
    }
    let preview = area.width >= 150;
    let regions = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(if preview {
            vec![Constraint::Percentage(70), Constraint::Percentage(30)]
        } else {
            vec![Constraint::Percentage(100)]
        })
        .split(area);
    let capacity = regions[0].height.saturating_sub(5).max(1) as usize;
    state.page_size = capacity;
    let selected = state.table_state.selected().unwrap_or(0);
    if selected < state.table_scroll {
        state.table_scroll = selected;
    }
    if selected >= state.table_scroll + capacity {
        state.table_scroll = selected + 1 - capacity;
    }
    let visible_columns: &[usize] = if regions[0].width >= 105 {
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]
    } else if regions[0].width >= 80 {
        &[0, 2, 4, 5, 6, 7, 9]
    } else {
        &[2, 4, 6, 7, 9]
    };
    let headings = [
        "#",
        "Dimension",
        "Chunk X, Z",
        "Block X, Z",
        "Score",
        "Band",
        "Entities",
        "Hoppers",
        "Redstone",
        "Villagers",
    ];
    let widths = [4, 10, 14, 15, 7, 10, 8, 8, 8, 9];
    let header = Row::new(
        visible_columns
            .iter()
            .map(|&i| Span::styled(headings[i], Style::default().fg(COLOR_PRIMARY))),
    )
    .bottom_margin(1);
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
                c.severity().into(),
                c.entity_count.to_string(),
                c.hoppers.to_string(),
                (c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons)
                    .to_string(),
                c.villagers.to_string(),
            ];
            Row::new(
                visible_columns
                    .iter()
                    .map(|&i| {
                        Span::styled(
                            values[i].clone(),
                            if i == 4 || i == 5 {
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
        " Chunks · {} matches · {} at 80+ · Potential Load Score ",
        state.filtered_chunk_indices.len(),
        state.high_score_chunks
    );
    let table = Table::new(
        rows,
        visible_columns.iter().map(|&i| {
            if i == 2 {
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
            .bg(Color::Rgb(49, 46, 129))
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("› ");
    let mut local_state = ratatui::widgets::TableState::default();
    local_state.select(Some(selected.saturating_sub(state.table_scroll)));
    frame.render_stateful_widget(table, regions[0], &mut local_state);
    if preview {
        render_chunk_preview(frame, state, regions[1]);
    }
}

fn render_chunk_preview(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_BORDER))
        .title(Span::styled(
            " Selected Chunk Preview ",
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ));

    if let Some(c) = state.selected_chunk() {
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines = vec![
            Line::from(vec![
                Span::styled("Chunk: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("[{}, {}]", c.chunk_x, c.chunk_z),
                    Style::default()
                        .fg(COLOR_PRIMARY)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  Block: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("[{}, {}]", c.block_x(), c.block_z()),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Dimension: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(&c.dimension, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Lag Score: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}", c.score),
                    score_to_style(c.score).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" [{}]", c.severity()), score_to_style(c.score)),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Teleport: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    c.tp_command(),
                    Style::default()
                        .fg(COLOR_SUCCESS)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(Span::styled(
                "Press 'C' to copy teleport command",
                Style::default().fg(COLOR_MUTED),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Key Metrics:",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("  • Entities:          {}", c.entity_count)),
            Line::from(format!("  • Block Entities:    {}", c.block_entity_count)),
            Line::from(format!(
                "  • Hoppers & Carts:   {}",
                c.hoppers + c.hopper_minecarts
            )),
            Line::from(format!("  • Villagers:         {}", c.villagers)),
            Line::from(format!(
                "  • Redstone Active:   {}",
                c.redstone_wire + c.repeaters + c.comparators + c.observers + c.pistons
            )),
            Line::from(format!(
                "  • Loose Items / XP:  {} / {}",
                c.dropped_items, c.exp_orbs
            )),
            Line::from(format!("  • Scheduled Ticks:   {}", c.scheduled_ticks)),
            Line::from(format!("  • Item Frames:       {}", c.item_frames)),
            Line::from(""),
            Line::from(Span::styled(
                "Press [Enter] for Deep Diagnostic Inspector",
                Style::default().fg(COLOR_ACCENT),
            )),
        ];

        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(p, inner);
    } else {
        frame.render_widget(
            Paragraph::new("No chunk selected")
                .style(Style::default().fg(COLOR_MUTED))
                .block(block),
            area,
        );
    }
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
                .bg(Color::Rgb(49, 46, 129))
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
                Span::styled("  [Press 'C' to copy]", Style::default().fg(COLOR_MUTED)),
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
                        " (measured, last minute)"
                    } else {
                        " (timing data required)"
                    },
                    Style::default().fg(mspt_col),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(meta), layout[0]);

        // Category breakdown
        let breakdown = vec![
            Line::from(Span::styled(
                "Self Sample Share by Category:",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled("Entities: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}%  ", s.category_breakdown.entities_pct),
                    Style::default().fg(COLOR_WARNING),
                ),
                Span::styled("Tile Entities: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}%  ", s.category_breakdown.tile_entities_pct),
                    Style::default().fg(COLOR_WARNING),
                ),
                Span::styled("Redstone: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}%  ", s.category_breakdown.redstone_pct),
                    Style::default().fg(COLOR_WARNING),
                ),
                Span::styled("Chunks: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}%  ", s.category_breakdown.chunk_loading_pct),
                    Style::default().fg(COLOR_WARNING),
                ),
                Span::styled("Other: ", Style::default().fg(COLOR_MUTED)),
                Span::styled(
                    format!("{:.1}%", s.category_breakdown.other_pct),
                    Style::default().fg(COLOR_WARNING),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(breakdown), layout[1]);

        // Top Methods Table
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
    let shortcuts = match state.active_tab {
        Tab::Rcon => "Enter Send · ↑↓ History · Tab Switch · F1 Help · Ctrl-C Quit",
        Tab::Heatmap => "Arrows Pan · +/- Zoom · Tab Switch · ? Help · Q Quit",
        Tab::Monitor => "R Refresh · Tab Switch · ? Help · Q Quit",
        _ => "↑↓ Select · Enter Inspect · / Filter · C Copy · ? Help · Q Quit",
    };
    frame.render_widget(
        Paragraph::new(shortcuts)
            .style(Style::default().fg(COLOR_MUTED))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(COLOR_BORDER)),
            ),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(88, 80, area);
    frame.render_widget(Clear, popup);
    let text = "KEYBOARD\n\nTab / Shift-Tab     Next / previous view\n1–7                 Jump to a view\n↑↓ or J/K           Select a chunk or cluster\nPage Up / Down      Move one visible page\nEnter               Inspect selected chunk\n/                   Filter by dimension, coordinates, or score\nEscape              Cancel filter / leave console\nC                   Copy the selected teleport command\nArrows, + / -       Pan and zoom the heatmap\nR                   Refresh live server status\nQ / Ctrl-C          Quit (Ctrl-C also works in the console)\n\nScores describe saved content. They do not measure live MSPT.\nPress any key to return.";
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_PRIMARY))
                .title(" ChunkLens help "),
        ),
        popup,
    );
}

fn render_search_modal(frame: &mut Frame, state: &AppState, area: Rect) {
    let popup_area = centered_rect(50, 20, area);
    frame.render_widget(Clear, popup_area);

    let text = vec![
        Line::from(Span::styled(
            "Type filter (dim, x,z, score):",
            Style::default().fg(COLOR_MUTED),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " > ",
                Style::default()
                    .fg(COLOR_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                &state.search_query,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("█", Style::default().fg(COLOR_PRIMARY)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press [Enter] to apply, [Esc] to cancel",
            Style::default().fg(COLOR_MUTED),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_PRIMARY))
        .title(Span::styled(
            " Search & Filter ",
            Style::default()
                .fg(COLOR_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ));

    frame.render_widget(Paragraph::new(text).block(block), popup_area);
}

fn render_toast(frame: &mut Frame, msg: &str, area: Rect) {
    let toast_area = Rect {
        x: area.width.saturating_sub(52).max(2),
        y: area.height.saturating_sub(5),
        width: 50.min(area.width.saturating_sub(4)),
        height: 3,
    };
    frame.render_widget(Clear, toast_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(COLOR_SUCCESS))
        .title(Span::styled(" Notice ", Style::default().fg(COLOR_SUCCESS)));

    let p = Paragraph::new(Line::from(Span::styled(
        msg,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )))
    .block(block)
    .alignment(Alignment::Center);

    frame.render_widget(p, toast_area);
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
        for (width, height) in [(40, 12), (80, 24), (120, 40), (180, 50)] {
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
            for index in 0..7 {
                app.active_tab = Tab::from_index(index);
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
}
