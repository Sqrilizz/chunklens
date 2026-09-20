use anyhow::Result;
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct Server {
    pub name: String,
    pub path: PathBuf,
    pub motd: Option<String>,
    pub port: Option<u16>,
    pub level_name: Option<String>,
    pub software: Option<String>,
    pub running: bool,
    pub online_players: Option<u32>,
    pub max_players: Option<u32>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct World {
    pub name: String,
    pub path: PathBuf,
    pub size_bytes: u64,
}

pub fn discover_servers(full_filesystem: bool) -> Result<Vec<Server>> {
    let mut targets: Vec<(PathBuf, usize)> = Vec::new();
    if full_filesystem {
        targets.push((PathBuf::from("/"), 8));
    } else {
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(&home);
            if h.join("Desktop").exists() {
                targets.push((h.join("Desktop"), 3));
            }
            if h.join("Documents").exists() {
                targets.push((h.join("Documents"), 3));
            }
            targets.push((h, 2));
        }
        for p in [
            "/srv",
            "/opt",
            "/mnt",
            "/var/lib/pterodactyl/volumes",
            "/var/lib/pelican",
            "/var/lib/featherpanel/volumes",
        ] {
            if Path::new(p).exists() {
                targets.push((PathBuf::from(p), 4));
            }
        }
    }
    discover_servers_in_targets(&targets)
}

#[allow(dead_code)]
pub fn discover_servers_in(roots: &[PathBuf], depth: usize) -> Result<Vec<Server>> {
    let targets: Vec<_> = roots.iter().cloned().map(|r| (r, depth)).collect();
    discover_servers_in_targets(&targets)
}

pub fn discover_servers_in_targets(targets: &[(PathBuf, usize)]) -> Result<Vec<Server>> {
    let mut paths = Vec::new();
    for (root, depth) in targets {
        let mut it = WalkDir::new(root)
            .follow_links(false)
            .max_depth(*depth)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if e.depth() > 0 && name.starts_with('.') && name != ".minecraft" {
                    return false;
                }
                if matches!(
                    name.as_ref(),
                    "node_modules"
                        | "target"
                        | "flatpak"
                        | "docker"
                        | "containerd"
                        | "snap"
                        | "proc"
                        | "sys"
                        | "dev"
                        | "libraries"
                        | ".cargo"
                        | ".rustup"
                        | "Games"
                        | ".steam"
                        | ".wine"
                ) {
                    return false;
                }
                true
            });

        while let Some(entry) = it.next() {
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_type().is_dir() {
                if entry.path().join("server.properties").is_file()
                    || entry.path().join("eula.txt").is_file()
                {
                    paths.push(entry.path().to_path_buf());
                    it.skip_current_dir();
                    continue;
                }
            } else if entry.file_type().is_file()
                && server_marker(&entry.file_name().to_string_lossy())
                && let Some(parent) = entry.path().parent()
            {
                paths.push(parent.to_path_buf());
            }
        }
    }
    paths.sort();
    paths.dedup();
    let panel_meta = load_panel_servers();
    Ok(paths
        .into_iter()
        .filter_map(|path| read_server(&path, &panel_meta).ok())
        .collect())
}

pub fn server_at(path: &Path) -> Result<Server> {
    let panel_meta = load_panel_servers();
    if path.join("server.properties").is_file() || path.join("eula.txt").is_file() {
        return read_server(path, &panel_meta);
    }
    let dir_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Minecraft Server")
        .to_owned();
    let uuid_key = dir_name.to_ascii_lowercase();
    let name = if let Some(meta) = panel_meta.get(&uuid_key) {
        if uuid_key.len() >= 8 && uuid_key.contains('-') {
            format!("{} ({})", meta.name, &uuid_key[..8])
        } else {
            meta.name.clone()
        }
    } else {
        dir_name
    };
    Ok(Server {
        name,
        path: path.to_owned(),
        motd: panel_meta
            .get(&uuid_key)
            .and_then(|m| m.description.clone()),
        port: None,
        level_name: None,
        software: None,
        running: server_running(path),
        online_players: None,
        max_players: None,
        latency_ms: None,
    })
}

