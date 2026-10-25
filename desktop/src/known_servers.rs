//! The fingerprints of servers joined before, one `server fingerprint` line each, as SSH keeps
//! its known_hosts.
//!
//! The terminal client and the app share the file, so each access reads it anew and a write
//! replaces it whole. Two of them pinning at the same moment may still lose one pin, which then
//! costs one more trust on first use; for the few joins there are, no lock is worth that.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::process;

use bukospeak_core::proto::{Fingerprint, ParseFingerprintError};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("reading {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    #[error("{}, line {line}: a line is a server and its fingerprint", path.display())]
    Line { path: PathBuf, line: usize },
    #[error("{}, line {line}: {source}", path.display())]
    Fingerprint { path: PathBuf, line: usize, source: ParseFingerprintError },
    #[error("writing {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
}

pub struct KnownServers {
    path: PathBuf,
}

impl KnownServers {
    /// The servers kept in the file at `path`, which needn't exist yet.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The fingerprint pinned for `server`, if it was joined before.
    pub fn get(&self, server: &str) -> Result<Option<Fingerprint>, Error> {
        Ok(self.read()?.remove(server))
    }

    /// Pins `fingerprint` for `server`, in place of the one pinned before.
    pub fn insert(&self, server: &str, fingerprint: Fingerprint) -> Result<(), Error> {
        let mut fingerprints = self.read()?;
        fingerprints.insert(server.to_owned(), fingerprint);
        let text: String = fingerprints.iter().map(|(server, fingerprint)| format!("{server} {fingerprint}\n")).collect();
        self.replace(&text).map_err(|source| Error::Write { path: self.path.clone(), source })
    }

    fn read(&self) -> Result<BTreeMap<String, Fingerprint>, Error> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
            Err(source) => return Err(Error::Read { path: self.path.clone(), source }),
        };
        text.lines()
            .enumerate()
            .filter(|(_, line)| !line.trim().is_empty())
            .map(|(index, line)| {
                let (path, line_number) = (self.path.clone(), index + 1);
                let Some((server, fingerprint)) = line.split_once(' ') else {
                    return Err(Error::Line { path, line: line_number });
                };
                let fingerprint = fingerprint.parse().map_err(|source| Error::Fingerprint { path, line: line_number, source })?;
                Ok((server.to_owned(), fingerprint))
            })
            .collect()
    }

    /// Writes a file of its own beside the old one and renames it over it, so a reader never
    /// sees half of it.
    fn replace(&self, text: &str) -> io::Result<()> {
        let folder = self.path.parent().expect("the file is inside a folder");
        let name = self.path.file_name().expect("the path names a file").to_string_lossy();
        let written = folder.join(format!(".{name}.{}", process::id()));
        fs::create_dir_all(folder)?;
        fs::write(&written, text)?;
        fs::rename(&written, &self.path).inspect_err(|_| {
            // The rename's error is the one to tell; the stray file only takes space.
            let _ = fs::remove_file(&written);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder of its own under the system's temporary one.
    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("bukospeak-known-servers-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&folder); // Left over from an earlier run, if at all.
        folder
    }

    fn fingerprint(byte: u8) -> Fingerprint {
        Fingerprint([byte; 32])
    }

    #[test]
    fn a_pin_is_kept_and_replaced() {
        let folder = folder("kept");
        let known = KnownServers::at(folder.join("config/known_servers"));
        assert_eq!(known.get("first:9988").unwrap(), None);
        known.insert("first:9988", fingerprint(1)).unwrap();
        known.insert("second:9988", fingerprint(2)).unwrap();
        known.insert("first:9988", fingerprint(3)).unwrap();
        assert_eq!(known.get("first:9988").unwrap(), Some(fingerprint(3)));
        assert_eq!(known.get("second:9988").unwrap(), Some(fingerprint(2)));
        assert_eq!(fs::read_dir(folder.join("config")).unwrap().count(), 1, "no file is left beside it");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_pin_written_by_another_client_meanwhile_stays() {
        let folder = folder("other");
        let path = folder.join("known_servers");
        let (app, terminal) = (KnownServers::at(path.clone()), KnownServers::at(path));
        app.insert("first:9988", fingerprint(1)).unwrap();
        terminal.insert("second:9988", fingerprint(2)).unwrap();
        app.insert("third:9988", fingerprint(3)).unwrap();
        assert_eq!(terminal.get("second:9988").unwrap(), Some(fingerprint(2)));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_broken_line_is_told_with_its_number() {
        let folder = folder("broken");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("known_servers");
        fs::write(&path, format!("first:9988 {}\n\nsecond:9988 nonsense\n", fingerprint(1))).unwrap();
        let error = KnownServers::at(path).get("first:9988").unwrap_err();
        assert!(matches!(error, Error::Fingerprint { line: 3, .. }), "{error}");
        fs::remove_dir_all(folder).unwrap();
    }
}
