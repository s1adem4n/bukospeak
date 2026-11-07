use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bukospeak_core::Error;
use bukospeak_core::download::Download;
use bukospeak_core::proto::control::{Member, Rejection};
use bukospeak_core::proto::{IDLE_TIMEOUT, MemberId, audio};
use bukospeak_core::session::{Event, Session, Watch};
use bukospeak_server::{Device, Downloads, Room, Tailnet, endpoint};
use tokio::net::UdpSocket;
use tokio::time::timeout;

fn own(peer: SocketAddr) -> Option<Device> {
    Some(Device { id: peer.to_string(), name: format!("member {}", peer.port()) })
}

fn phone(_: SocketAddr) -> Option<Device> {
    Some(Device { id: "phone".into(), name: "Phone Owner".into() })
}

struct Down;

impl Tailnet for Down {
    fn whois(&self, _: SocketAddr) -> anyhow::Result<Option<Device>> {
        anyhow::bail!("tailscaled isn't running")
    }
}

fn serve(tailnet: impl Tailnet) -> String {
    let endpoint = endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = endpoint.local_addr().unwrap();
    tokio::spawn(Room::new(tailnet, Downloads::none()).serve(endpoint));
    address.to_string()
}

async fn join(server: &str) -> Result<Session, Error> {
    Session::join(server, false).await
}

async fn next(session: &mut Session) -> Event {
    next_within(session, Duration::from_secs(2)).await
}

async fn next_within(session: &mut Session, within: Duration) -> Event {
    timeout(within, session.next_event()).await.expect("an event in time").unwrap()
}

async fn changed(watch: &mut Watch) {
    changed_within(watch, Duration::from_secs(2)).await;
}

async fn changed_within(watch: &mut Watch, within: Duration) {
    timeout(within, watch.changed()).await.expect("a change in time").unwrap();
}

fn ids(members: &[Member]) -> Vec<MemberId> {
    members.iter().map(|member| member.id).collect()
}

#[tokio::test]
async fn members_hear_each_other_and_see_who_comes_and_goes() {
    let server = serve(own);
    let mut first = join(&server).await.unwrap();
    let mut second = join(&server).await.unwrap();

    assert_eq!(ids(second.members()), [first.you(), second.you()]);
    let Event::Joined(joined) = next(&mut first).await else { panic!("the first member isn't told the second joined") };
    assert_eq!(joined.id, second.you());

    let frame = audio::Frame { from: MemberId(0), index: 7, last: true, level: audio::Level::new(30), opus: vec![1, 2, 3] };
    first.audio_sender().send(&frame).unwrap();
    assert_eq!(next(&mut second).await, Event::Audio(audio::Frame { from: first.you(), ..frame }));

    let id = second.you();
    second.leave().await;
    assert_eq!(next(&mut first).await, Event::Left(id));
}

#[tokio::test]
async fn a_member_is_named_after_its_devices_owner_without_control_characters() {
    let server = serve(|_| Some(Device { id: "first".into(), name: " First\u{1b}[2J Member ".into() }));
    let first = join(&server).await.unwrap();
    assert_eq!(first.members()[0].name, "First[2J Member");
}

#[tokio::test]
async fn a_device_the_tailnet_doesnt_know_is_turned_away() {
    let server = serve(|_| None);
    assert!(matches!(join(&server).await, Err(Error::Rejected(Rejection::Stranger))));
    assert!(matches!(Watch::start(&server).await, Err(Error::Rejected(Rejection::Stranger))));
    let download = Download::start(&server, "sounds".into(), |_, _| ()).await;
    assert!(matches!(download, Err(Error::Rejected(Rejection::Stranger))));
}

#[tokio::test]
async fn a_server_that_cant_ask_its_tailnet_turns_everyone_away() {
    let server = serve(Down);
    assert!(matches!(join(&server).await, Err(Error::Rejected(Rejection::Unidentified))));
}

#[tokio::test(start_paused = true)]
async fn a_server_that_never_answers_times_the_join_out() {
    let silent = endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server = silent.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let connection = silent.accept().await.unwrap().await.unwrap();
        connection.closed().await;
    });
    assert!(matches!(join(&server).await, Err(Error::TimedOut)));
}

