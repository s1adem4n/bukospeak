//! The engine the apps drive: joins and leaves the channel, runs the network side of a call,
//! joins again once when its connection is lost, and tells a [`Listener`] the channel after
//! each change.
//!
//! A join hands the caller the call's two realtime ends, a [`Microphone`] and a [`Speaker`],
//! for the platform's audio to drive until the call is over; they exist only for a call, and
//! last through joining again.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use bukospeak_proto::{Fingerprint, MemberId};
use rtrb::{Producer, RingBuffer};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::capture::Capture;
use crate::channel::{Announcement, Channel, Event};
use crate::conditioner::Conditioning;
use crate::jitter::Packet;
use crate::microphone::{Microphone, Settings};
use crate::playout::Mixer;
use crate::session::{self, AudioSender, Error, JoinConfig, Session};
use crate::speaker::{Downlink, Speaker};
use crate::speaking::Speaking;
use crate::uplink;

/// Frames the uplink may fall behind the microphone: 1 s.
const UPLINK_FRAMES: usize = 100;

/// Frames and leaves the speaker may fall behind the network: 0.5 s of every voice.
const DOWNLINK_FRAMES: usize = 50 * crate::playout::VOICES;

/// How often the engine looks who speaks: quick enough for an indicator to feel live.
const SPEAKING_EVERY: Duration = Duration::from_millis(50);

/// Receives the channel after each change, in order, on a thread of its own, so it may call
/// the engine; but not drop it, since dropping waits for that thread.
pub trait Listener: Send + Sync {
    fn update(&self, update: Update<'_>);
}

/// A change in the channel, and the channel after it.
pub struct Update<'a> {
    pub channel: &'a Channel,
    /// What to tell the user aloud about the change, if anything.
    pub announcement: Option<Announcement>,
    /// The call ended without a leave: why.
    pub lost: Option<Lost>,
}

/// Why a call ended without a leave: its connection was lost, and joining again failed.
#[derive(Debug, thiserror::Error)]
#[error("{connection}; joining again failed: {rejoin}")]
pub struct Lost {
    pub connection: Error,
    pub rejoin: Error,
}

/// A call just joined; who is in it comes as the next [`Update`].
pub struct Joined {
    /// The server's fingerprint, to pin for the next join.
    pub fingerprint: Fingerprint,
    pub microphone: Microphone,
    pub speaker: Speaker,
}

/// Joins and leaves the channel, one call at a time. Dropping it leaves the call and returns
/// once the listener has had its last update.
pub struct Engine {
    runtime: Runtime,
    conditioning: Conditioning,
    settings: Arc<Settings>,
    call: Mutex<Option<Call>>,
    // Fields drop in order: the last sender of events goes before the listener's thread, which
    // ends with it, is waited for.
    events: mpsc::Sender<Event>,
    _listening: Listening,
}

/// The listener's thread, waited for when dropped.
struct Listening(Option<thread::JoinHandle<()>>);

impl Drop for Listening {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            thread.join().expect("the listener doesn't panic");
        }
    }
}

struct Call {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Engine {
    /// An engine whose calls condition the microphone's signal as `conditioning` says.
    pub fn new(conditioning: Conditioning, listener: Arc<dyn Listener>) -> std::io::Result<Self> {
        // One worker runs the network side; a current-thread runtime would run it only while
        // a call into the engine blocks on it.
        let runtime =
            tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("bukospeak-network").enable_all().build()?;
        let (events, inbox) = mpsc::channel();
        let listening = thread::Builder::new().name("bukospeak-events".into()).spawn(move || {
            let mut channel = Channel::default();
            for event in inbox {
                let announcement = channel.apply(&event);
                let lost = match event {
                    Event::Lost(lost) => Some(lost),
                    _ => None,
                };
                listener.update(Update { channel: &channel, announcement, lost });
            }
        })?;
        Ok(Self { runtime, conditioning, settings: Arc::default(), call: Mutex::default(), events, _listening: Listening(Some(listening)) })
    }

