use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use bukospeak_proto::control::{self, SOUNDS};

use crate::downloads::{Source, compress};

pub struct Sounds {
    folder: PathBuf,
}

impl Sounds {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    fn files(&self) -> anyhow::Result<Vec<(String, PathBuf)>> {
        let entries = match fs::read_dir(&self.folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).with_context(|| format!("reading {}", self.folder.display())),
        };
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let is_wav = path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
            if !is_wav || !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name().into_string().map_err(|name| anyhow::anyhow!("{name:?} isn't UTF-8"))?;
            files.push((name.to_lowercase(), path));
        }
        files.sort();
        Ok(files)
    }
}

impl Source for Sounds {
    fn offers(&self, name: &str) -> anyhow::Result<Option<String>> {
        if name != SOUNDS {
            return Ok(None);
        }
        let files = self.files()?;
        if files.is_empty() {
            return Ok(None);
        }
        let mut stamp = String::new();
        for (name, path) in files {
            let (size, sha256) = control::digest(&mut fs::File::open(&path).with_context(|| format!("reading {}", path.display()))?)?;
            stamp.push_str(&format!("{name} {size} {sha256}\n"));
        }
        Ok(Some(stamp))
    }

    fn prepare(&self, _: &str, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
        let files = self.files()?;
        for (done, (name, path)) in files.iter().enumerate() {
            progress(done as u64, files.len() as u64);
            compress(&mut fs::File::open(path).with_context(|| format!("reading {}", path.display()))?, &folder.join(name))
                .with_context(|| format!("compressing {}", path.display()))?;
        }
        progress(files.len() as u64, files.len() as u64);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("bukospeak-sounds-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_folder_without_sounds_offers_none() {
        let folder = folder("none");
        let sounds = Sounds::new(folder.clone());
        assert_eq!(sounds.offers(SOUNDS).unwrap(), None);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("notes.txt"), b"not a sound").unwrap();
        assert_eq!(sounds.offers(SOUNDS).unwrap(), None);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn its_stamp_changes_with_its_sounds_and_its_names_are_lowercase() {
        let folder = folder("stamp");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("Joined.WAV"), b"first").unwrap();
        let sounds = Sounds::new(folder.clone());
        assert_eq!(sounds.offers("other").unwrap(), None);
        let before = sounds.offers(SOUNDS).unwrap().unwrap();
        assert!(before.starts_with("joined.wav 5 "), "{before}");
        fs::write(folder.join("Joined.WAV"), b"second").unwrap();
        assert_ne!(sounds.offers(SOUNDS).unwrap().unwrap(), before);
        fs::remove_dir_all(folder).unwrap();
    }
}