fn find_listening_addrs(port: u16) -> Vec<std::net::SocketAddr> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    let mut addrs = Vec::new();
    let hex_port = format!(":{:04X} ", port);

    // Inspect Linux /proc/net/tcp for explicit listening bindings
    if let Ok(content) = fs::read_to_string("/proc/net/tcp") {
        for line in content.lines() {
            if let Some(pos) = line.find(&hex_port) {
                let rest = &line[pos + hex_port.len()..];
                let mut parts = rest.split_whitespace();
                let _rem = parts.next();
                if let Some(st) = parts.next()
                    && (st == "0A" || st == "01")
                {
                    let ip_str = line[..pos].split_whitespace().last().unwrap_or("");
                    if ip_str.len() == 8
                        && let Ok(num) = u32::from_str_radix(ip_str, 16)
                    {
                        let b0 = (num & 0xFF) as u8;
                        let b1 = ((num >> 8) & 0xFF) as u8;
                        let b2 = ((num >> 16) & 0xFF) as u8;
                        let b3 = ((num >> 24) & 0xFF) as u8;
                        let ipv4 = Ipv4Addr::new(b0, b1, b2, b3);
                        if ipv4.is_unspecified() {
                            addrs.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
                        } else {
                            addrs.push(SocketAddr::new(IpAddr::V4(ipv4), port));
                        }
                    }
                }
            }
        }
    }

    addrs.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    addrs.dedup();
    addrs
}