    /// Joins the channel, after leaving the call before if there is one.
    pub fn join(&self, config: JoinConfig) -> Result<Joined, Error> {
        // Held until the call is in place, so a leave meanwhile, say from the listener's
        // thread, waits for the join and then ends its call instead of missing it.
        let mut call = self.call.lock().expect("the call lock is never poisoned");
        self.end(call.take());
        // libopus creates codecs without fail for the fixed formats asked for.
        let capture = Capture::new(self.conditioning).expect("the encoder can be created");
        let mixer = Mixer::new().expect("the decoders can be created");

        let session = self.runtime.block_on(Session::join(config.clone()))?;
        let fingerprint = session.fingerprint();
        // Joining again trusts only the server joined now.
        let rejoin = JoinConfig { pin: Some(fingerprint), ..config };
        let speaking = Arc::new(Speaking::default());
        let (frames, uplink_frames) = RingBuffer::new(UPLINK_FRAMES);
        let sender = session.audio_sender();
        let uplink = uplink::spawn(uplink_frames, sender.clone())?;
        let (downlink, speaker_downlink) = RingBuffer::new(DOWNLINK_FRAMES);

        // Sent before the call's own events, so the listener tells the calls apart. A failed
        // send means the engine is gone, and with it the listener.
        let _ = self.events.send(Event::Entered { you: session.you(), members: session.members().to_vec() });
        let (stop, stopped) = oneshot::channel();
        let link = Link { session, rejoin, sender };
        let task = self.runtime.spawn(run(link, Deliveries::new(downlink), Arc::clone(&speaking), self.events.clone(), stopped));
        *call = Some(Call { stop, task });
        Ok(Joined {
            fingerprint,
            microphone: Microphone::new(capture, frames, uplink, Arc::clone(&self.settings), Arc::clone(&speaking)),
            speaker: Speaker::new(mixer, speaker_downlink, speaking),
        })
    }

    /// Leaves the call, if there is one, once the server knows.
    pub fn leave(&self) {
        let mut call = self.call.lock().expect("the call lock is never poisoned");
        self.end(call.take());
    }

    /// Whether the microphone's noise is suppressed (the default), or sent as captured.
    pub fn set_noise_suppression(&self, on: bool) {
        self.settings.noise_suppression.store(on, Ordering::Relaxed);
    }

    /// Whether the microphone sends silence, so a talkspurt ends as in a pause.
    pub fn set_muted(&self, muted: bool) {
        self.settings.muted.store(muted, Ordering::Relaxed);
    }

    fn end(&self, call: Option<Call>) {
        let Some(call) = call else { return };
        // A call that ended by itself no longer listens.
        let _ = call.stop.send(());
        self.runtime.block_on(call.task).expect("the call task doesn't panic");
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.leave();
    }
}

/// A call's way to the channel: its session, and what joins it again.
struct Link {
    session: Session,
    /// The join, pinned to the server's fingerprint.
    rejoin: JoinConfig,
    /// The uplink's, which follows the session joined again.
    sender: AudioSender,
}

/// The network side of a call: moves the members' frames to the speaker and tells the
/// listener who comes, goes and speaks, until it is stopped, or the connection is lost and
/// joining again fails.
async fn run(
    mut link: Link,
    mut deliveries: Deliveries,
    speaking: Arc<Speaking>,
    events: mpsc::Sender<Event>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut you = link.session.you();
    let mut members = others(&link.session);
    let mut speakers = Vec::new();
    let mut watch = tokio::time::interval(SPEAKING_EVERY);
    watch.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let start = Instant::now();
    // A failed send means the engine is gone, and with it the listener; the call ends soon.
    let tell = |event| _ = events.send(event);

    loop {
        let lost = tokio::select! {
            _ = &mut stop => break,
            _ = watch.tick() => {
                deliveries.retry();
                let now = speaking.members(you);
                if now != speakers {
                    speakers.clone_from(&now);
                    tell(Event::Speaking(now));
                }
                continue;
            }
            event = link.session.next_event() => match event {
                Ok(session::Event::Audio(frame)) => {
                    // Frames of a member that left, still on the way, would take a voice for good.
                    if members.contains(&frame.from)
                        && let Some(packet) = Packet::new(frame.index, frame.last, frame.level, start.elapsed().as_micros() as u64, &frame.opus)
                    {
                        deliveries.frame(frame.from, packet);
                    }
                    continue;
                }
                Ok(session::Event::Joined(member)) => {
                    members.insert(member.id);
                    tell(Event::Joined(member));
                    continue;
                }
                Ok(session::Event::Left(id)) => {
                    members.remove(&id);
                    deliveries.left(id);
                    tell(Event::Left(id));
                    continue;
                }
                Err(error) => error,
            },
        };

        tell(Event::Reconnecting);
        let rejoined = tokio::select! {
            _ = &mut stop => return tell(Event::Ended),
            rejoined = Session::join(link.rejoin.clone()) => rejoined,
        };
        match rejoined {
            Ok(session) => {
                // The members of the lost session are gone, even if they joined again too.
                members.iter().for_each(|&id| deliveries.left(id));
                link.sender.follow(&session);
                link.session = session;
                you = link.session.you();
                members = others(&link.session);
                speakers.clear();
                tell(Event::Entered { you, members: link.session.members().to_vec() });
            }
            Err(rejoin) => return tell(Event::Lost(Lost { connection: lost, rejoin })),
        }
    }
    link.session.leave().await;
    tell(Event::Ended);
}

/// Everyone in the session's channel but this member.
fn others(session: &Session) -> BTreeSet<MemberId> {
    session.members().iter().map(|member| member.id).filter(|&id| id != session.you()).collect()
}

/// The network task's end of the ring to the speaker.
struct Deliveries {
    ring: Producer<Downlink>,
    /// Leaves the full ring didn't take yet: unlike a frame, a leave can't be lost, or its
    /// member's voice stays taken.
    lefts: VecDeque<MemberId>,
}

impl Deliveries {
    fn new(ring: Producer<Downlink>) -> Self {
        Self { ring, lefts: VecDeque::new() }
    }

