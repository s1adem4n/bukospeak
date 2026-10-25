//! The engine the apps drive: joins and leaves the channel, runs the network side of a call,
//! and tells a [`Listener`] what happens.
//!
//! A join hands the caller the call's two realtime ends, a [`Microphone`] and a [`Speaker`],
//! for the platform's audio to drive until the call is over; they exist only for a call.

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use bukospeak_proto::control::Member;
use bukospeak_proto::{Fingerprint, MemberId};
use rtrb::{Producer, RingBuffer};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::capture::Capture;
use crate::jitter::Packet;
use crate::microphone::{Microphone, Settings};
use crate::playout::Mixer;
use crate::session::{self, Error, JoinConfig, Session};
use crate::speaker::{Downlink, Speaker};
use crate::speaking::Speaking;
use crate::uplink;

/// Frames the uplink may fall behind the microphone: 1 s.
const UPLINK_FRAMES: usize = 100;

/// Frames and leaves the speaker may fall behind the network: 0.5 s of every voice.
const DOWNLINK_FRAMES: usize = 50 * crate::playout::VOICES;

/// How often the engine looks who speaks: quick enough for an indicator to feel live.
const SPEAKING_EVERY: Duration = Duration::from_millis(50);

/// Receives the engine's events, in order, on a thread of their own, so it may call the
/// engine; but not drop it, since dropping waits for that thread.
pub trait Listener: Send + Sync {
    fn event(&self, event: Event);
}

#[derive(Debug)]
pub enum Event {
    Joined(Member),
    Left(MemberId),
    /// Who speaks now, in id order, this member among them; sent when it changes.
    Speaking(Vec<MemberId>),
    /// The call ended without a leave: the connection was lost or the server broke the protocol.
    Disconnected(Error),
}

/// A call just joined.
pub struct Joined {
    pub you: MemberId,
    /// Everyone in the channel, this member included; [`Event`]s tell what changes.
    pub members: Vec<Member>,
    /// The server's fingerprint, to pin for the next join.
    pub fingerprint: Fingerprint,
    pub microphone: Microphone,
    pub speaker: Speaker,
}

/// Joins and leaves the channel, one call at a time. Dropping it leaves the call and returns
/// once the listener has had its last event.
pub struct Engine {
    runtime: Runtime,
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
    pub fn new(listener: Arc<dyn Listener>) -> std::io::Result<Self> {
        // One worker runs the network side; a current-thread runtime would run it only while
        // a call into the engine blocks on it.
        let runtime =
            tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("bukospeak-network").enable_all().build()?;
        let (events, inbox) = mpsc::channel();
        let listening = thread::Builder::new()
            .name("bukospeak-events".into())
            .spawn(move || inbox.into_iter().for_each(|event| listener.event(event)))?;
        Ok(Self { runtime, settings: Arc::default(), call: Mutex::default(), events, _listening: Listening(Some(listening)) })
    }