fn is_port_listening(port: u16) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    let addrs = find_listening_addrs(port);
    for addr in &addrs {
        if TcpStream::connect_timeout(addr, Duration::from_millis(80)).is_ok() {
            return true;
        }
    }
    let hex_port = format!(":{:04X} ", port);
    for net_file in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(content) = fs::read_to_string(net_file) {
            for line in content.lines() {
                if let Some(pos) = line.find(&hex_port) {
                    let rest = &line[pos + hex_port.len()..];
                    let mut parts = rest.split_whitespace();
                    let _rem = parts.next();
                    if let Some(st) = parts.next()
                        && (st == "0A" || st == "01")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn detect_software(path: &Path) -> Option<String> {
    if path.join("config/leaf-global.yml").is_file() || path.join("leaf.yml").is_file() {
        return Some("leaf".to_string());
    }
    if path.join("config/gale-global.yml").is_file() || path.join("gale.yml").is_file() {
        return Some("gale".to_string());
    }
    if path.join("config/folia-global.yml").is_file() || path.join("folia.yml").is_file() {
        return Some("folia".to_string());
    }
    if path.join("purpur.yml").is_file() {
        return Some("purpur".to_string());
    }
    if path.join("pufferfish.yml").is_file() {
        return Some("pufferfish".to_string());
    }
    if path.join("config/paper-global.yml").is_file() || path.join("paper.yml").is_file() {
        return Some("paper".to_string());
    }
    if path.join("velocity.toml").is_file() {
        return Some("velocity".to_string());
    }
    if path.join("flamecord.yml").is_file() {
        return Some("flamecord".to_string());
    }
    if path.join("bungee.yml").is_file() {
        return Some("bungeecord".to_string());
    }
    if path.join("spigot.yml").is_file() {
        return Some("spigot".to_string());
    }
    if path.join("bukkit.yml").is_file() {
        return Some("bukkit".to_string());
    }
    if path.join("fabric.mod.json").is_file() || path.join(".fabric").is_dir() {
        return Some("fabric".to_string());
    }
    if path.join("mods").is_dir() {
        return Some("forge".to_string());
    }

    let candidates = [
        "leaf",
        "gale",
        "folia",
        "purpur",
        "pufferfish",
        "paper",
        "fabric",
        "neoforge",
        "forge",
        "quilt",
        "spigot",
        "velocity",
        "bungee",
        "server",
    ];

    if let Ok(entries) = fs::read_dir(path) {
        let names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_ascii_lowercase())
            .collect();
        for candidate in candidates {
            if names.iter().any(|n| n.contains(candidate)) {
                return Some(candidate.to_string());
            }
        }
    }

    None
}

#[derive(Debug, Clone, Default)]
pub struct PanelServerMeta {
    pub name: String,
    pub description: Option<String>,
}

pub fn load_panel_servers() -> HashMap<String, PanelServerMeta> {
    let mut map = HashMap::new();

    // Check Wings daemon configs across Pterodactyl, FeatherPanel, Pelican
    for conf_path in [
        "/etc/pterodactyl/config.yml",
        "/etc/featherpanel/config.yml",
        "/etc/pelican/config.yml",
    ] {
        let Ok(content) = fs::read_to_string(conf_path) else {
            continue;
        };

        let mut token = None;
        let mut port = 8080u16;

        let mut in_api = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "api:" {
                in_api = true;
                continue;
            }
            if in_api && !line.starts_with(' ') && !line.starts_with('\t') {
                in_api = false;
            }

            if trimmed.starts_with("token:") && !trimmed.starts_with("token_id:") {
                let val = trimmed
                    .trim_start_matches("token:")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                token = Some(val.to_string());
            } else if in_api
                && trimmed.starts_with("port:")
                && let Ok(p) = trimmed.trim_start_matches("port:").trim().parse::<u16>()
            {
                port = p;
            }
        }

        if let Some(tok) = token
            && let Some(body) = fetch_wings_servers(port, &tok)
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
            && let Some(arr) = parsed.as_array()
        {
            for item in arr {
                if let Some(cfg) = item.get("configuration")
                    && let Some(uuid) = cfg.get("uuid").and_then(|u| u.as_str())
                {
                    let name = cfg
                        .get("meta")
                        .and_then(|m| m.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let desc = cfg
                        .get("meta")
                        .and_then(|m| m.get("description"))
                        .and_then(|d| d.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from);

                    if !name.is_empty() {
                        map.insert(
                            uuid.to_ascii_lowercase(),
                            PanelServerMeta {
                                name: name.to_string(),
                                description: desc,
                            },
                        );
                    }
                }
            }
        }
    }

    map
}

fn fetch_wings_servers(port: u16, token: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
    use std::time::Duration;

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(1000)))
        .ok()?;
    stream
        .set_write_timeout(Some(Duration::from_millis(300)))
        .ok()?;

    let request = format!(
        "GET /api/servers HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nUser-Agent: ChunkLens\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).ok()?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response).ok()?;
    let text = String::from_utf8_lossy(&response);

    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(&text);
    if let (Some(start), Some(end)) = (body.find('['), body.rfind(']'))
        && start <= end
    {
        return Some(body[start..=end].to_string());
    }
    None
}

fn read_server(path: &Path, panel_meta: &HashMap<String, PanelServerMeta>) -> Result<Server> {
    if !path.join("server.properties").is_file()
        && !path.join("eula.txt").is_file()
        && !path.join("world").is_dir()
    {
        anyhow::bail!("Not a minecraft server directory");
    }
    let properties = parse_properties(&path.join("server.properties")).unwrap_or_default();
    let software = detect_software(path);

    let port: Option<u16> = properties.get("server-port").and_then(|p| p.parse().ok());

    let dir_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("server")
        .to_owned();

    let uuid_key = dir_name.to_ascii_lowercase();

    let name = if let Some(meta) = panel_meta.get(&uuid_key) {
        if uuid_key.len() >= 8 && uuid_key.contains('-') {
            format!("{} ({})", meta.name, &uuid_key[..8])
        } else {
            meta.name.clone()
        }
    } else if (dir_name == "server" || dir_name == "world" || dir_name == "downloads")
        && path.parent().is_some()
    {
        let parent = path
            .parent()
            .unwrap()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if !parent.is_empty() {
            format!("{parent}/{dir_name}")
        } else {
            dir_name
        }
    } else {
        dir_name
    };

    let mut online_players = None;
    let mut max_players = None;
    let mut latency_ms = None;
    let mut live_motd = None;

    if let Some(p) = port {
        use std::time::Duration;
        let addrs = find_listening_addrs(p);
        for addr in addrs {
            let host_str = addr.ip().to_string();
            if let Ok(ping) =
                crate::protocol::slp::ping_server(&host_str, addr, Duration::from_millis(300))
            {
                online_players = Some(ping.online_players);
                max_players = Some(ping.max_players);
                latency_ms = Some(ping.latency_ms);
                if !ping.description_clean.is_empty()
                    && ping.description_clean != "A Minecraft Server"
                {
                    live_motd = Some(ping.description_clean);
                }
                break;
            }
        }
    }

    let running =
        online_players.is_some() || port.is_some_and(is_port_listening) || server_running(path);

    Ok(Server {
        name,
        path: path.to_path_buf(),
        motd: live_motd
            .or_else(|| properties.get("motd").cloned())
            .or_else(|| {
                panel_meta
                    .get(&uuid_key)
                    .and_then(|m| m.description.clone())
            }),
        port,
        level_name: properties.get("level-name").cloned(),
        software,
        running,
        online_players,
        max_players,
        latency_ms,
    })
}

fn server_marker(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name == "server.properties"
        || name == "eula.txt"
        || (name.ends_with(".jar")
            && ["paper", "purpur", "leaf", "fabric", "spigot", "server"]
                .iter()
                .any(|needle| name.contains(needle)))
}

pub fn discover_worlds(server: &Path) -> Result<Vec<World>> {
    let mut worlds = Vec::new();
    let root = server.canonicalize()?;
    let entries = WalkDir::new(&root)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(
                    "region"
                        | "entities"
                        | "poi"
                        | "playerdata"
                        | "advancements"
                        | "stats"
                        | "plugins"
                        | "libraries"
                        | "target"
                )
            )
        });
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if entry.file_type().is_dir() && path.join("region").is_dir() {
            let relative = path.strip_prefix(&root).unwrap_or(path);
            let name = if relative.as_os_str().is_empty() {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                relative.to_string_lossy().into_owned()
            };
            worlds.push(World {
                name,
                path: path.to_owned(),
                size_bytes: directory_size(path),
            });
        }
    }
    worlds.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(worlds)
}

