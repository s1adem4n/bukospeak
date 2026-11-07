use std::io;
use std::path::Path;
use std::thread;

use futures::channel::mpsc::UnboundedSender;
use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{ListenerOptions, Name, Stream};

use crate::{Action, send};

fn name(folder: &Path) -> io::Result<Name<'static>> {
    #[cfg(unix)]
    let name = folder.join("instance.sock").to_fs_name::<interprocess::local_socket::GenericFilePath>()?;
    // A named pipe's name is anything but a backslash.
    #[cfg(windows)]
    let name = folder.to_string_lossy().replace('\\', "/").to_ns_name::<interprocess::local_socket::GenericNamespaced>()?;
    Ok(name.into_owned())
}

pub fn take(folder: &Path, actions: UnboundedSender<Action>) -> bool {
    let name = match name(folder) {
        Ok(name) => name,
        Err(error) => {
            tracing::warn!("no single instance: {error}");
            return true;
        }
    };
    if Stream::connect(name.borrow()).is_ok() {
        tracing::info!("BukoSpeak runs already, and shows its window");
        return false;
    }
    // No one answered, so a socket left there is from an app that ended without removing it.
    let listener = std::fs::create_dir_all(folder).and_then(|()| ListenerOptions::new().name(name).try_overwrite(true).create_sync());
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            tracing::warn!("no single instance: {error}");
            return true;
        }
    };
    let listening = thread::Builder::new().name("instance".into()).spawn(move || {
        for start in listener.incoming() {
            match start {
                Ok(_) => send(&actions, Action::Open),
                Err(error) => {
                    tracing::warn!("later starts aren't heard: {error}");
                    return;
                }
            }
        }
    });
    if let Err(error) = listening {
        tracing::warn!("no single instance: {error}");
    }
    true
}
