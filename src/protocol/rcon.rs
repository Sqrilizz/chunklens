use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use anyhow::{Context, Result, ensure};

const SERVERDATA_AUTH: i32 = 3;
const SERVERDATA_EXECCOMMAND: i32 = 2;
const SERVERDATA_AUTH_RESPONSE: i32 = 2;

pub struct RconClient {
    stream: TcpStream,
    request_id: i32,
}

impl RconClient {
    pub fn connect(addr: SocketAddr, password: &str, timeout: Duration) -> Result<Self> {
        let stream = TcpStream::connect_timeout(&addr, timeout)
            .with_context(|| format!("Failed to connect to RCON server at {}", addr))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;

        let mut client = Self {
            stream,
            request_id: 1,
        };

        client.authenticate(password)?;
        Ok(client)
    }

    fn authenticate(&mut self, password: &str) -> Result<()> {
        let req_id = self.request_id;
        self.request_id += 1;

        self.send_packet(req_id, SERVERDATA_AUTH, password)?;

        for _ in 0..4 {
            let (id, kind, _) = self.read_packet()?;
            ensure!(id != -1, "RCON authentication failed: bad password");
            ensure!(id == req_id, "RCON authentication response ID mismatch");
            if kind == SERVERDATA_AUTH_RESPONSE {
                return Ok(());
            }
            ensure!(kind == 0, "unexpected RCON authentication response type");
        }
        anyhow::bail!("missing RCON authentication response")
    }

    pub fn execute(&mut self, command: &str) -> Result<String> {
        let req_id = self.request_id;
        let sentinel_id = req_id.checked_add(1).context("RCON request ID exhausted")?;
        self.request_id = sentinel_id
            .checked_add(1)
            .context("RCON request ID exhausted")?;

        self.send_packet(req_id, SERVERDATA_EXECCOMMAND, command)?;
        self.send_packet(sentinel_id, SERVERDATA_EXECCOMMAND, "")?;
        let mut response = String::new();
        loop {
            let (resp_id, kind, body) = self.read_packet()?;
            ensure!(kind == 0, "unexpected RCON command response type");
            if resp_id == sentinel_id {
                return Ok(response);
            }
            ensure!(resp_id == req_id, "unexpected RCON command response ID");
            ensure!(
                response.len() + body.len() <= 8 * 1024 * 1024,
                "RCON response exceeds 8 MiB limit"
            );
            response.push_str(&body);
        }
    }

    fn send_packet(&mut self, id: i32, packet_type: i32, body: &str) -> Result<()> {
        ensure!(
            !body.contains('\0') && body.len() <= 65525,
            "invalid RCON request body"
        );
        let body_bytes = body.as_bytes();
        let packet_size = (4 + 4 + body_bytes.len() + 2) as i32;

        let mut packet = Vec::with_capacity(packet_size as usize + 4);
        packet.extend_from_slice(&packet_size.to_le_bytes());
        packet.extend_from_slice(&id.to_le_bytes());
        packet.extend_from_slice(&packet_type.to_le_bytes());
        packet.extend_from_slice(body_bytes);
        packet.push(0x00); // 2-byte null terminator
        packet.push(0x00);

        self.stream.write_all(&packet)?;
        self.stream.flush()?;
        Ok(())
    }

    fn read_packet(&mut self) -> Result<(i32, i32, String)> {
        let mut size_buf = [0u8; 4];
        self.stream.read_exact(&mut size_buf)?;
        let size = i32::from_le_bytes(size_buf) as usize;
        if !(10..=65535).contains(&size) {
            anyhow::bail!("Invalid RCON packet size: {}", size);
        }

        let mut body_buf = vec![0u8; size];
        self.stream.read_exact(&mut body_buf)?;

        let id = i32::from_le_bytes([body_buf[0], body_buf[1], body_buf[2], body_buf[3]]);
        let packet_type = i32::from_le_bytes([body_buf[4], body_buf[5], body_buf[6], body_buf[7]]);

        ensure!(
            body_buf[size - 2..] == [0, 0],
            "RCON packet is not terminated"
        );
        let payload_slice = &body_buf[8..size - 2];

        let body = String::from_utf8_lossy(payload_slice).to_string();
        Ok((id, packet_type, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn handles_pre_auth_packet_and_unicode_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut peer = RconClient {
                stream,
                request_id: 1,
            };
            let (id, kind, body) = peer.read_packet().unwrap();
            assert_eq!((kind, body), (3, "test-password".into()));
            peer.send_packet(id, 0, "").unwrap();
            peer.send_packet(id, 2, "").unwrap();
            let (id, kind, body) = peer.read_packet().unwrap();
            assert_eq!((kind, body), (2, "list".into()));
            peer.send_packet(id, 0, "Игроков: 2").unwrap();
            let (sentinel_id, kind, body) = peer.read_packet().unwrap();
            assert_eq!((kind, body), (2, "".into()));
            peer.send_packet(sentinel_id, 0, "").unwrap();
        });
        let mut client =
            RconClient::connect(addr, "test-password", Duration::from_secs(2)).unwrap();
        assert_eq!(client.execute("list").unwrap(), "Игроков: 2");
        server.join().unwrap();
    }

    #[test]
    fn rejects_mismatched_authentication_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut peer = RconClient {
                stream,
                request_id: 1,
            };
            let (id, _, _) = peer.read_packet().unwrap();
            peer.send_packet(id + 100, 2, "").unwrap();
        });
        assert!(RconClient::connect(addr, "test", Duration::from_secs(2)).is_err());
        server.join().unwrap();
    }

    #[test]
    fn assembles_multiple_command_packets() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut peer = RconClient {
                stream,
                request_id: 1,
            };
            let (auth_id, _, _) = peer.read_packet().unwrap();
            peer.send_packet(auth_id, 2, "").unwrap();
            let (command_id, _, _) = peer.read_packet().unwrap();
            let (sentinel_id, _, _) = peer.read_packet().unwrap();
            peer.send_packet(command_id, 0, "first ").unwrap();
            peer.send_packet(command_id, 0, "second").unwrap();
            peer.send_packet(sentinel_id, 0, "").unwrap();
        });
        let mut client = RconClient::connect(addr, "test", Duration::from_secs(2)).unwrap();
        assert_eq!(client.execute("list").unwrap(), "first second");
        server.join().unwrap();
    }
}
