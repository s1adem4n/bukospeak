use std::net::SocketAddr;
use std::time::Duration;

use bukospeak_core::proto::control::Rejection;
use bukospeak_core::proto::{Fingerprint, MemberId, audio};
use bukospeak_core::{Error, Event, JoinConfig, Session};
use bukospeak_server::{Identity, Room, endpoint};
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

async fn next(session: &mut Session) -> Event {
    timeout(Duration::from_secs(2), session.next_event()).await.expect("an event within 2 s").unwrap()
}

#[tokio::test]
async fn members_hear_each_other_and_see_who_comes_and_goes() {
    let server = serve();
    let mut first = Session::join(config(&server, "first")).await.unwrap();
    let mut second = Session::join(config(&server, " second ")).await.unwrap();

    let names: Vec<&str> = second.members().iter().map(|member| member.name.as_str()).collect();
    assert_eq!(names, ["first", "second"]);
    let Event::Joined(joined) = next(&mut first).await else { panic!("the first member isn't told the second joined") };
    assert_eq!((joined.id, joined.name.as_str()), (second.you(), "second"));

    let frame = audio::Frame { from: MemberId(0), seq: 7, flags: audio::FLAG_LAST, opus: vec![1, 2, 3] };
    first.audio_sender().send(&frame).unwrap();
    assert_eq!(next(&mut second).await, Event::Audio(audio::Frame { from: first.you(), ..frame }));

    let id = second.you();
    second.leave().await;
    assert_eq!(next(&mut first).await, Event::Left(id));
}

#[tokio::test]
async fn a_wrong_room_key_is_turned_away() {
    let server = serve();
    let joined = Session::join(JoinConfig { room_key: "guess".into(), ..config(&server, "stranger") }).await;
    assert!(matches!(joined, Err(Error::Rejected(Rejection::RoomKey))));
}

#[tokio::test]
async fn the_first_join_learns_the_fingerprint_and_a_changed_one_is_refused() {
    let server = serve();
    let first = Session::join(JoinConfig { pin: None, ..config(&server, "first") }).await.unwrap();
    assert_eq!(first.fingerprint(), server.fingerprint);

    let stale = Fingerprint::of(b"another certificate");
    let joined = Session::join(JoinConfig { pin: Some(stale), ..config(&server, "first") }).await;
    let Err(Error::FingerprintChanged { pinned, presented }) = joined else { panic!("joined despite a changed certificate") };
    assert_eq!((pinned, presented), (stale, server.fingerprint));
}
