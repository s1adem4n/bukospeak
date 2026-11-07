use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use serde::Deserialize;

const MAX_ANSWER: u64 = 1 << 20;

// tailscaled's LocalAPI insists on this Host, so that no web page can ask it through a browser.
const LOCAL_API_HOST: &str = "local-tailscaled.sock";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

pub trait Tailnet: Send + Sync + 'static {
    fn whois(&self, peer: SocketAddr) -> anyhow::Result<Option<Device>>;
}

impl<F: Fn(SocketAddr) -> Option<Device> + Send + Sync + 'static> Tailnet for F {
    fn whois(&self, peer: SocketAddr) -> anyhow::Result<Option<Device>> {
        Ok(self(peer))
    }
}

pub struct LocalApi {
    socket: PathBuf,
}

impl LocalApi {
    #[cfg(unix)]
    pub const DEFAULT_SOCKET: &str = "/var/run/tailscale/tailscaled.sock";
    #[cfg(windows)]
    pub const DEFAULT_SOCKET: &str = r"\\.\pipe\ProtectedPrefix\Administrators\Tailscale\tailscaled";

    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }

    fn get(&self, path: &str) -> anyhow::Result<(u16, Vec<u8>)> {
        let mut stream = connect(&self.socket).with_context(|| format!("connecting to tailscaled at {}", self.socket.display()))?;
        // HTTP/1.0: the answer ends with the connection, which needs no chunking read.
        write!(stream, "GET {path} HTTP/1.0\r\nHost: {LOCAL_API_HOST}\r\n\r\n")?;
        stream.flush()?;
        let mut answer = Vec::new();
        stream.take(MAX_ANSWER).read_to_end(&mut answer)?;
        parse(&answer)
    }
}

impl Tailnet for LocalApi {
    fn whois(&self, peer: SocketAddr) -> anyhow::Result<Option<Device>> {
        // tailscaled carries the connection in the kernel, so the address names the device
        // whatever the port.
        let (status, body) = self.get(&format!("/localapi/v0/whois?addr={}", peer.ip().to_canonical()))?;
        match status {
            200 => Ok(Some(serde_json::from_slice::<WhoIs>(&body).context("tailscaled's whois")?.device())),
            404 => Ok(None),
            _ => anyhow::bail!("tailscaled answered a whois with {status}: {}", String::from_utf8_lossy(&body).trim()),
        }
    }
}

#[cfg(unix)]
fn connect(socket: &std::path::Path) -> io::Result<impl Read + Write> {
    std::os::unix::net::UnixStream::connect(socket)
}

#[cfg(windows)]
fn connect(socket: &std::path::Path) -> io::Result<impl Read + Write> {
    std::fs::OpenOptions::new().read(true).write(true).open(socket)
}

fn parse(answer: &[u8]) -> anyhow::Result<(u16, Vec<u8>)> {
    let end = answer.windows(4).position(|window| window == b"\r\n\r\n").context("tailscaled's answer has no end of its head")?;
    let head = std::str::from_utf8(&answer[..end]).context("tailscaled's answer's head isn't text")?;
    let status = head.lines().next().and_then(|line| line.split(' ').nth(1)).and_then(|code| code.parse().ok());
    let status = status.with_context(|| format!("tailscaled's answer has no status: {head}"))?;
    Ok((status, answer[end + 4..].to_vec()))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WhoIs {
    node: Node,
    user_profile: UserProfile,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Node {
    #[serde(rename = "StableID")]
    stable_id: String,
    name: String,
    #[serde(default)]
    tags: Vec<String>,
}

impl Node {
    fn host_name(&self) -> &str {
        self.name.split('.').next().unwrap_or_default()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserProfile {
    login_name: String,
    display_name: String,
}

impl WhoIs {
    fn device(self) -> Device {
        // A tagged device belongs to the tailnet, whose profile names no person.
        let name = if !self.node.tags.is_empty() {
            self.node.host_name().to_owned()
        } else if self.user_profile.display_name.trim().is_empty() {
            self.user_profile.login_name
        } else {
            self.user_profile.display_name
        };
        Device { id: self.node.stable_id, name }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whois(node: &str, profile: &str) -> Device {
        serde_json::from_str::<WhoIs>(&format!(r#"{{"Node": {node}, "UserProfile": {profile}, "CapMap": {{}}}}"#)).unwrap().device()
    }

    #[test]
    fn a_device_is_named_after_its_owner() {
        let device = whois(
            r#"{"ID": 7, "StableID": "nFirst", "Name": "laptop.example.ts.net.", "Addresses": ["100.64.0.1/32"]}"#,
            r#"{"ID": 3, "LoginName": "first@example.com", "DisplayName": "First Member", "ProfilePicURL": ""}"#,
        );
        assert_eq!(device, Device { id: "nFirst".into(), name: "First Member".into() });
    }

    #[test]
    fn an_owner_without_a_display_name_goes_by_the_login() {
        let device = whois(
            r#"{"StableID": "nFirst", "Name": "laptop.example.ts.net."}"#,
            r#"{"LoginName": "first@example.com", "DisplayName": ""}"#,
        );
        assert_eq!(device.name, "first@example.com");
    }

    #[test]
    fn a_tagged_device_is_named_after_itself() {
        let device = whois(
            r#"{"StableID": "nServer", "Name": "speaker.example.ts.net.", "Tags": ["tag:speaker"]}"#,
            r#"{"LoginName": "tagged-devices", "DisplayName": "tagged-devices"}"#,
        );
        assert_eq!(device.name, "speaker");
    }

    #[test]
    fn an_answer_parts_into_its_status_and_body() {
        let answer = b"HTTP/1.0 404 Not Found\r\nContent-Type: text/plain\r\n\r\nno match for IP:port\n";
        assert_eq!(parse(answer).unwrap(), (404, b"no match for IP:port\n".to_vec()));
        assert!(parse(b"HTTP/1.0 200 OK\r\n").is_err());
    }

    #[cfg(unix)]
    fn local_api(name: &str, answer: String) -> (LocalApi, std::thread::JoinHandle<String>) {
        use std::io::BufRead;

        let socket = std::env::temp_dir().join(format!("bukospeak-tailscaled-{name}-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let asked = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            let mut reader = io::BufReader::new(&mut stream);
            while reader.read_line(&mut request).unwrap() > "\r\n".len() {}
            stream.write_all(answer.as_bytes()).unwrap();
            request
        });
        (LocalApi::new(socket), asked)
    }

    #[cfg(unix)]
    #[test]
    fn asks_tailscaled_for_the_peers_address() {
        let body = r#"{"Node": {"StableID": "nFirst", "Name": "laptop.example.ts.net."}, "UserProfile": {"LoginName": "first@example.com", "DisplayName": "First"}}"#;
        let (api, asked) = local_api("known", format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{body}"));
        let device = api.whois("[::ffff:100.64.0.1]:41641".parse().unwrap()).unwrap();
        assert_eq!(device, Some(Device { id: "nFirst".into(), name: "First".into() }));
        let request = asked.join().unwrap();
        assert!(request.starts_with("GET /localapi/v0/whois?addr=100.64.0.1 HTTP/1.0\r\n"), "{request}");
        assert!(request.contains("\r\nHost: local-tailscaled.sock\r\n"), "{request}");
    }

    #[cfg(unix)]
    #[test]
    fn a_peer_tailscaled_doesnt_know_is_nobody() {
        let (api, _) = local_api("stranger", "HTTP/1.0 404 Not Found\r\n\r\nno match for IP:port\n".into());
        assert_eq!(api.whois("127.0.0.1:41641".parse().unwrap()).unwrap(), None);
    }
}
