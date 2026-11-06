//! The engine the apps drive: joins and leaves the channel, runs the network side of a call,
//! joins again once when its connection is lost, watches who is in the channel while out of it,
//! and tells a [`Listener`] the channel after each change.
//!
//! A join hands the caller the call's two realtime ends, a [`Microphone`] and a [`Speaker`],
//! for the platform's audio to drive until the call is over; they exist only for a call, and
//! last through joining again.

use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use bukospeak_proto::{Fingerprint, MemberId, SILENT_AFTER};
use rtrb::{Producer, RingBuffer};
use tokio::runtime::Runtime;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::capture::Capture;
use crate::channel::{Announcement, Channel, Event, Lost};
use crate::conditioner::Conditioning;
use crate::jitter::Packet;
use crate::microphone::{Microphone, Settings};
use crate::playout::Mixer;
use crate::session::{self, AudioSender, Entry, Error, JoinConfig, Session, Watch};
use crate::speaker::{Downlink, Speaker};
use crate::speaking::Speaking;
use crate::suppression::{Stage, Suppression};
use crate::uplink::{self, Made};

/// The loudest [`Engine::set_volume`] plays the others: 6 dB above their level.
pub const MAX_VOLUME: f32 = 2.0;

/// Frames the uplink may fall behind the microphone: 1 s.
const UPLINK_FRAMES: usize = 100;

/// Frames and leaves the speaker may fall behind the network: 0.5 s of every voice.
const DOWNLINK_FRAMES: usize = 50 * crate::playout::VOICES;

/// How often the engine looks who speaks and whether the server went silent: quick enough for
/// an indicator to feel live.
const LOOK_EVERY: Duration = Duration::from_millis(50);

/// How long a watch that failed or broke waits before it starts again.
const WATCH_AGAIN_AFTER: Duration = Duration::from_secs(5);

/// How long a join again that failed waits before it tries anew.
const REJOIN_AGAIN_AFTER: Duration = Duration::from_millis(250);

/// Receives the channel after each change, in order, on a thread of its own, so it may call
/// the engine; but not drop it, since dropping waits for that thread.
pub trait Listener: Send + Sync {
    fn update(&self, update: Update);
}

