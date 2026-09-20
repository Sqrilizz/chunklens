use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPingResponse {
    pub latency_ms: u64,
    pub version_name: String,
    pub protocol_version: i32,
    pub online_players: u32,
    pub max_players: u32,
    pub description_clean: String,
    pub raw_json: String,
}

#[derive(Deserialize)]
struct RawSlpResponse {
    #[serde(default)]
    version: RawVersion,
    #[serde(default)]
    players: RawPlayers,
    #[serde(default)]
    description: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct RawVersion {
    #[serde(default)]
    name: String,
    #[serde(default)]
    protocol: i32,
}

#[derive(Deserialize, Default)]
struct RawPlayers {
    #[serde(default)]
    online: u32,
    #[serde(default)]
    max: u32,
}

pub fn resolve_minecraft_srv(host: &str) -> Option<(String, u16)> {
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let targets = [format!("_minecraft._tcp.{host}")];

    for srv_query in targets {
        if let Ok(output) = std::process::Command::new("dig")
            .args(["+short", "+time=1", "+tries=1", "SRV", &srv_query])
            .output()
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4
                    && let Ok(port) = parts[2].parse::<u16>()
                {
                    let target = parts[3].trim_end_matches('.').to_string();
                    if !target.is_empty() {
                        return Some((target, port));
                    }
                }
            }
        }
    }
    None
}

pub fn ping_server(host: &str, addr: SocketAddr, timeout: Duration) -> Result<ServerPingResponse> {
    let start_time = Instant::now();
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .with_context(|| format!("Failed to connect to Minecraft server at {}", addr))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    // Handshake packet (ID: 0x00)
    // Protocol version: 767 (1.21), Host: host, Port: addr.port(), Next state: 1 (status)
    let mut handshake_payload = Vec::new();
    write_varint(&mut handshake_payload, 0x00); // Packet ID 0
    write_varint(&mut handshake_payload, 767); // Protocol 767 (1.21.x)
    write_string(&mut handshake_payload, host);
    handshake_payload.extend_from_slice(&addr.port().to_be_bytes());
    write_varint(&mut handshake_payload, 1); // State 1 (Status)

    // Write framed Handshake packet
    write_packet(&mut stream, &handshake_payload)?;

    // Status Request packet (ID: 0x00, empty body)
    let mut status_request = Vec::new();
    write_varint(&mut status_request, 0x00);
    write_packet(&mut stream, &status_request)?;

    // Read Status Response packet
    let response_bytes = read_packet(&mut stream)?;
    let mut cursor = std::io::Cursor::new(response_bytes);
    let packet_id = read_varint(&mut cursor)?;
    if packet_id != 0x00 {
        anyhow::bail!("Invalid SLP response packet ID: {}", packet_id);
    }

    let json_str = read_string(&mut cursor)?;
    let latency_ms = start_time.elapsed().as_millis() as u64;

    // Parse JSON
    let parsed: RawSlpResponse =
        serde_json::from_str(&json_str).context("invalid server status JSON")?;

    let description_clean = extract_clean_motd(&parsed.description);

    Ok(ServerPingResponse {
        latency_ms,
        version_name: parsed.version.name,
        protocol_version: parsed.version.protocol,
        online_players: parsed.players.online,
        max_players: parsed.players.max,
        description_clean,
        raw_json: json_str,
    })
}

fn extract_clean_motd(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => strip_minecraft_codes(s),
        serde_json::Value::Object(obj) => {
            let mut out = String::new();
            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                out.push_str(text);
            }
            if let Some(extra) = obj.get("extra").and_then(|v| v.as_array()) {
                for item in extra {
                    out.push_str(&extract_clean_motd(item));
                }
            }
            strip_minecraft_codes(&out)
        }
        serde_json::Value::Array(parts) => parts.iter().map(extract_clean_motd).collect(),
        _ => String::new(),
    }
}

fn strip_minecraft_codes(s: &str) -> String {
    let mut clean = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' {
            let _ = chars.next(); // Skip formatting character
        } else if !c.is_control() || c == '\n' || c == '\t' {
            clean.push(c);
        }
    }
    clean.trim().to_string()
}

fn write_packet<W: Write>(writer: &mut W, data: &[u8]) -> Result<()> {
    let mut header = Vec::new();
    write_varint(&mut header, data.len() as i32);
    writer.write_all(&header)?;
    writer.write_all(data)?;
    writer.flush()?;
    Ok(())
}