    /// Joins the channel, after leaving the call before if there is one. A call starts unmuted.
    pub fn join(&self, config: JoinConfig) -> Result<Joined, Error> {
        // Held until the call is in place, so a leave meanwhile, say from the listener's
        // thread, waits for the join and then ends its call instead of missing it.
        let mut call = self.call.lock().expect("the call lock is never poisoned");
        self.end(call.take());
        // libopus creates codecs without fail for the fixed formats asked for.
        let capture = Capture::new().expect("the encoder can be created");
        let mixer = Mixer::new().expect("the decoders can be created");

        let session = self.runtime.block_on(Session::join(config))?;
        self.settings.muted.store(false, Ordering::Relaxed);
        let (you, members, fingerprint) = (session.you(), session.members().to_vec(), session.fingerprint());
        let speaking = Arc::new(Speaking::default());
        let (frames, uplink_frames) = RingBuffer::new(UPLINK_FRAMES);
        let uplink = uplink::spawn(uplink_frames, session.audio_sender())?;
        let (downlink, speaker_downlink) = RingBuffer::new(DOWNLINK_FRAMES);

        let (stop, stopped) = oneshot::channel();
        let task = self.runtime.spawn(run(session, Deliveries::new(downlink), Arc::clone(&speaking), self.events.clone(), stopped));
        *call = Some(Call { stop, task });
        Ok(Joined {
            you,
            members,
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

/// The network side of a call: moves the members' frames to the speaker and tells the
/// listener who comes, goes and speaks, until it is stopped or the connection is lost.
async fn run(
    mut session: Session,
    mut deliveries: Deliveries,
    speaking: Arc<Speaking>,
    events: mpsc::Sender<Event>,
    mut stop: oneshot::Receiver<()>,
) {
    let you = session.you();
    let mut members: BTreeSet<MemberId> = session.members().iter().map(|member| member.id).filter(|&id| id != you).collect();
    let mut speakers = Vec::new();
    let mut watch = tokio::time::interval(SPEAKING_EVERY);
    watch.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let start = Instant::now();
    // A failed send means the engine is gone, and with it the listener; the call ends soon.
    let tell = |event| _ = events.send(event);

    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = watch.tick() => {
                deliveries.retry();
                let now = speaking.members(you);
                if now != speakers {
                    speakers.clone_from(&now);
                    tell(Event::Speaking(now));
                }
            }
            event = session.next_event() => match event {
                Ok(session::Event::Audio(frame)) => {
                    // Frames of a member that left, still on the way, would take a voice for good.
                    if members.contains(&frame.from)
                        && let Some(packet) = Packet::new(frame.index, frame.last, frame.level, start.elapsed().as_micros() as u64, &frame.opus)
                    {
                        deliveries.frame(frame.from, packet);
                    }
                }
                Ok(session::Event::Joined(member)) => {
                    members.insert(member.id);
                    tell(Event::Joined(member));
                }
                Ok(session::Event::Left(id)) => {
                    members.remove(&id);
                    deliveries.left(id);
                    tell(Event::Left(id));
                }
                Err(error) => {
                    tell(Event::Disconnected(error));
                    return;
                }
            },
        }
    }
    session.leave().await;
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
    use std::time::Instant;

    use bukospeak_proto::audio::{Level, SAMPLE_RATE};
    use bukospeak_server::{Identity, Room};
    use rtrb::Consumer;

    use super::*;
    use crate::signal::voice;

    const ROOM_KEY: &str = "room-key";
    const FRAME: Duration = Duration::from_millis(10);

    struct Events(mpsc::Sender<Event>);

    impl Listener for Events {
        fn event(&self, event: Event) {
            // The test may have stopped listening.
            let _ = self.0.send(event);
        }
    }

    fn engine() -> (Engine, mpsc::Receiver<Event>) {
        let (events, received) = mpsc::channel();
        (Engine::new(Arc::new(Events(events))).unwrap(), received)
    }

    /// Waits for the first event that `wanted` picks.
    fn wait_for<T>(events: &mpsc::Receiver<Event>, mut wanted: impl FnMut(Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let event = events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("the event within 5 s");
            if let Some(found) = wanted(event) {
                return found;
            }
        }
    }

    #[test]
    fn members_hear_each_other_and_see_who_speaks() {
        let server = Runtime::new().unwrap();
        let identity = Identity::generate().unwrap();
        let endpoint = server.block_on(async { bukospeak_server::endpoint(&identity, "127.0.0.1:0".parse().unwrap()) }).unwrap();
        let address = endpoint.local_addr().unwrap().to_string();
        server.spawn(Room::new(ROOM_KEY.into()).serve(endpoint));
        let config = |name: &str| JoinConfig {
            server: address.clone(),
            name: name.into(),
            room_key: ROOM_KEY.into(),
            pin: Some(identity.fingerprint()),
        };

        let (first, first_events) = engine();
        let (second, second_events) = engine();
        let mut talking = first.join(config("first")).unwrap();
        let mut hearing = second.join(config("second")).unwrap();
        let joined = wait_for(&first_events, |event| match event {
            Event::Joined(member) => Some(member.id),
            _ => None,
        });
        assert_eq!(joined, hearing.you);

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
        let speaking = wait_for(&second_events, |event| match event {
            Event::Speaking(members) if !members.is_empty() => Some(members),
            _ => None,
        });
        assert_eq!(speaking, [talking.you]);
        wait_for(&second_events, |event| matches!(event, Event::Speaking(members) if members.is_empty()).then_some(()));

        first.leave();
        let left = wait_for(&second_events, |event| match event {
            Event::Left(id) => Some(id),
            _ => None,
        });
        assert_eq!(left, talking.you);
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
        let (engine, events) = engine();
        drop(engine);
        // The listener, holding the only sender, is gone with its thread.
        assert_eq!(events.try_recv().unwrap_err(), mpsc::TryRecvError::Disconnected);
    }
}
