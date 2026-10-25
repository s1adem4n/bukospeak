use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_proto::control::{self, ClientMessage, MAX_NAME_CHARS, Member, Rejection, ResumeToken, ServerMessage};
use bukospeak_proto::{MemberId, PROTOCOL_VERSION, audio};
use bytes::Bytes;
use quinn::{Connection, ConnectionError, Endpoint, Incoming, RecvStream, SendStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// More than a group of friends needs, and few enough that relaying to all stays cheap.
const MAX_MEMBERS: usize = 16;

/// How long a new connection has to say Hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a turned-away client has to read why before the server closes on it.
const REJECTION_GRACE: Duration = Duration::from_secs(1);

/// How long the place of a member whose connection was lost is held for it to come back: the
/// member notices the loss about when the server does, and then has a join's time.
const RESUME_GRACE: Duration = Duration::from_secs(20);

/// The one channel: who is in it, and the relay of their audio.
pub struct Room {
    room_key: String,
    members: Mutex<Members>,
}

#[derive(Default)]
struct Members {
    peers: BTreeMap<MemberId, Peer>,
    /// The outboxes of the clients that watch the channel without joining it.
    watchers: Vec<mpsc::UnboundedSender<ServerMessage>>,
    last_id: u16,
}

struct Peer {
    name: String,
    muted: bool,
    reconnecting: bool,
    resume: ResumeToken,
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

/// What a Hello asks for beyond a name.
struct Hello {
    muted: bool,
    resume: Option<ResumeToken>,
}

impl Room {
    pub fn new(room_key: String) -> Arc<Self> {
        Arc::new(Self { room_key, members: Mutex::default() })
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
    /// lost and not resumed; or once its Watch checks out, tells it who is in the channel until
    /// it closes.
    async fn admit(&self, incoming: Incoming) -> anyhow::Result<()> {
        let connection = incoming.await?;
        let (send, recv, first) = timeout(HELLO_TIMEOUT, async {
            let (send, mut recv) = connection.accept_bi().await?;
            let first = control::read(&mut recv).await?;
            anyhow::Ok((send, recv, first))
        })
        .await??;
        let (control, outbox) = mpsc::unbounded_channel();
        match first {
            Some(ClientMessage::Hello { version, name, room_key, muted, resume }) => {
                let name = name.trim();
                let joined = self
                    .check(version, &room_key)
                    .and_then(|()| check_name(name))
                    .and_then(|()| self.join(name, Hello { muted, resume }, &connection, control));
                let id = match joined {
                    Ok(id) => id,
                    Err(reason) => return reject(send, &connection, reason).await,
                };
                tracing::info!(id = id.0, name, remote = %connection.remote_address(), "joined");
                tokio::select! {
                    () = deliver(send, outbox) => {}
                    () = self.relay(id, &connection) => {}
                    () = self.listen(id, &connection, recv) => {}
                }
                if is_lost(&connection) && self.reconnecting(id, &connection) {
                    tokio::time::sleep(RESUME_GRACE).await;
                }
                self.leave(id, &connection);
            }
            Some(ClientMessage::Watch { version, room_key }) => {
                if let Err(reason) = self.check(version, &room_key) {
                    return reject(send, &connection, reason).await;
                }
                self.watch(control);
                tracing::debug!(remote = %connection.remote_address(), "watching");
                tokio::select! {
                    () = deliver(send, outbox) => {}
                    () = drain(recv) => {}
                }
            }
            Some(ClientMessage::Mute { .. }) => anyhow::bail!("a Mute before a Hello"),
            None => anyhow::bail!("the stream ended before a Hello or a Watch"),
        }
        Ok(())
    }

    fn check(&self, version: u32, room_key: &str) -> Result<(), Rejection> {
        if version != PROTOCOL_VERSION {
            return Err(Rejection::Version);
        }
        if room_key != self.room_key {
            return Err(Rejection::RoomKey);
        }
        Ok(())
    }

    /// Adds a member, or puts the one the Hello resumes on the connection, closing its old
    /// one; welcomes it with everyone in the channel and tells the others.
    fn join(
        &self,
        name: &str,
        hello: Hello,
        connection: &Connection,
        control: mpsc::UnboundedSender<ServerMessage>,
    ) -> Result<MemberId, Rejection> {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        let resumed = hello.resume.and_then(|token| members.peers.iter().find(|(_, peer)| peer.resume == token).map(|(&id, _)| id));
        let id = match resumed {
            Some(id) => {
                let peer = members.peers.get_mut(&id).expect("the resumed member is in the channel");
                peer.reconnecting = false;
                peer.muted = hello.muted;
                let member = peer.member(id);
                // Told before the member is on the new connection, whose Welcome comes first.
                members.tell(&ServerMessage::Changed { member });
                let peer = members.peers.get_mut(&id).expect("the resumed member is in the channel");
                peer.connection.close(0u32.into(), b"resumed");
                peer.connection = connection.clone();
                peer.control = control.clone();
                id
            }
            None => {
                if members.peers.len() >= MAX_MEMBERS {
                    return Err(Rejection::Full);
                }
                let id = members.next_id();
                let peer = Peer {
                    name: name.to_owned(),
                    muted: hello.muted,
                    reconnecting: false,
                    resume: ResumeToken(random()),
                    connection: connection.clone(),
                    control: control.clone(),
                };
                members.tell(&ServerMessage::Joined { member: peer.member(id) });
                members.peers.insert(id, peer);
                id
            }
        };
        let resume = members.peers[&id].resume;
        // The channel is fresh and its receiver alive, so the Welcome is queued first.
        let _ = control.send(ServerMessage::Welcome { you: id, members: members.everyone(), resume });
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

    /// Shows the member as reconnecting while its place is held, if it is still on the lost
    /// `connection`; whether it is.
    fn reconnecting(&self, id: MemberId, connection: &Connection) -> bool {
        let changed = self.change(id, connection, |peer| peer.reconnecting = true);
        if changed {
            tracing::info!(id = id.0, "reconnecting");
        }
        changed
    }

    /// Changes the member and tells everyone, if it is still on `connection`; whether it is.
    fn change(&self, id: MemberId, connection: &Connection, change: impl FnOnce(&mut Peer)) -> bool {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        let Some(peer) = members.peers.get_mut(&id).filter(|peer| peer.is_on(connection)) else { return false };
        change(peer);
        let member = peer.member(id);
        members.tell(&ServerMessage::Changed { member });
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
        tracing::info!(id = id.0, name = peer.name, "left");
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

/// Control characters would let a name rewrite the terminals and logs that show it.
fn check_name(name: &str) -> Result<(), Rejection> {
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(Rejection::Name);
    }
    Ok(())
}

/// Tells the client why it was turned away, and closes the connection once it has read it.
async fn reject(mut send: SendStream, connection: &Connection, reason: Rejection) -> anyhow::Result<()> {
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

/// Bytes from the system's secure randomness.
fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    rustls::crypto::ring::default_provider().secure_random.fill(&mut bytes).expect("the system has secure randomness");
    bytes
}
