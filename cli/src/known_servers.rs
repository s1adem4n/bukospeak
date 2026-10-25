//! The fingerprints of servers joined before, one `server fingerprint` line each, as SSH keeps
//! its known_hosts.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use bukospeak_core::proto::Fingerprint;

pub struct KnownServers {
    path: PathBuf,
    fingerprints: BTreeMap<String, Fingerprint>,
}

impl KnownServers {
    pub fn load() -> anyhow::Result<Self> {
        let path = dirs::config_dir().context("this system has no config folder")?.join("bukospeak/known_servers");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        let fingerprints = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (server, fingerprint) = line.split_once(' ').context("a line is a server and its fingerprint")?;
                Ok((server.to_owned(), fingerprint.parse()?))
            })
            .collect::<anyhow::Result<_>>()
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(Self { path, fingerprints })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, server: &str) -> Option<Fingerprint> {
        self.fingerprints.get(server).copied()
    }

    pub fn insert(&mut self, server: &str, fingerprint: Fingerprint) -> anyhow::Result<()> {
        self.fingerprints.insert(server.to_owned(), fingerprint);
        let text: String = self.fingerprints.iter().map(|(server, fingerprint)| format!("{server} {fingerprint}\n")).collect();
        let folder = self.path.parent().expect("the path is inside the config folder");
        fs::create_dir_all(folder).and_then(|()| fs::write(&self.path, text)).with_context(|| format!("writing {}", self.path.display()))
    }
}
