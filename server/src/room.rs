use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use bukospeak_proto::control::{self, ClientMessage, Member, Rejection, ServerMessage};
use bukospeak_proto::{KEEP_ALIVE, MemberId, PROTOCOL_VERSION, REPLACED, SILENT_AFTER, audio};
use bytes::Bytes;
use quinn::{Connection, ConnectionError, Endpoint, Incoming, RecvStream, SendStream, VarInt};
use tokio::sync::mpsc;
use tokio::time::{Instant, timeout};

use crate::downloads::Downloads;
use crate::tailnet::{Device, Tailnet};

/// More than a group of friends needs, and few enough that relaying to all stays cheap.
const MAX_MEMBERS: usize = 16;

/// The longest name members see, in characters; a longer one is cut.
const MAX_NAME_CHARS: usize = 32;

/// How long a new connection has to say Hello, and the tailnet to tell who it is.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a turned-away client has to read why before the server closes on it.
const REJECTION_GRACE: Duration = Duration::from_secs(1);

/// How long the place of a member whose connection was lost is held for its device to come
/// back: the member notices the loss about when the server does, and then has a join's time.
const RESUME_GRACE: Duration = Duration::from_secs(20);

/// The one channel: who is in it, and the relay of their audio.
pub struct Room {
    tailnet: Arc<dyn Tailnet>,
    members: Mutex<Members>,
    downloads: Downloads,
}

#[derive(Default)]
struct Members {
    peers: BTreeMap<MemberId, Peer>,
    /// The outboxes of the clients that watch the channel without joining it.
    watchers: Vec<mpsc::UnboundedSender<ServerMessage>>,
    last_id: u16,
}

struct Peer {
    /// The [`Device::id`] of the device the member is.
    device: String,
    name: String,
    muted: bool,
    reconnecting: bool,
    connection: Connection,
    control: mpsc::UnboundedSender<ServerMessage>,
}

impl Peer {
    fn member(&self, id: MemberId) -> Member {
        Member { id, name: self.name.clone(), muted: self.muted, reconnecting: self.reconnecting }
    }

    /// Whether the peer is on `connection`, not on one that took its place.
    fn is_on(&self, connection: &Connection) -> bool {
        self.connection.stable_id() == connection.stable_id()
    }
}

impl Room {
    /// The channel that whoever `tailnet` knows may join, watch, and download what `downloads`
    /// offers from.
    pub fn new(tailnet: impl Tailnet, downloads: Downloads) -> Arc<Self> {
        Arc::new(Self { tailnet: Arc::new(tailnet), members: Mutex::default(), downloads })
    }

    /// Admits the endpoint's connections until it closes.
    pub async fn serve(self: Arc<Self>, endpoint: Endpoint) {
        while let Some(incoming) = endpoint.accept().await {
            let room = Arc::clone(&self);
            tokio::spawn(async move {
                let remote = incoming.remote_address();
                if let Err(error) = room.admit(incoming).await {
                    tracing::debug!(%remote, %error, "connection ended");
                }
            });
        }
    }

    /// Lets a connection join once its Hello checks out, and relays for it until it leaves or is
    /// lost and its device doesn't come back; once its Watch checks out, tells it who is in the
    /// channel until it closes; or once its Download checks out, sends it the download. A
    /// first message checks out if it is of this protocol's version and the tailnet knows the
    /// device it comes from.
    async fn admit(&self, incoming: Incoming) -> anyhow::Result<()> {
        let connection = incoming.await?;
        let (send, recv, first, device) = timeout(HELLO_TIMEOUT, async {
            let (send, mut recv) = connection.accept_bi().await?;
            let first = control::read(&mut recv).await?.context("the stream ended before a Hello, a Watch or a Download")?;
            let device = self.whois(connection.remote_address()).await;
            anyhow::Ok((send, recv, first, device))
        })
        .await??;
        let (version, asked) = match first {
            ClientMessage::Hello { version, muted } => (version, Asked::Join { muted }),
            ClientMessage::Watch { version } => (version, Asked::Watch),
            ClientMessage::Download { version, name } => (version, Asked::Download(name)),
            ClientMessage::Mute { .. } => anyhow::bail!("a Mute before a Hello"),
        };
        let device = match (version == PROTOCOL_VERSION, device) {
            (false, _) => return reject(send, &connection, Rejection::Version).await,
            (true, Ok(Some(device))) => device,
            (true, Ok(None)) => return reject(send, &connection, Rejection::Stranger).await,
            (true, Err(error)) => {
                tracing::warn!(remote = %connection.remote_address(), "asking the tailnet who connects: {error:#}");
                return reject(send, &connection, Rejection::Unidentified).await;
            }
        };
        let (control, outbox) = mpsc::unbounded_channel();
        match asked {
            Asked::Join { muted } => {
                let id = match self.join(&device, muted, &connection, control) {
                    Ok(id) => id,
                    Err(reason) => return reject(send, &connection, reason).await,
                };
                tracing::info!(id = id.0, device = device.id, remote = %connection.remote_address(), "joined");
                tokio::select! {
                    () = deliver(send, outbox) => {}
                    () = self.relay(id, &connection) => {}
                    () = self.listen(id, &connection, recv) => {}
                    () = self.notice_silence(id, &connection) => {}
                }
                // Shown reconnecting while its place is held, unless another connection took it.
                if is_lost(&connection) && self.change(id, &connection, |peer| peer.reconnecting = true) {
                    tracing::info!(id = id.0, "reconnecting");
                    tokio::time::sleep(RESUME_GRACE).await;
                }
                self.leave(id, &connection);
            }
            Asked::Watch => {
                self.watch(control);
                tracing::debug!(device = device.id, remote = %connection.remote_address(), "watching");
                tokio::select! {
                    () = deliver(send, outbox) => {}
                    () = drain(recv) => {}
                }
            }
            Asked::Download(name) => {
                tracing::debug!(name, device = device.id, remote = %connection.remote_address(), "downloading");
                // The client says nothing more on the control stream.
                drop(recv);
                self.downloads.serve(&name, &connection, send).await?;
            }
        }
        Ok(())
    }