fn parse_properties(path: &Path) -> Result<HashMap<String, String>> {
    Ok(fs::read_to_string(path)?
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect())
}

fn directory_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn server_running(path: &Path) -> bool {
    let Ok(server) = path.canonicalize() else {
        return false;
    };
    let Ok(processes) = fs::read_dir("/proc") else {
        return false;
    };
    let server_str = server.to_string_lossy();
    for entry in processes.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let proc_path = entry.path();
        if let Ok(cwd) = proc_path.join("cwd").canonicalize()
            && cwd == server
            && fs::read_link(proc_path.join("exe"))
                .ok()
                .and_then(|exe| exe.file_name().map(|name| name == "java"))
                .unwrap_or(false)
        {
            return true;
        }
        // Support Docker / FeatherPanel / Pterodactyl container mounts
        if let Ok(mountinfo) = fs::read_to_string(proc_path.join("mountinfo"))
            && mountinfo.contains(server_str.as_ref())
        {
            return true;
        }
    }
    false
}

pub fn ansi_motd(text: &str) -> String {
    let unescaped = text.replace("\\n", "\n");
    let mut output = String::new();
    let mut chars = unescaped.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            if let Some(code) = chars.next() {
                output.push_str(match code.to_ascii_lowercase() {
                    '0' => "\x1b[30m",
                    '1' => "\x1b[34m",
                    '2' => "\x1b[32m",
                    '3' => "\x1b[36m",
                    '4' => "\x1b[31m",
                    '5' => "\x1b[35m",
                    '6' => "\x1b[33m",
                    '7' => "\x1b[37m",
                    '8' => "\x1b[90m",
                    '9' => "\x1b[94m",
                    'a' => "\x1b[92m",
                    'b' => "\x1b[96m",
                    'c' => "\x1b[91m",
                    'd' => "\x1b[95m",
                    'e' => "\x1b[93m",
                    'f' => "\x1b[97m",
                    'l' => "\x1b[1m",
                    'r' => "\x1b[0m",
                    _ => "",
                });
            }
        } else {
            output.push(c);
        }
    }
    output + "\x1b[0m"
}

#[cfg(test)]
mod tests {
    use super::{ansi_motd, discover_servers_in};
    #[test]
    fn minecraft_colors_are_converted() {
        assert!(ansi_motd("§aGreen").contains("\x1b[92mGreen"));
    }

    #[test]
    fn discovers_server_from_eula_without_properties() {
        let temp = tempfile::tempdir().expect("temporary directory");
        std::fs::write(temp.path().join("eula.txt"), "eula=true").expect("eula");
        let servers = discover_servers_in(&[temp.path().to_owned()], 2).expect("discover");
        assert_eq!(servers.len(), 1);
    }
    #[test]
    fn discovers_nested_dimensions() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            "world/region",
            "world/DIM-1/region",
            "world/DIM1/region",
            "world/dimensions/custom/moon/region",
        ] {
            std::fs::create_dir_all(dir.path().join(path)).unwrap();
        }
        let worlds = super::discover_worlds(dir.path()).unwrap();
        assert_eq!(worlds.len(), 4);
        assert!(worlds.iter().any(|w| w.name == "world/DIM-1"));
        assert!(!super::server_running(dir.path()));
    }
}
