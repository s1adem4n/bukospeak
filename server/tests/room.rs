use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::proto::{Fingerprint, IDLE_TIMEOUT, MemberId, audio};
use bukospeak_core::session::{Entry, Event, Session, Watch};
use bukospeak_core::{Error, JoinConfig};
use bukospeak_server::{Identity, Room, endpoint};
use tokio::net::UdpSocket;
use tokio::time::timeout;

const ROOM_KEY: &str = "room-key";

struct Server {
    address: SocketAddr,
    fingerprint: Fingerprint,
}

fn serve() -> Server {
    let identity = Identity::generate().unwrap();
    let endpoint = endpoint(&identity, "127.0.0.1:0".parse().unwrap()).unwrap();
    let address = endpoint.local_addr().unwrap();
    tokio::spawn(Room::new(ROOM_KEY.into()).serve(endpoint));
    Server { address, fingerprint: identity.fingerprint() }
}

fn config(server: &Server, name: &str) -> JoinConfig {
    JoinConfig { server: server.address.to_string(), name: name.into(), room_key: ROOM_KEY.into(), pin: Some(server.fingerprint) }
}

/// Joins unmuted, as a new member.
async fn join(config: JoinConfig) -> Result<Session, Error> {
    Session::join(config, Entry::default()).await
}

async fn next(session: &mut Session) -> Event {
    next_within(session, Duration::from_secs(2)).await
}

async fn next_within(session: &mut Session, within: Duration) -> Event {
    timeout(within, session.next_event()).await.expect("an event in time").unwrap()
}

async fn changed(watch: &mut Watch) {
    timeout(Duration::from_secs(2), watch.changed()).await.expect("a change in time").unwrap();
}

fn names(members: &[Member]) -> Vec<&str> {
    members.iter().map(|member| member.name.as_str()).collect()
}

#[tokio::test]
async fn members_hear_each_other_and_see_who_comes_and_goes() {
    let server = serve();
    let mut first = join(config(&server, "first")).await.unwrap();
    let mut second = join(config(&server, " second ")).await.unwrap();

    assert_eq!(names(second.members()), ["first", "second"]);
    let Event::Joined(joined) = next(&mut first).await else { panic!("the first member isn't told the second joined") };
    assert_eq!((joined.id, joined.name.as_str()), (second.you(), "second"));

    let frame = audio::Frame { from: MemberId(0), index: 7, last: true, level: audio::Level::new(30), opus: vec![1, 2, 3] };
    first.audio_sender().send(&frame).unwrap();
    assert_eq!(next(&mut second).await, Event::Audio(audio::Frame { from: first.you(), ..frame }));

    let id = second.you();
    second.leave().await;
    assert_eq!(next(&mut first).await, Event::Left(id));
}

#[tokio::test]
async fn a_wrong_room_key_is_turned_away() {
    let server = serve();
    let joined = join(JoinConfig { room_key: "guess".into(), ..config(&server, "stranger") }).await;
    assert!(matches!(joined, Err(Error::Rejected(Rejection::RoomKey))));
}

#[tokio::test]
async fn a_name_with_control_characters_is_turned_away() {
    let server = serve();
    let joined = join(config(&server, "first\u{1b}[2J")).await;
    assert!(matches!(joined, Err(Error::Rejected(Rejection::Name))));
}