    /// The device that connects from `peer`, as the tailnet tells it.
    async fn whois(&self, peer: SocketAddr) -> anyhow::Result<Option<Device>> {
        let tailnet = Arc::clone(&self.tailnet);
        tokio::task::spawn_blocking(move || tailnet.whois(peer)).await.context("asking the tailnet panicked")?
    }

    /// Adds the device as a member, or puts the member it is on the connection, closing its old
    /// one; welcomes it with everyone in the channel and tells the others.
    fn join(
        &self,
        device: &Device,
        muted: bool,
        connection: &Connection,
        control: mpsc::UnboundedSender<ServerMessage>,
    ) -> Result<MemberId, Rejection> {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        let held = members.peers.iter().find(|(_, peer)| peer.device == device.id).map(|(&id, _)| id);
        let peer = Peer {
            device: device.id.clone(),
            name: shown(&device.name),
            muted,
            reconnecting: false,
            connection: connection.clone(),
            control: control.clone(),
        };
        let (id, back) = match held.and_then(|id| members.peers.remove_entry(&id)) {
            Some((id, old)) => {
                old.connection.close(VarInt::from_u32(REPLACED), b"replaced");
                (id, true)
            }
            None if members.peers.len() >= MAX_MEMBERS => return Err(Rejection::Full),
            None => (members.next_id(), false),
        };
        let member = peer.member(id);
        // Told while the member is out of the map: it hears of itself only in its Welcome.
        members.tell(&if back { ServerMessage::Changed { member } } else { ServerMessage::Joined { member } });
        members.peers.insert(id, peer);
        // The channel is fresh and its receiver alive, so the Welcome is queued first.
        let _ = control.send(ServerMessage::Welcome { you: id, members: members.everyone() });
        Ok(id)
    }

    /// Tells a watcher who is in the channel, and from now on who joins and leaves.
    fn watch(&self, control: mpsc::UnboundedSender<ServerMessage>) {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        // The channel is fresh and its receiver alive, so the Present is queued first.
        let _ = control.send(ServerMessage::Present { members: members.everyone() });
        members.watchers.push(control);
    }

    /// Mutes or unmutes the member as it asks, until its control stream ends.
    async fn listen(&self, id: MemberId, connection: &Connection, mut recv: RecvStream) {
        while let Ok(Some(message)) = control::read::<_, ClientMessage>(&mut recv).await {
            let ClientMessage::Mute { muted } = message else {
                tracing::debug!(id = id.0, "a second Hello or Watch, ignored");
                continue;
            };
            self.change(id, connection, |peer| peer.muted = muted);
        }
    }

    /// Shows the member reconnecting while nothing comes from it for [`SILENT_AFTER`], and back
    /// once something does again; the connection's end ends it.
    async fn notice_silence(&self, id: MemberId, connection: &Connection) {
        let mut check = tokio::time::interval(KEEP_ALIVE / 2);
        let (mut received, mut heard) = (connection.stats().udp_rx.datagrams, Instant::now());
        loop {
            check.tick().await;
            let now = connection.stats().udp_rx.datagrams;
            if now != received {
                (received, heard) = (now, Instant::now());
            }
            let silent = heard.elapsed() >= SILENT_AFTER;
            self.change(id, connection, |peer| peer.reconnecting = silent);
        }
    }