    fn frame(&mut self, from: MemberId, packet: Packet) {
        self.retry();
        // A full ring means the speaker stopped playing: the frame would be too late anyway.
        let _ = self.ring.push(Downlink::Frame(from, packet));
    }

    fn left(&mut self, member: MemberId) {
        self.lefts.push_back(member);
        self.retry();
    }

    fn retry(&mut self) {
        while let Some(&member) = self.lefts.front()
            && self.ring.push(Downlink::Left(member)).is_ok()
        {
            self.lefts.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Instant;

    use bukospeak_proto::audio::{Level, SAMPLE_RATE};
    use bukospeak_server::{Identity, Room};
    use rtrb::Consumer;
    use tokio::net::UdpSocket;

    use super::*;
    use crate::channel::State;
    use crate::signal::voice;

    const ROOM_KEY: &str = "room-key";
    const FRAME: Duration = Duration::from_millis(10);

    /// An update as the test keeps it.
    #[derive(Debug)]
    struct Seen {
        channel: Channel,
        announcement: Option<Announcement>,
        lost: Option<Lost>,
    }

    struct Updates(mpsc::Sender<Seen>);

    impl Listener for Updates {
        fn update(&self, update: Update<'_>) {
            let Update { channel, announcement, lost } = update;
            // The test may have stopped listening.
            let _ = self.0.send(Seen { channel: channel.clone(), announcement, lost });
        }
    }

    fn engine() -> (Engine, mpsc::Receiver<Seen>) {
        let (updates, received) = mpsc::channel();
        (Engine::new(Conditioning::Engine, Arc::new(Updates(updates))).unwrap(), received)
    }

    /// Waits for the first update that `wanted` picks.
    fn wait_for<T>(updates: &mpsc::Receiver<Seen>, mut wanted: impl FnMut(Seen) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let seen = updates.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("the update within 10 s");
            if let Some(found) = wanted(seen) {
                return found;
            }
        }
    }

    fn announced(updates: &mpsc::Receiver<Seen>, announcement: Announcement) -> Channel {
        wait_for(updates, |seen| (seen.announcement.as_ref() == Some(&announcement)).then_some(seen.channel))
    }

    /// Who is in the channel the engine entered, as `(you, names)`.
    fn entered(updates: &mpsc::Receiver<Seen>) -> (MemberId, Vec<String>) {
        wait_for(updates, |seen| {
            let names = seen.channel.members().iter().map(|member| member.name.clone()).collect();
            seen.channel.you().map(|you| (you, names))
        })
    }

    /// A room on a new endpoint on `address` that presents the identity.
    fn serve(runtime: &Runtime, identity: &Identity, address: &str) -> quinn::Endpoint {
        let endpoint = runtime.block_on(async { bukospeak_server::endpoint(identity, address.parse().unwrap()) }).unwrap();
        runtime.spawn(Room::new(ROOM_KEY.into()).serve(endpoint.clone()));
        endpoint
    }

    fn config(server: SocketAddr, identity: &Identity, name: &str) -> JoinConfig {
        JoinConfig { server: server.to_string(), name: name.into(), room_key: ROOM_KEY.into(), pin: Some(identity.fingerprint()) }
    }

    /// A UDP relay in front of a server that can turn to another, as a server's address does
    /// when the server restarts elsewhere.
    struct Relay {
        address: SocketAddr,
        upstream: Arc<Mutex<SocketAddr>>,
    }

    impl Relay {
        fn new(runtime: &Runtime, upstream: SocketAddr) -> Self {
            let bind = || async { Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()) };
            let (front, back) = runtime.block_on(async { (bind().await, bind().await) });
            let address = front.local_addr().unwrap();
            let upstream = Arc::new(Mutex::new(upstream));
            let client = Arc::new(Mutex::new(None));
            // Datagrams the relay can't pass on are lost, as on any path.
            runtime.spawn({
                let (front, back, upstream, client) = (Arc::clone(&front), Arc::clone(&back), Arc::clone(&upstream), Arc::clone(&client));
                async move {
                    let mut datagram = [0; 65536];
                    while let Ok((length, from)) = front.recv_from(&mut datagram).await {
                        *client.lock().unwrap() = Some(from);
                        let to = *upstream.lock().unwrap();
                        let _ = back.send_to(&datagram[..length], to).await;
                    }
                }
            });
            runtime.spawn(async move {
                let mut datagram = [0; 65536];
                while let Ok((length, _)) = back.recv_from(&mut datagram).await {
                    let to = *client.lock().unwrap();
                    if let Some(to) = to {
                        let _ = front.send_to(&datagram[..length], to).await;
                    }
                }
            });
            Self { address, upstream }
        }

        fn turn_to(&self, upstream: SocketAddr) {
            *self.upstream.lock().unwrap() = upstream;
        }
    }