#[tokio::test]
async fn the_first_join_learns_the_fingerprint_and_a_changed_one_is_refused() {
    let server = serve();
    let first = join(JoinConfig { pin: None, ..config(&server, "first") }).await.unwrap();
    assert_eq!(first.fingerprint(), server.fingerprint);

    let stale = Fingerprint::of(b"another certificate");
    let joined = join(JoinConfig { pin: Some(stale), ..config(&server, "first") }).await;
    let Err(Error::FingerprintChanged { pinned, presented }) = joined else { panic!("joined despite a changed certificate") };
    assert_eq!((pinned, presented), (stale, server.fingerprint));
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_times_the_join_out() {
    let identity = Identity::generate().unwrap();
    let silent = endpoint(&identity, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = Server { address: silent.local_addr().unwrap(), fingerprint: identity.fingerprint() };
    // It completes the handshake but never reads the Hello.
    tokio::spawn(async move {
        let connection = silent.accept().await.unwrap().await.unwrap();
        connection.closed().await;
    });
    assert!(matches!(join(config(&server, "first")).await, Err(Error::TimedOut)));
}

#[tokio::test]
async fn a_watcher_sees_who_comes_and_goes_without_joining() {
    let server = serve();
    let first = join(config(&server, "first")).await.unwrap();
    let mut watch = Watch::start(&server.address.to_string(), ROOM_KEY.into(), server.fingerprint).await.unwrap();
    assert_eq!(names(watch.members()), ["first"]);

    let second = join(config(&server, "second")).await.unwrap();
    changed(&mut watch).await;
    assert_eq!(names(watch.members()), ["first", "second"]);
    // The watcher isn't in the channel.
    assert_eq!(names(second.members()), ["first", "second"]);

    first.leave().await;
    changed(&mut watch).await;
    assert_eq!(names(watch.members()), ["second"]);
}

#[tokio::test]
async fn a_watch_with_a_wrong_room_key_is_turned_away() {
    let server = serve();
    let watch = Watch::start(&server.address.to_string(), "guess".into(), server.fingerprint).await;
    assert!(matches!(watch, Err(Error::Rejected(Rejection::RoomKey))));
}

#[tokio::test]
async fn members_and_watchers_see_who_is_muted() {
    let server = serve();
    let mut first = join(config(&server, "first")).await.unwrap();
    let second = Session::join(config(&server, "second"), Entry { muted: true, resume: None }).await.unwrap();
    let Event::Joined(joined) = next(&mut first).await else { panic!("the first member isn't told the second joined") };
    assert!(joined.muted);
    let mut watch = Watch::start(&server.address.to_string(), ROOM_KEY.into(), server.fingerprint).await.unwrap();
    assert_eq!(watch.members().iter().map(|member| member.muted).collect::<Vec<_>>(), [false, true]);

    first.set_muted(true).await.unwrap();
    let Event::Changed(muted) = next(&mut first).await else { panic!("the first member isn't told it muted") };
    assert_eq!((muted.id, muted.muted), (first.you(), true));
    changed(&mut watch).await;
    assert!(watch.members().iter().all(|member| member.muted));
    drop(second);
}

#[tokio::test]
async fn a_resume_takes_the_members_place_while_its_old_connection_lingers() {
    let server = serve();
    let mut first = join(config(&server, "first")).await.unwrap();
    let mut second = join(config(&server, "second")).await.unwrap();
    next(&mut first).await;

    let entry = Entry { muted: true, resume: Some(second.resume()) };
    let resumed = Session::join(config(&server, "second"), entry).await.unwrap();
    assert_eq!(resumed.you(), second.you());
    assert_eq!(names(resumed.members()), ["first", "second"]);
    let Event::Changed(changed) = next(&mut first).await else { panic!("the first member sees the second leave or join") };
    assert_eq!((changed.id, changed.muted), (second.you(), true));
    // The server closed the old connection.
    assert!(timeout(Duration::from_secs(2), second.next_event()).await.unwrap().is_err());

    resumed.leave().await;
    assert_eq!(next(&mut first).await, Event::Left(second.you()));
}

#[tokio::test(start_paused = true)]
async fn a_lost_member_keeps_its_place_for_a_while_and_comes_back_or_leaves() {
    let server = serve();
    let mut first = join(config(&server, "first")).await.unwrap();
    for comes_back in [true, false] {
        let relay = Relay::new(server.address).await;
        let second = join(JoinConfig { server: relay.address.to_string(), ..config(&server, "second") }).await.unwrap();
        let id = second.you();
        assert!(matches!(next(&mut first).await, Event::Joined(_)));

        relay.cut.store(true, Ordering::Relaxed);
        let Event::Changed(reconnecting) = next_within(&mut first, Duration::from_secs(5)).await else {
            panic!("the second member leaves")
        };
        assert_eq!((reconnecting.id, reconnecting.reconnecting), (id, true));
        // The connection times out meanwhile, and the server holds the place on.
        assert!(timeout(IDLE_TIMEOUT + Duration::from_secs(5), first.next_event()).await.is_err());
        let entry = Entry { muted: false, resume: Some(second.resume()) };
        if comes_back {
            let resumed = Session::join(config(&server, "second"), entry).await.unwrap();
            assert_eq!(resumed.you(), id);
            let Event::Changed(back) = next(&mut first).await else { panic!("the second member doesn't come back") };
            assert!(!back.reconnecting);
            resumed.leave().await;
            assert_eq!(next(&mut first).await, Event::Left(id));
        } else {
            assert_eq!(next_within(&mut first, Duration::from_secs(60)).await, Event::Left(id));
            // Once the place is given up, the token joins as a new member.
            let late = Session::join(config(&server, "second"), entry).await.unwrap();
            assert_ne!(late.you(), id);
            assert!(matches!(next(&mut first).await, Event::Joined(_)));
            late.leave().await;
            next(&mut first).await;
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_member_nothing_comes_from_shows_reconnecting_until_it_is_heard_again() {
    let server = serve();
    let mut first = join(config(&server, "first")).await.unwrap();
    let relay = Relay::new(server.address).await;
    let second = join(JoinConfig { server: relay.address.to_string(), ..config(&server, "second") }).await.unwrap();
    assert!(matches!(next(&mut first).await, Event::Joined(_)));

    // Well before the connection would time out.
    let soon = Duration::from_secs(5);
    relay.cut.store(true, Ordering::Relaxed);
    let Event::Changed(silent) = next_within(&mut first, soon).await else { panic!("the second member leaves") };
    assert_eq!((silent.id, silent.reconnecting), (second.you(), true));
    relay.cut.store(false, Ordering::Relaxed);
    let Event::Changed(heard) = next_within(&mut first, soon).await else { panic!("the second member leaves") };
    assert_eq!((heard.id, heard.reconnecting), (second.you(), false));
}

/// A UDP relay in front of the server whose path can be cut, as a phone's is when it loses
/// the network.
struct Relay {
    address: SocketAddr,
    cut: Arc<AtomicBool>,
}

impl Relay {
    async fn new(server: SocketAddr) -> Self {
        let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let back = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        back.connect(server).await.unwrap();
        let address = front.local_addr().unwrap();
        let cut = Arc::new(AtomicBool::new(false));
        let client = Arc::new(Mutex::new(None));
        // Datagrams the relay can't pass on are lost, as on any path.
        tokio::spawn({
            let (front, back, cut, client) = (Arc::clone(&front), Arc::clone(&back), Arc::clone(&cut), Arc::clone(&client));
            async move {
                let mut datagram = [0; 65536];
                while let Ok((length, from)) = front.recv_from(&mut datagram).await {
                    *client.lock().unwrap() = Some(from);
                    if !cut.load(Ordering::Relaxed) {
                        let _ = back.send(&datagram[..length]).await;
                    }
                }
            }
        });
        tokio::spawn({
            let cut = Arc::clone(&cut);
            async move {
                let mut datagram = [0; 65536];
                while let Ok(length) = back.recv(&mut datagram).await {
                    let to = *client.lock().unwrap();
                    if let Some(to) = to
                        && !cut.load(Ordering::Relaxed)
                    {
                        let _ = front.send_to(&datagram[..length], to).await;
                    }
                }
            }
        });
        Self { address, cut }
    }
}