/// A change in the channel, and the channel after it.
#[derive(Debug)]
pub struct Update {
    pub channel: Channel,
    /// What to tell the user aloud about the change, if anything.
    pub announcement: Option<Announcement>,
    /// The call ended without a leave: why.
    pub lost: Option<Lost>,
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
    /// Whether this member is muted, for the call to tell the server; the microphone goes by
    /// the settings.
    muted: watch::Sender<bool>,
    calls: Mutex<Calls>,
    watch: Mutex<Option<Running>>,
    suppressing: Mutex<Suppressing>,
    /// How many times a suppression was chosen: the stages made for an earlier choice are
    /// dropped.
    chosen: Arc<AtomicU64>,
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

/// The engine's calls: how many it joined, and the one running, if any.
#[derive(Default)]
struct Calls {
    joined: u64,
    running: Option<Running>,
}

/// The suppression chosen, and the call's uplink, which takes the stages made for it.
struct Suppressing {
    suppression: Suppression,
    uplink: Option<mpsc::Sender<Made>>,
}

impl Suppressing {
    /// Makes the stage of the `choice`-th suppression for the call's uplink, if there is a call,
    /// on a thread of its own: loading a plugged-in suppressor may take seconds.
    fn make(&self, choice: u64) -> io::Result<()> {
        let Some(uplink) = self.uplink.clone() else { return Ok(()) };
        let suppression = self.suppression.clone();
        thread::Builder::new().name("bukospeak-suppression".into()).spawn(move || {
            let stage = Stage::new(&suppression);
            // The call may be over by now, and its uplink gone with it.
            let _ = uplink.send(Made { choice, stage });
        })?;
        Ok(())
    }
}

/// A call or a watch, on the runtime until it is stopped.
struct Running {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Engine {
    /// An engine whose calls condition the microphone's signal as `conditioning` says.
    pub fn new(conditioning: Conditioning, listener: Arc<dyn Listener>) -> io::Result<Self> {
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
                listener.update(Update { channel: channel.clone(), announcement, lost });
            }
        })?;
        Ok(Self {
            runtime,
            conditioning,
            settings: Arc::default(),
            muted: watch::Sender::new(false),
            calls: Mutex::default(),
            watch: Mutex::default(),
            suppressing: Mutex::new(Suppressing { suppression: Suppression::built_in(), uplink: None }),
            chosen: Arc::default(),
            events,
            _listening: Listening(Some(listening)),
        })
    }

    /// Joins the channel, after leaving the call before if there is one, and stops watching it.
    pub fn join(&self, config: JoinConfig) -> Result<Joined, Error> {
        // Held until the call is in place, so a leave meanwhile, say from the listener's
        // thread, waits for the join and then ends its call instead of missing it.
        let mut calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.end(calls.running.take());
        self.unwatch();
        let conditioning = self.conditioning;
        let (suppression, choice) = {
            let mut suppressing = self.suppressing();
            suppressing.uplink = None;
            (suppressing.suppression.clone(), self.chosen.load(Ordering::Relaxed))
        };
        // Built while the session joins, which hides the time a plugged-in suppressor takes to load.
        let capture = thread::Builder::new().name("bukospeak-capture".into()).spawn(move || capture(conditioning, &suppression))?;
        // libopus creates codecs without fail for the fixed formats asked for.
        let mixer = Mixer::new().expect("the decoders can be created");

        let mut muted = self.muted.subscribe();
        let entry = Entry { muted: *muted.borrow_and_update(), resume: None };
        let session = self.runtime.block_on(Session::join(config.clone(), entry))?;
        calls.joined += 1;
        let fingerprint = session.fingerprint();
        // Joining again trusts only the server joined now.
        let rejoin = JoinConfig { pin: Some(fingerprint), ..config };
        let capture = capture.join().expect("building the capture doesn't panic");
        let speaking = Arc::new(Speaking::default());
        let (frames, uplink_frames) = RingBuffer::new(UPLINK_FRAMES);
        let sender = session.audio_sender();
        let (made, uplink_made) = mpsc::channel();
        let uplink = uplink::spawn(uplink_frames, capture, uplink_made, Arc::clone(&self.chosen), sender.clone(), Arc::clone(&speaking))?;
        {
            let mut suppressing = self.suppressing();
            suppressing.uplink = Some(made);
            // Chosen anew while the call joined.
            let latest = self.chosen.load(Ordering::Relaxed);
            if latest != choice {
                suppressing.make(latest)?;
            }
        }
        let (microphone, tap) = Microphone::new(self.conditioning, frames, uplink, Arc::clone(&self.settings));
        let (downlink, speaker_downlink) = RingBuffer::new(DOWNLINK_FRAMES);

        // Sent before the call's own events, so they apply to the channel it enters. A failed
        // send means the engine is gone, and with it the listener.
        let _ = self.events.send(Event::Entered { call: calls.joined, you: session.you(), members: session.members().to_vec() });
        let (stop, stopped) = oneshot::channel();
        let link = Link { call: calls.joined, session, rejoin, sender, muted };
        let task = self.runtime.spawn(run(link, Deliveries::new(downlink), Arc::clone(&speaking), self.events.clone(), stopped));
        calls.running = Some(Running { stop, task });
        Ok(Joined { fingerprint, microphone, speaker: Speaker::new(mixer, speaker_downlink, speaking, Arc::clone(&self.settings), tap) })
    }

    /// Leaves the call, if there is one, once the server knows. Returns the
    /// [`call`](Channel::call) the engine joined last: updates of it or of a call before that are
    /// still on their way to the listener are of a call over now.
    pub fn leave(&self) -> u64 {
        let mut calls = self.calls.lock().expect("the calls lock is never poisoned");
        self.end(calls.running.take());
        self.suppressing().uplink = None;
        calls.joined
    }

    /// Watches who is in the channel without joining it, until [`unwatch`](Self::unwatch) or a
    /// join; the listener sees them as the channel's [`present`](Channel::present). The server
    /// is trusted by `pin` only, from a join before.
    pub fn watch(&self, server: String, room_key: String, pin: Fingerprint) {
        let mut watch = self.watch.lock().expect("the watch lock is never poisoned");
        self.end(watch.take());
        let (stop, stopped) = oneshot::channel();
        let task = self.runtime.spawn(watch_present(server, room_key, pin, self.events.clone(), stopped));
        *watch = Some(Running { stop, task });
    }

    /// Stops watching who is in the channel, if the engine does.
    pub fn unwatch(&self) {
        let mut watch = self.watch.lock().expect("the watch lock is never poisoned");
        self.end(watch.take());
    }

    /// How the microphone's noise is suppressed: by the best suppression built in unless set
    /// otherwise. In a call, the suppression before goes on until the new one is ready; a
    /// plugged-in suppressor takes what it takes to load.
    pub fn set_suppression(&self, suppression: Suppression) -> io::Result<()> {
        let mut suppressing = self.suppressing();
        suppressing.suppression = suppression;
        let choice = self.chosen.fetch_add(1, Ordering::Relaxed) + 1;
        suppressing.make(choice)
    }

    fn suppressing(&self) -> std::sync::MutexGuard<'_, Suppressing> {
        self.suppressing.lock().expect("the suppressing lock is never poisoned")
    }

    /// Whether the speaker's echo is cancelled in what the microphone hears (the default), where
    /// the engine conditions the signal. A headset doesn't echo, and AEC3 then leaves the voice
    /// as it is within seconds; off, it doesn't touch it at all.
    pub fn set_echo_cancellation(&self, on: bool) {
        self.settings.echo_cancellation.store(on, Ordering::Relaxed);
    }

    /// How loud the others play: 1.0 as they come, up to [`MAX_VOLUME`]; the limiter bends the
    /// peaks a louder mix would clip. It holds across calls until set again.
    pub fn set_volume(&self, volume: f32) {
        // A NaN would silence the mix for good.
        let volume = if volume.is_nan() { 1.0 } else { volume.clamp(0.0, MAX_VOLUME) };
        self.settings.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    /// Whether this member is muted: the microphone sends silence, so a talkspurt ends as in a
    /// pause, and the others see it muted. It holds across calls until set again.
    pub fn set_muted(&self, muted: bool) {
        self.settings.muted.store(muted, Ordering::Relaxed);
        self.muted.send_replace(muted);
    }

    fn end(&self, running: Option<Running>) {
        let Some(running) = running else { return };
        // A call that ended by itself, or a watch, may no longer listen.
        let _ = running.stop.send(());
        self.runtime.block_on(running.task).expect("a call or watch task doesn't panic");
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.unwatch();
        self.leave();
    }
}

