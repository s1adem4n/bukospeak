use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_proto::control::{self, ClientMessage, MAX_NAME_CHARS, Member, Rejection, ServerMessage};
use bukospeak_proto::{MemberId, PROTOCOL_VERSION, audio};
use bytes::Bytes;
use quinn::{Connection, Endpoint, Incoming, RecvStream, SendStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// More than a group of friends needs, and few enough that relaying to all stays cheap.
const MAX_MEMBERS: usize = 16;

/// How long a new connection has to say Hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a turned-away client has to read why before the server closes on it.
const REJECTION_GRACE: Duration = Duration::from_secs(1);

/// The one channel: who is in it, and the relay of their audio.
pub struct Room {
    room_key: String,
    members: Mutex<Members>,
}

#[derive(Default)]
struct Members {
    peers: BTreeMap<MemberId, Peer>,
    last_id: u16,
}

struct Peer {
    name: String,
    connection: Connection,
    control: mpsc::UnboundedSender<ServerMessage>,
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

    /// Lets a connection join once its Hello checks out, and relays for it until it leaves.
    async fn admit(&self, incoming: Incoming) -> anyhow::Result<()> {
        let connection = incoming.await?;
        let (mut send, mut recv) = timeout(HELLO_TIMEOUT, connection.accept_bi()).await??;
        let Some(ClientMessage::Hello { version, name, room_key }) = timeout(HELLO_TIMEOUT, control::read(&mut recv)).await?? else {
            anyhow::bail!("the stream ended before a Hello");
        };

        let name = name.trim();
        let (control, outbox) = mpsc::unbounded_channel();
        let joined = self.check(version, name, &room_key).and_then(|()| self.join(name, &connection, control));
        let id = match joined {
            Ok(id) => id,
            Err(reason) => {
                control::write(&mut send, &ServerMessage::Rejected { reason }).await?;
                send.finish()?;
                // The client closes once it has read the rejection; one that doesn't is closed on.
                let _ = timeout(REJECTION_GRACE, connection.closed()).await;
                anyhow::bail!("rejected: {reason}");
            }
        };
        tracing::info!(id = id.0, name, remote = %connection.remote_address(), "joined");

        tokio::select! {
            () = deliver(send, outbox) => {}
            () = self.relay(id, &connection) => {}
            () = drain(recv) => {}
        }
        self.leave(id);
        Ok(())
    }

    fn check(&self, version: u32, name: &str, room_key: &str) -> Result<(), Rejection> {
        if version != PROTOCOL_VERSION {
            return Err(Rejection::Version);
        }
        if room_key != self.room_key {
            return Err(Rejection::RoomKey);
        }
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(Rejection::Name);
        }
        Ok(())
    }

    /// Adds a member, welcomes it with everyone in the channel and tells the others.
    fn join(&self, name: &str, connection: &Connection, control: mpsc::UnboundedSender<ServerMessage>) -> Result<MemberId, Rejection> {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        if members.peers.len() >= MAX_MEMBERS {
            return Err(Rejection::Full);
        }
        let id = members.next_id();
        let member = Member { id, name: name.to_owned() };
        for peer in members.peers.values() {
            let _ = peer.control.send(ServerMessage::Joined { member: member.clone() });
        }

        let mut everyone: Vec<Member> = members.peers.iter().map(|(&id, peer)| Member { id, name: peer.name.clone() }).collect();
        everyone.push(member);
        // The channel is fresh and its receiver alive, so the Welcome is queued first.
        let _ = control.send(ServerMessage::Welcome { you: id, members: everyone });
        members.peers.insert(id, Peer { name: name.to_owned(), connection: connection.clone(), control });
        Ok(id)
    }

    fn leave(&self, id: MemberId) {
        let mut members = self.members.lock().expect("the members lock is never poisoned");
        let Some(peer) = members.peers.remove(&id) else { return };
        for other in members.peers.values() {
            let _ = other.control.send(ServerMessage::Left { id });
        }
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

/// Writes the control messages queued for a member until it is gone.
async fn deliver(mut send: SendStream, mut outbox: mpsc::UnboundedReceiver<ServerMessage>) {
    while let Some(message) = outbox.recv().await {
        if control::write(&mut send, &message).await.is_err() {
            return;
        }
    }
}

/// Reads the client's side of the control stream, which says nothing after its Hello yet, until
/// the client ends it.
async fn drain(mut recv: RecvStream) {
    while let Ok(Some(_)) = control::read::<_, ClientMessage>(&mut recv).await {}
}