    #[test]
    fn members_hear_each_other_and_see_who_speaks() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity, "127.0.0.1:0").local_addr().unwrap();

        let (first, first_updates) = engine();
        let (second, second_updates) = engine();
        let mut talking = first.join(config(address, &identity, "first")).unwrap();
        let (talking_id, _) = entered(&first_updates);
        let mut hearing = second.join(config(address, &identity, "second")).unwrap();
        let (hearing_id, names) = entered(&second_updates);
        assert_eq!(names, ["first", "second"]);
        // The first call's updates come after its entry.
        let channel = announced(&first_updates, Announcement::Joined("second".into()));
        assert_eq!(channel.members().last().unwrap().id, hearing_id);

        // Two seconds of speech and one of silence, captured and played in real time, in
        // buffers that don't line up with frames.
        let signal = [voice(2 * SAMPLE_RATE as usize, 0.3), vec![0.0; SAMPLE_RATE as usize]].concat();
        let mut heard = Vec::new();
        let mut next = Instant::now();
        for (step, captured) in signal.chunks(480).enumerate() {
            let (first_half, second_half) = captured.split_at(if step % 2 == 0 { 441 } else { 39 });
            talking.microphone.push(first_half);
            talking.microphone.push(second_half);
            let mut played = vec![0.0; if step % 2 == 0 { 400 } else { 560 }];
            hearing.speaker.pull(&mut played);
            heard.extend(played);
            next += FRAME;
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }

        let second_of_speech = &heard[SAMPLE_RATE as usize / 2..][..SAMPLE_RATE as usize];
        let rms = (second_of_speech.iter().map(|sample| sample * sample).sum::<f32>() / second_of_speech.len() as f32).sqrt();
        assert!(rms > 0.03, "heard speech at an RMS of {rms}");
        let speaking = wait_for(&second_updates, |seen| (!seen.channel.speaking().is_empty()).then(|| seen.channel.speaking().to_vec()));
        assert_eq!(speaking, [talking_id]);
        wait_for(&second_updates, |seen| seen.channel.speaking().is_empty().then_some(()));

