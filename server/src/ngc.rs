use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bukospeak_maxine::{Architecture, Kind, LICENSES, RUNTIME};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::downloads::{Source, compress};

const API: &str = "https://api.ngc.nvidia.com/v2/org/nvidia/team/maxine";

const VERSION: &str = "3.0.0";

const SDK: &str = "maxine_windows_audio_effects_sdk";

const STALLED: Duration = Duration::from_secs(60);

pub struct Ngc {
    key: String,
    agent: ureq::Agent,
}

impl Ngc {
    pub fn new(key: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(STALLED))
            .timeout_recv_response(Some(STALLED))
            .timeout_per_read(Some(STALLED))
            .build()
            .new_agent();
        Self { key, agent }
    }

    fn files(&self, version: &str) -> anyhow::Result<Vec<Remote>> {
        let listing: Listing = self
            .agent
            .get(format!("{API}/{version}/files"))
            .header("Authorization", format!("Bearer {}", self.key))
            .call()
            .and_then(|mut response| response.body_mut().read_json())
            .with_context(|| format!("listing NGC's {version}"))?;
        // NGC lists each file's MD5 as a file of its own.
        let files = listing.model_files.into_iter().chain(listing.recipe_files).filter(|file| !file.path.ends_with(".md5"));
        Ok(files.map(|file| Remote { version: version.to_owned(), ..file }).collect())
    }

    fn fetch<T>(
        &self,
        file: &Remote,
        done: &mut u64,
        total: u64,
        progress: &dyn Fn(u64, u64),
        read: impl FnOnce(&mut dyn Read) -> io::Result<T>,
    ) -> anyhow::Result<T> {
        let what = || format!("downloading {} of NGC's {}", file.path, file.version);
        // NGC redirects to storage the key isn't for; ureq leaves the Authorization out there.
        let mut response = self
            .agent
            .get(format!("{API}/{}/files/{}", file.version, file.path))
            .header("Authorization", format!("Bearer {}", self.key))
            .call()
            .with_context(what)?;
        let mut counted = Counted { inner: response.body_mut().as_reader(), hasher: Sha256::new(), file_bytes: 0, done, total, progress };
        let value = read(&mut counted).with_context(what)?;
        let digest = BASE64.encode(counted.hasher.finalize());
        anyhow::ensure!(
            counted.file_bytes == file.size && digest == file.sha256,
            "{} of NGC's {} came other than listed",
            file.path,
            file.version
        );
        Ok(value)
    }
}

impl Source for Ngc {
    fn offers(&self, name: &str) -> anyhow::Result<Option<String>> {
        Ok(Architecture::of_download(name).map(|_| VERSION.to_owned()))
    }

    fn prepare(&self, name: &str, folder: &Path, progress: &dyn Fn(u64, u64)) -> anyhow::Result<()> {
        let architecture = Architecture::of_download(name).context("no such download")?;
        let mut wanted = Vec::new();
        for kind in Kind::ALL {
            let model = format!("models/afx_win_{}/versions/{VERSION}", kind.name());
            let feature = Path::new("features").join(kind.feature());
            for file in self.files(&format!("{model}_dynamic_library"))? {
                wanted.push((feature.join("bin").join(&file.path), file));
            }
            for file in self.files(&format!("{model}_models_48k_{}", architecture.name()))? {
                wanted.push((feature.join("models").join(architecture.name()).join(&file.path), file));
            }
        }
        let licenses = format!("models/afx_win_{}/versions/{VERSION}_license", Kind::DereverbDenoiser.name());
        for file in self.files(&licenses)? {
            wanted.push((Path::new(LICENSES).join(&file.path), file));
        }
        let sdk = self.files(&format!("resources/{SDK}/versions/{VERSION}"))?;
        let [sdk] = &sdk[..] else { anyhow::bail!("NGC lists {} files for the SDK, not one", sdk.len()) };

        let total = sdk.size + wanted.iter().map(|(_, file)| file.size).sum::<u64>();
        let mut done = 0;
        progress(done, total);
        for (path, file) in &wanted {
            self.fetch(file, &mut done, total, progress, |read| compress(read, &folder.join(path)))?;
        }

        let archive = Removed(folder.with_extension("sdk.zip"));
        self.fetch(sdk, &mut done, total, progress, |read| io::copy(read, &mut fs::File::create(&archive.0)?))?;
        let mut archive = zip::ZipArchive::new(fs::File::open(&archive.0)?).context("opening the SDK")?;
        for path in RUNTIME {
            // zip checks each file's CRC as it is read to the end.
            let mut file = archive.by_name(path).with_context(|| format!("the SDK has no {path}"))?;
            compress(&mut file, &folder.join(path)).with_context(|| format!("unpacking {path} from the SDK"))?;
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Remote {
    #[serde(skip)]
    version: String,
    path: String,
    #[serde(rename = "sizeInBytes")]
    size: u64,
    #[serde(rename = "sha256_base64")]
    sha256: String,
}

#[derive(Deserialize)]
struct Listing {
    #[serde(rename = "modelFiles", default)]
    model_files: Vec<Remote>,
    #[serde(rename = "recipeFiles", default)]
    recipe_files: Vec<Remote>,
}

struct Counted<'a, R> {
    inner: R,
    hasher: Sha256,
    file_bytes: u64,
    done: &'a mut u64,
    total: u64,
    progress: &'a dyn Fn(u64, u64),
}

impl<R: Read> Read for Counted<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        self.file_bytes += read as u64;
        *self.done += read as u64;
        (self.progress)(*self.done, self.total);
        Ok(read)
    }
}

struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.0.display(), %error, "removing a file");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_counts_toward_the_download_and_is_hashed_as_it_comes() {
        let progress = std::cell::Cell::new((0, 0));
        let mut done = 10;
        let set = |done, total| progress.set((done, total));
        let mut counted = Counted { inner: &b"abc"[..], hasher: Sha256::new(), file_bytes: 0, done: &mut done, total: 20, progress: &set };
        io::copy(&mut counted, &mut io::sink()).unwrap();
        assert_eq!(counted.file_bytes, 3);
        assert_eq!(BASE64.encode(counted.hasher.finalize()), "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=");
        assert_eq!(progress.get(), (13, 20));
    }
}