fn read_packet<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
    let length = read_varint(reader)? as usize;
    if length > 2_097_152 {
        anyhow::bail!("Packet length too large: {}", length);
    }
    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf)?;
    Ok(buf)
}

pub fn write_varint(buf: &mut Vec<u8>, mut val: i32) {
    loop {
        if (val & !0x7F) == 0 {
            buf.push(val as u8);
            return;
        }
        buf.push(((val & 0x7F) | 0x80) as u8);
        val = ((val as u32) >> 7) as i32;
    }
}

pub fn read_varint<R: Read>(reader: &mut R) -> Result<i32> {
    let mut value = 0_u32;
    for index in 0..5 {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        if index == 4 && byte[0] & 0xf0 != 0 {
            anyhow::bail!("VarInt exceeds 32 bits");
        }
        value |= u32::from(byte[0] & 0x7f) << (index * 7);
        if byte[0] & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    anyhow::bail!("VarInt is too big")
}

pub fn resolve_server(
    address: &str,
    default_port: u16,
    use_srv: bool,
) -> Result<(String, SocketAddr)> {
    use std::net::ToSocketAddrs;
    let (host, port, explicit_port) = split_address(address, default_port)?;
    let (target, port) = if use_srv && !explicit_port {
        resolve_minecraft_srv(&host).unwrap_or_else(|| (host.clone(), port))
    } else {
        (host.clone(), port)
    };
    let mut addresses: Vec<_> = (target.as_str(), port).to_socket_addrs()?.collect();
    addresses.sort_by_key(|addr| addr.is_ipv6());
    let socket = addresses
        .first()
        .copied()
        .ok_or_else(|| anyhow::anyhow!("could not resolve {address}"))?;
    Ok((host, socket))
}

fn split_address(address: &str, default_port: u16) -> Result<(String, u16, bool)> {
    if let Ok(addr) = address.parse::<SocketAddr>() {
        return Ok((addr.ip().to_string(), addr.port(), true));
    }
    if let Ok(ip) = address.parse::<std::net::IpAddr>() {
        return Ok((ip.to_string(), default_port, false));
    }
    if let Some((host, port)) = address.rsplit_once(':') {
        anyhow::ensure!(
            !host.is_empty() && !host.contains(':') && !host.contains(['[', ']']),
            "invalid server address"
        );
        return Ok((
            host.to_string(),
            port.parse().context("invalid server port")?,
            true,
        ));
    }
    anyhow::ensure!(
        !address.is_empty() && !address.contains(char::is_whitespace),
        "invalid server address"
    );
    Ok((address.to_string(), default_port, false))
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    write_varint(buf, bytes.len() as i32);
    buf.extend_from_slice(bytes);
}

fn read_string<R: Read>(reader: &mut R) -> Result<String> {
    let length = read_varint(reader)? as usize;
    if length > 2_097_152 {
        anyhow::bail!("String length too large: {}", length);
    }

    let mut buf = vec![0u8; length];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| anyhow::anyhow!("Invalid UTF-8 in SLP string: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        let test_values = [
            0,
            1,
            2,
            127,
            128,
            255,
            256,
            65535,
            2147483647,
            -1,
            -2147483648,
        ];
        for &val in &test_values {
            let mut buf = Vec::new();
            write_varint(&mut buf, val);
            let mut cursor = std::io::Cursor::new(buf);
            let decoded = read_varint(&mut cursor).expect("decode varint");
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn test_strip_minecraft_codes() {
        let colored = "§aWelcome to §b§lServer§r! §c1.21";
        assert_eq!(strip_minecraft_codes(colored), "Welcome to Server! 1.21");
    }
    #[test]
    fn rejects_overlong_varints_without_panicking() {
        assert!(read_varint(&mut &b"\xff\xff\xff\xff\xff\x00"[..]).is_err());
        assert!(read_varint(&mut &b"\x80"[..]).is_err());
    }

    #[test]
    fn parses_ipv6_and_rejects_bad_ports() {
        assert_eq!(
            split_address("[::1]:25570", 25565).unwrap(),
            ("::1".into(), 25570, true)
        );
        assert_eq!(
            split_address("::1", 25565).unwrap(),
            ("::1".into(), 25565, false)
        );
        assert_eq!(
            split_address("example.com:25570", 25565).unwrap(),
            ("example.com".into(), 25570, true)
        );
        assert!(split_address("example.com:no", 25565).is_err());
    }
}