    /// Changes the member and tells everyone if that changed it, if it is still on `connection`;
    /// whether it is.
    fn change(&self, id: MemberId, connection: &Connection, change: impl FnOnce(&mut Peer)) -> bool {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        let Some(peer) = members.peers.get_mut(&id).filter(|peer| peer.is_on(connection)) else { return false };
        let before = peer.member(id);
        change(peer);
        let member = peer.member(id);
        if member != before {
            members.tell(&ServerMessage::Changed { member });
        }
        true
    }

    /// Removes the member, unless another connection took its place.
    fn leave(&self, id: MemberId, connection: &Connection) {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        if !members.peers.get(&id).is_some_and(|peer| peer.is_on(connection)) {
            return;
        }
        let peer = members.peers.remove(&id).expect("the member is in the channel");
        members.tell(&ServerMessage::Left { id });
        tracing::info!(id = id.0, device = peer.device, "left");
    }

    /// Forwards the member's audio to everyone else until its connection is lost.
    async fn relay(&self, from: MemberId, connection: &Connection) {
        while let Ok(datagram) = connection.read_datagram().await {
            let mut datagram = Vec::from(datagram);
            if !audio::stamp(&mut datagram, from) {
                continue;
            }
            let datagram = Bytes::from(datagram);
            let members = self.members.lock().expect("the members lock is never poisoned");
            for (_, peer) in members.peers.iter().filter(|&(&id, _)| id != from) {
                // A datagram a peer can't take now is a lost frame, which its jitter buffer conceals.
                let _ = peer.connection.send_datagram(datagram.clone());
            }
        }
    }
}

impl Members {
    fn everyone(&self) -> Vec<Member> {
        self.peers.iter().map(|(&id, peer)| peer.member(id)).collect()
    }

    /// Tells the members and the watchers who joined, changed or left.
    fn tell(&mut self, message: &ServerMessage) {
        for peer in self.peers.values() {
            // A peer whose outbox is gone is leaving and needs no news.
            let _ = peer.control.send(message.clone());
        }
        // A watcher whose outbox is gone closed its connection.
        self.watchers.retain(|watcher| watcher.send(message.clone()).is_ok());
    }

    /// The next id after the last one handed out that no member holds, skipping 0, which marks
    /// a client's own frames.
    fn next_id(&mut self) -> MemberId {
        loop {
            self.last_id = self.last_id.wrapping_add(1);
            let id = MemberId(self.last_id);
            if id.0 != 0 && !self.peers.contains_key(&id) {
                return id;
            }
        }
    }
}

/// What a connection asks for with its first message.
enum Asked {
    Join { muted: bool },
    Watch,
    Download(String),
}

/// A device's name as members see it: without control characters, which would let it rewrite
/// the terminals that show it, and cut to [`MAX_NAME_CHARS`].
fn shown(name: &str) -> String {
    let name: String = name.chars().filter(|char| !char.is_control()).collect();
    name.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// Tells the client why it was turned away, and closes the connection once it has read it.
pub(crate) async fn reject(mut send: SendStream, connection: &Connection, reason: Rejection) -> anyhow::Result<()> {
    control::write(&mut send, &ServerMessage::Rejected { reason }).await?;
    send.finish()?;
    // The client closes once it has read the rejection; one that doesn't is closed on.
    let _ = timeout(REJECTION_GRACE, connection.closed()).await;
    anyhow::bail!("rejected: {reason}");
}

/// Writes the control messages queued for a member or watcher until it is gone.
async fn deliver(mut send: SendStream, mut outbox: mpsc::UnboundedReceiver<ServerMessage>) {
    while let Some(message) = outbox.recv().await {
        if control::write(&mut send, &message).await.is_err() {
            return;
        }
    }
}

/// Reads the watcher's side of the control stream, which says nothing after its Watch, until the
/// watcher ends it.
async fn drain(mut recv: RecvStream) {
    while let Ok(Some(_)) = control::read::<_, ClientMessage>(&mut recv).await {}
}

/// Whether the connection ended without the member leaving: it timed out or broke. A member
/// leaves by closing it, or by ending its control stream, which leaves it open.
fn is_lost(connection: &Connection) -> bool {
    connection
        .close_reason()
        .is_some_and(|reason| !matches!(reason, ConnectionError::ApplicationClosed(_) | ConnectionError::LocallyClosed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_shown_without_control_characters_and_cut() {
        assert_eq!(shown(" First\u{1b}[2J Member "), "First[2J Member");
        assert_eq!(shown(&"long".repeat(10)).chars().count(), MAX_NAME_CHARS);
    }
}