        first.leave();
        let channel = announced(&second_updates, Announcement::Left("first".into()));
        assert!(channel.members().iter().all(|member| member.id != talking_id));
        let ended = wait_for(&first_updates, |seen| (seen.channel.state() == State::Out).then_some(seen.lost));
        assert!(ended.is_none());
    }

    #[test]
    fn a_lost_connection_joins_again_and_keeps_the_microphone() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let old = serve(&server, &identity, "127.0.0.1:0");
        let new = serve(&server, &identity, "127.0.0.1:0");
        let relay = Relay::new(&server, old.local_addr().unwrap());

        let (engine, updates) = engine();
        let mut joined = engine.join(config(relay.address, &identity, "first")).unwrap();
        entered(&updates);
        relay.turn_to(new.local_addr().unwrap());
        old.close(0u32.into(), b"restarting");

        let reconnecting = announced(&updates, Announcement::Lost);
        assert_eq!(reconnecting.state(), State::Reconnecting);
        let back = announced(&updates, Announcement::Reconnected);
        assert_eq!(back.state(), State::Connected);
        let you = back.you().unwrap();

        // The microphone of the first join sends over the second.
        let mut listener = server.block_on(Session::join(config(new.local_addr().unwrap(), &identity, "listener"))).unwrap();
        joined.microphone.push(&voice(SAMPLE_RATE as usize, 0.3));
        let from = server.block_on(async {
            loop {
                match tokio::time::timeout(Duration::from_secs(5), listener.next_event()).await.expect("audio within 5 s").unwrap() {
                    session::Event::Audio(frame) => break frame.from,
                    _ => continue,
                }
            }
        });
        assert_eq!(from, you);
    }

    #[test]
    fn a_lost_connection_that_cant_join_again_ends_the_call() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let old = serve(&server, &identity, "127.0.0.1:0");
        let relay = Relay::new(&server, old.local_addr().unwrap());
        // A socket nobody answers on.
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let (engine, updates) = engine();
        let _joined = engine.join(config(relay.address, &identity, "first")).unwrap();
        entered(&updates);
        relay.turn_to(silent.local_addr().unwrap());
        old.close(0u32.into(), b"gone");

        announced(&updates, Announcement::Lost);
        let (channel, lost) = wait_for(&updates, |seen| seen.lost.map(|lost| (seen.channel, lost)));
        assert_eq!(channel.state(), State::Out);
        assert!(matches!(lost.rejoin, Error::TimedOut), "{lost}");
    }

    #[test]
    fn a_leave_ends_joining_again() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let old = serve(&server, &identity, "127.0.0.1:0");
        let relay = Relay::new(&server, old.local_addr().unwrap());
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();

        let (engine, updates) = engine();
        let _joined = engine.join(config(relay.address, &identity, "first")).unwrap();
        entered(&updates);
        relay.turn_to(silent.local_addr().unwrap());
        old.close(0u32.into(), b"gone");
        announced(&updates, Announcement::Lost);

        let leaving = Instant::now();
        engine.leave();
        assert!(leaving.elapsed() < Duration::from_secs(1), "the leave took {:?}", leaving.elapsed());
        let ended = wait_for(&updates, |seen| (seen.channel.state() == State::Out).then_some(seen.lost));
        assert!(ended.is_none());
    }

    /// What the speaker takes from the ring now, as `(was a leave, member)`.
    fn take(ring: &mut Consumer<Downlink>) -> Vec<(bool, MemberId)> {
        std::iter::from_fn(|| ring.pop().ok())
            .map(|delivered| match delivered {
                Downlink::Frame(from, _) => (false, from),
                Downlink::Left(member) => (true, member),
            })
            .collect()
    }

    #[test]
    fn leaves_the_full_ring_refuses_wait_and_arrive_in_order_once_it_has_room() {
        let (ring, mut speaker) = RingBuffer::new(2);
        let mut deliveries = Deliveries::new(ring);
        let packet = || Packet::new(0, false, Level::new(20), 0, &[0]).unwrap();
        let (first, second, third) = (MemberId(1), MemberId(2), MemberId(3));

        deliveries.frame(first, packet());
        deliveries.frame(second, packet());
        deliveries.left(first);
        deliveries.left(second);
        // A frame the full ring refuses is lost.
        deliveries.frame(third, packet());
        assert_eq!(take(&mut speaker), [(false, first), (false, second)]);

        deliveries.retry();
        assert_eq!(take(&mut speaker), [(true, first), (true, second)]);
        deliveries.frame(third, packet());
        assert_eq!(take(&mut speaker), [(false, third)]);
    }

    #[test]
    fn dropping_waits_for_the_listener() {
        let (engine, updates) = engine();
        drop(engine);
        // The listener, holding the only sender, is gone with its thread.
        assert_eq!(updates.try_recv().unwrap_err(), mpsc::TryRecvError::Disconnected);
    }
}