/// A call's way to the channel: its session, and what joins it again.
struct Link {
    /// Which of the engine's joins the call is.
    call: u64,
    session: Session,
    /// The join, pinned to the server's fingerprint.
    rejoin: JoinConfig,
    /// The uplink's, which follows the session joined again.
    sender: AudioSender,
    /// Whether this member is muted, to tell the server.
    muted: watch::Receiver<bool>,
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
    let mut look = tokio::time::interval(LOOK_EVERY);
    look.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let start = Instant::now();
    // When something last came from the server, by the count of what came then.
    let (mut received, mut heard) = (link.session.received(), Instant::now());
    let mut silent = false;
    // A failed send means the engine is gone, and with it the listener; the call ends soon.
    let tell = |event| _ = events.send(event);

    loop {
        let lost = tokio::select! {
            _ = &mut stop => break,
            Ok(()) = link.muted.changed() => {
                let muted = *link.muted.borrow_and_update();
                match link.session.set_muted(muted).await {
                    Ok(()) => continue,
                    Err(error) => error,
                }
            }
            _ = look.tick() => {
                deliveries.retry();
                let count = link.session.received();
                if count != received {
                    (received, heard) = (count, Instant::now());
                }
                if silent != (heard.elapsed() >= SILENT_AFTER) {
                    silent = !silent;
                    speakers.clear();
                    tell(if silent { Event::Reconnecting } else { Event::Heard });
                }
                let mut now = speaking.members(you);
                // Voices of members that left, or were in the lost session, may still play out.
                now.retain(|id| *id == you || members.contains(id));
                // Cut off from the others, nobody shows speaking.
                if !silent && now != speakers {
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
                Ok(session::Event::Changed(member)) => {
                    tell(Event::Changed(member));
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
            // In its own place, if the server still holds it: the others see it come back, not
            // leave and join.
            rejoined = rejoin(
                &link.rejoin,
                Entry { muted: *link.muted.borrow_and_update(), resume: Some(link.session.resume()) },
            ) => rejoined,
        };
        match rejoined {
            Ok(session) => {
                // The voices of the lost session broke off; ones that go on start anew.
                members.iter().for_each(|&id| deliveries.left(id));
                link.sender.follow(&session);
                link.session = session;
                you = link.session.you();
                members = others(&link.session);
                speakers.clear();
                (received, heard, silent) = (link.session.received(), Instant::now(), false);
                tell(Event::Entered { call: link.call, you, members: link.session.members().to_vec() });
            }
            Err(rejoin) => return tell(Event::Lost(Lost { connection: lost, rejoin })),
        }
    }
    link.session.leave().await;
    tell(Event::Ended);
}

/// Joins again after the connection was lost, trying anew for up to [`session::JOIN_TIMEOUT`]
/// while the server can't take the join yet: one that stops closes its connections, and turns
/// joins away until it is gone and up again.
async fn rejoin(config: &JoinConfig, entry: Entry) -> Result<Session, Error> {
    let deadline = Instant::now() + session::JOIN_TIMEOUT;
    loop {
        match tokio::time::timeout_at(deadline, Session::join(config.clone(), entry)).await {
            Ok(Ok(session)) => return Ok(session),
            // Answers that trying again won't change.
            Ok(Err(error @ (Error::FingerprintChanged { .. } | Error::Rejected(_)))) => return Err(error),
            Ok(Err(error)) if Instant::now() + REJOIN_AGAIN_AFTER >= deadline => return Err(error),
            Ok(Err(_)) => tokio::time::sleep(REJOIN_AGAIN_AFTER).await,
            Err(_) => return Err(Error::TimedOut),
        }
    }
}

/// Tells the listener who is in the channel, watched from outside, until it is stopped; a watch
/// that fails or breaks shows nobody and starts again after a while.
async fn watch_present(server: String, room_key: String, pin: Fingerprint, events: mpsc::Sender<Event>, mut stop: oneshot::Receiver<()>) {
    // A failed send means the engine is gone, and with it the listener; the watch ends soon.
    let tell = |event| _ = events.send(event);
    let watched = async || {
        // Who is in the channel is a glance before joining: a watch that fails shows nobody,
        // and why matters only once a join fails too.
        let Ok(mut watch) = Watch::start(&server, room_key.clone(), pin).await else { return };
        tell(Event::Present(watch.members().to_vec()));
        while watch.changed().await.is_ok() {
            tell(Event::Present(watch.members().to_vec()));
        }
    };
    loop {
        tokio::select! {
            _ = &mut stop => break,
            () = watched() => {}
        }
        tell(Event::Present(Vec::new()));
        tokio::select! {
            _ = &mut stop => return,
            () = tokio::time::sleep(WATCH_AGAIN_AFTER) => {}
        }
    }
    tell(Event::Present(Vec::new()));
}

/// A call's capture, suppressing noise as `suppression` says.
fn capture(conditioning: Conditioning, suppression: &Suppression) -> Capture {
    // libopus creates codecs without fail for the fixed formats asked for.
    Capture::new(conditioning, Stage::new(suppression)).expect("the encoder can be created")
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
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Instant;

    use bukospeak_proto::audio::{FRAME_DURATION, FRAME_SAMPLES, Level, SAMPLE_RATE};
    use bukospeak_server::{Downloads, Identity, Room};
    use rtrb::Consumer;
    use tokio::net::UdpSocket;

    use super::*;
    use crate::channel::State;
    use crate::signal::voice;
    use crate::suppression::tests::Delayed;
    use crate::suppression::{MakeSuppressor, Suppressor};

    const ROOM_KEY: &str = "room-key";
    const FRAME: Duration = Duration::from_millis(10);

    struct Updates(mpsc::Sender<Update>);

    impl Listener for Updates {
        fn update(&self, update: Update) {
            // The test may have stopped listening.
            let _ = self.0.send(update);
        }
    }

    fn engine() -> (Engine, mpsc::Receiver<Update>) {
        let (updates, received) = mpsc::channel();
        (Engine::new(Conditioning::Engine, Arc::new(Updates(updates))).unwrap(), received)
    }

    /// Waits for the first update that `wanted` picks.
    fn wait_for<T>(updates: &mpsc::Receiver<Update>, mut wanted: impl FnMut(Update) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let update = updates.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("the update within 10 s");
            if let Some(found) = wanted(update) {
                return found;
            }
        }
    }

    fn announced(updates: &mpsc::Receiver<Update>, announcement: Announcement) -> Channel {
        wait_for(updates, |update| (update.announcement.as_ref() == Some(&announcement)).then_some(update.channel))
    }

    /// Who is in the channel the engine entered, as `(you, names)`.
    fn entered(updates: &mpsc::Receiver<Update>) -> (MemberId, Vec<String>) {
        wait_for(updates, |update| {
            let names = update.channel.members().iter().map(|member| member.name.clone()).collect();
            update.channel.you().map(|you| (you, names))
        })
    }

    /// A room on a new endpoint on localhost that presents the identity.
    fn serve(runtime: &Runtime, identity: &Identity) -> quinn::Endpoint {
        let endpoint = runtime.block_on(async { bukospeak_server::endpoint(identity, "127.0.0.1:0".parse().unwrap()) }).unwrap();
        runtime.spawn(Room::new(ROOM_KEY.into(), Downloads::none()).serve(endpoint.clone()));
        endpoint
    }

    fn config(server: SocketAddr, identity: &Identity, name: &str) -> JoinConfig {
        JoinConfig { server: server.to_string(), name: name.into(), room_key: ROOM_KEY.into(), pin: Some(identity.fingerprint()) }
    }

    /// A UDP relay in front of a server that can turn to another, as a server's address does
    /// when the server restarts elsewhere, or be cut both ways, as a phone's path is when it
    /// loses the network.
    struct Relay {
        address: SocketAddr,
        upstream: Arc<Mutex<SocketAddr>>,
        cut: Arc<AtomicBool>,
    }

    impl Relay {
        fn new(runtime: &Runtime, upstream: SocketAddr) -> Self {
            let bind = || async { Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()) };
            let (front, back) = runtime.block_on(async { (bind().await, bind().await) });
            let address = front.local_addr().unwrap();
            let upstream = Arc::new(Mutex::new(upstream));
            let cut = Arc::new(AtomicBool::new(false));
            let client = Arc::new(Mutex::new(None));
            // Datagrams the relay can't pass on are lost, as on any path.
            runtime.spawn({
                let (front, back, upstream, cut, client) =
                    (Arc::clone(&front), Arc::clone(&back), Arc::clone(&upstream), Arc::clone(&cut), Arc::clone(&client));
                async move {
                    let mut datagram = [0; 65536];
                    while let Ok((length, from)) = front.recv_from(&mut datagram).await {
                        *client.lock().unwrap() = Some(from);
                        let to = *upstream.lock().unwrap();
                        if !cut.load(Ordering::Relaxed) {
                            let _ = back.send_to(&datagram[..length], to).await;
                        }
                    }
                }
            });
            runtime.spawn({
                let cut = Arc::clone(&cut);
                async move {
                    let mut datagram = [0; 65536];
                    while let Ok((length, _)) = back.recv_from(&mut datagram).await {
                        let to = *client.lock().unwrap();
                        if let Some(to) = to
                            && !cut.load(Ordering::Relaxed)
                        {
                            let _ = front.send_to(&datagram[..length], to).await;
                        }
                    }
                }
            });
            Self { address, upstream, cut }
        }

        fn cut(&self, cut: bool) {
            self.cut.store(cut, Ordering::Relaxed);
        }

        fn turn_to(&self, upstream: SocketAddr) {
            *self.upstream.lock().unwrap() = upstream;
        }
    }

    /// Joins `first` through a relay, then closes the server and turns the relay to `next`;
    /// returns once the engine announced that it joins again.
    fn lose_connection(server: &Runtime, identity: &Identity, next: SocketAddr) -> (Engine, mpsc::Receiver<Update>, Joined) {
        let old = serve(server, identity);
        let relay = Relay::new(server, old.local_addr().unwrap());
        let (engine, updates) = engine();
        let joined = engine.join(config(relay.address, identity, "first")).unwrap();
        entered(&updates);
        relay.turn_to(next);
        old.close(0u32.into(), b"gone");
        let reconnecting = announced(&updates, Announcement::Reconnecting);
        assert_eq!(reconnecting.state(), State::Reconnecting);
        (engine, updates, joined)
    }

    #[test]
    fn members_hear_each_other_and_see_who_speaks() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();

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
            talking.microphone.push(first_half, Instant::now());
            talking.microphone.push(second_half, Instant::now());
            let mut played = vec![0.0; if step % 2 == 0 { 400 } else { 560 }];
            hearing.speaker.pull(&mut played, Instant::now());
            heard.extend(played);
            next += FRAME;
            thread::sleep(next.saturating_duration_since(Instant::now()));
        }