#[tokio::test]
async fn a_watcher_sees_who_comes_and_goes_without_joining() {
    let server = serve(own);
    let first = join(&server).await.unwrap();
    let mut watch = Watch::start(&server).await.unwrap();
    assert_eq!(ids(watch.members()), [first.you()]);

    let second = join(&server).await.unwrap();
    changed(&mut watch).await;
    assert_eq!(ids(watch.members()), [first.you(), second.you()]);
    assert_eq!(ids(second.members()), [first.you(), second.you()]);

    first.leave().await;
    changed(&mut watch).await;
    assert_eq!(ids(watch.members()), [second.you()]);
}

#[tokio::test]
async fn members_and_watchers_see_who_is_muted() {
    let server = serve(own);
    let mut first = join(&server).await.unwrap();
    let second = Session::join(&server, true).await.unwrap();
    let Event::Joined(joined) = next(&mut first).await else { panic!("the first member isn't told the second joined") };
    assert!(joined.muted);
    let mut watch = Watch::start(&server).await.unwrap();
    assert_eq!(watch.members().iter().map(|member| member.muted).collect::<Vec<_>>(), [false, true]);

    first.set_muted(true).await.unwrap();
    let Event::Changed(muted) = next(&mut first).await else { panic!("the first member isn't told it muted") };
    assert_eq!((muted.id, muted.muted), (first.you(), true));
    changed(&mut watch).await;
    assert!(watch.members().iter().all(|member| member.muted));
    drop(second);
}

#[tokio::test]
async fn a_device_that_joins_again_takes_its_place_and_its_old_connection_is_replaced() {
    let server = serve(phone);
    let mut old = join(&server).await.unwrap();
    let mut watch = Watch::start(&server).await.unwrap();

    let new = Session::join(&server, true).await.unwrap();
    assert_eq!(new.you(), old.you());
    assert_eq!(ids(new.members()), [old.you()]);
    changed(&mut watch).await;
    assert_eq!((ids(watch.members()), watch.members()[0].muted), (vec![old.you()], true));
    assert!(matches!(timeout(Duration::from_secs(2), old.next_event()).await.unwrap(), Err(Error::Replaced)));

    new.leave().await;
    changed(&mut watch).await;
    assert!(watch.members().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_lost_member_keeps_its_place_for_a_while_and_its_device_comes_back_or_it_leaves() {
    let server = serve(phone);
    let mut watch = Watch::start(&server).await.unwrap();
    for comes_back in [true, false] {
        let relay = Relay::new(server.parse().unwrap()).await;
        let lost = join(&relay.address.to_string()).await.unwrap();
        let id = lost.you();
        changed(&mut watch).await;

        relay.cut.store(true, Ordering::Relaxed);
        changed_within(&mut watch, Duration::from_secs(5)).await;
        assert_eq!((ids(watch.members()), watch.members()[0].reconnecting), (vec![id], true));
        assert!(timeout(IDLE_TIMEOUT + Duration::from_secs(5), watch.changed()).await.is_err());
        if comes_back {
            let back = join(&server).await.unwrap();
            assert_eq!(back.you(), id);
            changed(&mut watch).await;
            assert_eq!((ids(watch.members()), watch.members()[0].reconnecting), (vec![id], false));
            back.leave().await;
        } else {
            changed_within(&mut watch, Duration::from_secs(60)).await;
            assert!(watch.members().is_empty());
            let late = join(&server).await.unwrap();
            assert_ne!(late.you(), id);
            changed(&mut watch).await;
            late.leave().await;
        }
        changed(&mut watch).await;
        assert!(watch.members().is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn a_member_nothing_comes_from_shows_reconnecting_until_it_is_heard_again() {
    let server = serve(own);
    let mut first = join(&server).await.unwrap();
    let relay = Relay::new(server.parse().unwrap()).await;
    let second = join(&relay.address.to_string()).await.unwrap();
    assert!(matches!(next(&mut first).await, Event::Joined(_)));

    let soon = Duration::from_secs(5);
    relay.cut.store(true, Ordering::Relaxed);
    let Event::Changed(silent) = next_within(&mut first, soon).await else { panic!("the second member leaves") };
    assert_eq!((silent.id, silent.reconnecting), (second.you(), true));
    relay.cut.store(false, Ordering::Relaxed);
    let Event::Changed(heard) = next_within(&mut first, soon).await else { panic!("the second member leaves") };
    assert_eq!((heard.id, heard.reconnecting), (second.you(), false));
}

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