        let second_of_speech = &heard[SAMPLE_RATE as usize / 2..][..SAMPLE_RATE as usize];
        let rms = (second_of_speech.iter().map(|sample| sample * sample).sum::<f32>() / second_of_speech.len() as f32).sqrt();
        assert!(rms > 0.03, "heard speech at an RMS of {rms}");
        let speaking =
            wait_for(&second_updates, |update| (!update.channel.speaking().is_empty()).then(|| update.channel.speaking().to_vec()));
        assert_eq!(speaking, [talking_id]);
        wait_for(&second_updates, |update| update.channel.speaking().is_empty().then_some(()));

        first.leave();
        let channel = announced(&second_updates, Announcement::Left("first".into()));
        assert!(channel.members().iter().all(|member| member.id != talking_id));
        let ended = wait_for(&first_updates, |update| (update.channel.state() == State::Out).then_some(update.lost));
        assert!(ended.is_none());
    }

    #[test]
    fn updates_tell_which_call_they_are_of() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();
        let (engine, updates) = engine();
        let calls = |updates: &mpsc::Receiver<Update>, state: State| {
            wait_for(updates, |update| (update.channel.state() == state).then_some(update.channel.call()))
        };

        let _first = engine.join(config(address, &identity, "first")).unwrap();
        assert_eq!(calls(&updates, State::Connected), 1);
        assert_eq!(engine.leave(), 1);
        // The call's last update, after the leave.
        assert_eq!(calls(&updates, State::Out), 1);
        let _second = engine.join(config(address, &identity, "first")).unwrap();
        assert_eq!(calls(&updates, State::Connected), 2);
    }

    #[test]
    fn a_lost_connection_joins_again_and_keeps_the_microphone() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let new = serve(&server, &identity).local_addr().unwrap();
        let (_engine, updates, mut joined) = lose_connection(&server, &identity, new);

        let back = announced(&updates, Announcement::Reconnected);
        assert_eq!(back.state(), State::Connected);
        let you = back.you().unwrap();

        // The microphone of the first join sends over the second.
        let mut listener = server.block_on(Session::join(config(new, &identity, "listener"), Entry::default())).unwrap();
        // Paced as a device captures it: a second at once would be too late to be heard.
        let start = Instant::now();
        for (frame, samples) in voice(SAMPLE_RATE as usize, 0.3).as_chunks::<FRAME_SAMPLES>().0.iter().enumerate() {
            joined.microphone.push(samples, Instant::now());
            thread::sleep((start + FRAME_DURATION * (frame as u32 + 1)).saturating_duration_since(Instant::now()));
        }
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
    fn a_server_nothing_comes_from_shows_reconnecting_until_it_is_heard_again() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();
        let relay = Relay::new(&server, address);
        let (engine, updates) = engine();
        let _joined = engine.join(config(relay.address, &identity, "first")).unwrap();
        let (you, _) = entered(&updates);

        relay.cut(true);
        let stalled = Instant::now();
        announced(&updates, Announcement::Reconnecting);
        assert!(stalled.elapsed() < Duration::from_secs(5), "noticed after {:?}", stalled.elapsed());
        relay.cut(false);
        let back = announced(&updates, Announcement::Reconnected);
        assert_eq!((back.state(), back.you()), (State::Connected, Some(you)));
    }

    /// Makes suppressors that pass the voice through, and counts how many it made and how many
    /// frames they processed.
    #[derive(Default)]
    struct Counting {
        made: AtomicUsize,
        processed: Arc<AtomicUsize>,
    }

    struct Counted {
        delayed: Delayed,
        processed: Arc<AtomicUsize>,
    }

    impl Suppressor for Counted {
        fn delay(&self) -> usize {
            self.delayed.delay()
        }

        fn process(&mut self, frame: &[f32; FRAME_SAMPLES], out: &mut [f32; FRAME_SAMPLES]) {
            self.processed.fetch_add(1, Ordering::Relaxed);
            self.delayed.process(frame, out);
        }

        fn reset(&mut self) {
            self.delayed.reset();
        }
    }

    impl MakeSuppressor for Counting {
        fn make(&self) -> Option<Box<dyn Suppressor>> {
            self.made.fetch_add(1, Ordering::Relaxed);
            Some(Box::new(Counted { delayed: Delayed::new(2), processed: Arc::clone(&self.processed) }))
        }
    }

    /// Pushes a frame of voice into the microphone each frame's time until `done`, for at most 5 s.
    fn speak_until(microphone: &mut Microphone, done: impl Fn() -> bool) {
        let frame = voice(FRAME_SAMPLES, 0.3);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "not done within 5 s");
            microphone.push(&frame, Instant::now());
            thread::sleep(FRAME);
        }
    }

    #[test]
    fn a_suppressor_plugged_in_runs_in_the_call_and_one_chosen_during_it_takes_over() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();
        let (engine, updates) = engine();
        let (first, second) = (Arc::new(Counting::default()), Arc::new(Counting::default()));
        engine.set_suppression(Suppression::Plugged(first.clone())).unwrap();
        assert_eq!(first.made.load(Ordering::Relaxed), 0, "made for a call only");

        let mut joined = engine.join(config(address, &identity, "first")).unwrap();
        entered(&updates);
        assert_eq!(first.made.load(Ordering::Relaxed), 1);
        speak_until(&mut joined.microphone, || first.processed.load(Ordering::Relaxed) > 0);

        engine.set_suppression(Suppression::Plugged(second.clone())).unwrap();
        speak_until(&mut joined.microphone, || second.processed.load(Ordering::Relaxed) > 0);
        // The first is dropped with the frame the second took over on.
        let before = first.processed.load(Ordering::Relaxed);
        speak_until(&mut joined.microphone, || second.processed.load(Ordering::Relaxed) > 10);
        assert_eq!(first.processed.load(Ordering::Relaxed), before);
        assert_eq!(second.made.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_server_that_restarts_is_joined_again_once_it_is_up() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let old = serve(&server, &identity);
        let relay = Relay::new(&server, old.local_addr().unwrap());
        let (engine, updates) = engine();
        let _joined = engine.join(config(relay.address, &identity, "first")).unwrap();
        entered(&updates);
        // It stops as the server does, and turns away joins while it closes.
        old.close(0u32.into(), b"server stopped");
        announced(&updates, Announcement::Reconnecting);
        thread::sleep(Duration::from_millis(500));
        relay.turn_to(serve(&server, &identity).local_addr().unwrap());

        assert_eq!(announced(&updates, Announcement::Reconnected).state(), State::Connected);
    }

    #[test]
    fn a_lost_connection_that_cant_join_again_ends_the_call() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        // A socket nobody answers on.
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let (_engine, updates, _joined) = lose_connection(&server, &identity, silent.local_addr().unwrap());

        let (channel, lost) = wait_for(&updates, |update| update.lost.map(|lost| (update.channel, lost)));
        assert_eq!(channel.state(), State::Out);
        assert!(matches!(lost.rejoin, Error::TimedOut), "{lost}");
    }

    #[test]
    fn a_leave_ends_joining_again() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let (engine, updates, _joined) = lose_connection(&server, &identity, silent.local_addr().unwrap());

        let leaving = Instant::now();
        engine.leave();
        assert!(leaving.elapsed() < Duration::from_secs(1), "the leave took {:?}", leaving.elapsed());
        let ended = wait_for(&updates, |update| (update.channel.state() == State::Out).then_some(update.lost));
        assert!(ended.is_none());
    }

    #[test]
    fn the_others_see_who_is_muted() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();
        let muted = |updates: &mpsc::Receiver<Update>, wanted: bool| {
            wait_for(updates, |update| {
                let first = update.channel.members().iter().find(|member| member.name == "first")?;
                (first.muted == wanted).then_some(())
            });
        };

        let (first, _first_updates) = engine();
        first.set_muted(true);
        let _talking = first.join(config(address, &identity, "first")).unwrap();
        let (second, updates) = engine();
        let _hearing = second.join(config(address, &identity, "second")).unwrap();
        muted(&updates, true);
        first.set_muted(false);
        muted(&updates, false);
    }

    #[test]
    fn watching_shows_who_is_in_the_channel_until_a_join() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let address = serve(&server, &identity).local_addr().unwrap();
        let present = |updates: &mpsc::Receiver<Update>, names: &[&str]| {
            wait_for(updates, |update| {
                let present: Vec<&str> = update.channel.present().iter().map(|member| member.name.as_str()).collect();
                (present == names).then_some(())
            });
        };

        let (first, _first_updates) = engine();
        let _talking = first.join(config(address, &identity, "first")).unwrap();
        let (watching, updates) = engine();
        watching.watch(address.to_string(), ROOM_KEY.into(), identity.fingerprint());
        present(&updates, &["first"]);
        let (second, _second_updates) = engine();
        let _hearing = second.join(config(address, &identity, "second")).unwrap();
        present(&updates, &["first", "second"]);
        first.leave();
        present(&updates, &["second"]);

        // Joining stops the watch: the call shows who is in it.
        let _joined = watching.join(config(address, &identity, "third")).unwrap();
        present(&updates, &[]);
        let (_, names) = entered(&updates);
        assert_eq!(names, ["second", "third"]);
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
