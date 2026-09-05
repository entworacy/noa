use super::super::queue::Pending;
use super::*;
use std::net::Shutdown;

fn pair() -> (NativeConnection, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let (stream, _) = listener.accept().unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    (
        NativeConnection {
            stream,
            reader,
            pid: 42,
        },
        peer,
    )
}

fn enqueue(
    state: &ChannelState<KakaoCommand>,
    id: u64,
    action: KakaoAction,
) -> mpsc::Receiver<Result<Option<String>, String>> {
    let (sender, response) = mpsc::sync_channel(1);
    state
        .enqueue(
            id,
            Pending { sender },
            KakaoCommand {
                id,
                room_id: 123,
                action,
                deadline: Instant::now() + Duration::from_secs(3),
            },
        )
        .unwrap();
    response
}

fn reply(peer: &mut TcpStream) {
    let mut line = String::new();
    BufReader::new(peer.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    writeln!(
        peer,
        "{}",
        serde_json::json!({"id": request["id"], "ok": true, "value": "done"})
    )
    .unwrap();
}

#[test]
fn idle_connection_detects_peer_shutdown_without_sending_a_command() {
    let (connection, peer) = pair();
    assert!(connection.idle());
    peer.shutdown(Shutdown::Both).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while connection.idle() {
        assert!(Instant::now() < deadline, "peer EOF was not detected");
        thread::yield_now();
    }
}

#[test]
fn audio_completes_while_both_control_channels_wait() {
    thread::scope(|scope| {
        let mut blocked = Vec::new();
        for (channel, action) in [
            (Channel::Control, KakaoAction::ChatOnRoom),
            (
                Channel::Vox,
                KakaoAction::VoxCreateRoom {
                    title: "test".into(),
                },
            ),
        ] {
            let (state, commands) = ChannelState::new(1);
            state.connected();
            let response = enqueue(&state, 1, action);
            let command = commands.recv().unwrap();
            let (mut connection, peer) = pair();
            let worker = scope
                .spawn(move || transact_kakao(&mut connection, "test", &command, channel, &state));
            let mut reader = BufReader::new(peer.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap(); // Confirm both transactions are waiting for a reply.
            blocked.push((peer, response, worker));
        }
        let (state, commands) = ChannelState::new(1);
        state.connected();
        for action in [
            KakaoAction::VoxAudioPush {
                encoded: "AAA=".into(),
            },
            KakaoAction::VoxStatus,
        ] {
            let response = enqueue(&state, 2, action);
            let command = commands.recv().unwrap();
            let (mut connection, mut peer) = pair();
            let responder = scope.spawn(move || reply(&mut peer));
            transact_kakao(&mut connection, "test", &command, Channel::Audio, &state).unwrap();
            assert_eq!(
                response.recv_timeout(Duration::from_secs(1)).unwrap(),
                Ok(Some("done".into()))
            );
            responder.join().unwrap();
        }
        for (mut peer, response, worker) in blocked {
            assert!(matches!(
                response.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            writeln!(peer, "{{\"id\":1,\"ok\":true}}").unwrap();
            worker.join().unwrap().unwrap();
            assert_eq!(response.recv().unwrap(), Ok(None));
        }
    });
}

#[test]
fn canceled_and_expired_commands_are_not_written() {
    let (state, commands) = ChannelState::new(2);
    state.connected();
    let canceled = enqueue(&state, 1, KakaoAction::VoxAudioStop);
    state.remove(1);
    let expired = enqueue(&state, 2, KakaoAction::VoxAudioStop);
    let (mut connection, mut peer) = pair();
    transact_kakao(
        &mut connection,
        "test",
        &commands.recv().unwrap(),
        Channel::Audio,
        &state,
    )
    .unwrap();
    let mut command = commands.recv().unwrap();
    command.deadline = Instant::now();
    transact_kakao(&mut connection, "test", &command, Channel::Audio, &state).unwrap();
    assert!(expired.recv().unwrap().is_err());
    assert!(canceled.recv().is_err());
    assert!(state.is_connected());
    connection.stream.shutdown(Shutdown::Write).unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut peer, &mut bytes).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn handshake_rejects_wrong_channel_version_token_and_pid() {
    // No supervisor runs in host tests, so zero is never a valid target PID.
    for (channel, protocol, token, pid) in [
        ("control", VERSION, "test", 0),
        ("audio", 1, "test", 0),
        ("audio", VERSION, "wrong", 0),
        ("audio", VERSION, "test", 0),
    ] {
        let (connection, mut peer) = pair();
        writeln!(peer, "{}", serde_json::json!({
            "event": "ready", "channel": channel, "protocol": protocol, "token": token, "pid": pid,
        })).unwrap();
        let error = accept_kakao_connection(connection.stream, "test", Channel::Audio)
            .err()
            .unwrap();
        if channel == "audio" && protocol == VERSION && token == "test" {
            assert!(error.contains("PID"));
        } else {
            assert!(error.contains("프로토콜"));
        }
    }
}

#[test]
fn each_channel_accepts_the_current_protocol_and_target_pid() {
    // Host tests never launch the Android supervisor. The rejection test uses
    // PID zero, which remains invalid even while this target is set.
    KAKAO_TARGET_PID.store(42, Ordering::Release);
    for channel in Channel::ALL {
        let (connection, mut peer) = pair();
        writeln!(
            peer,
            "{}",
            serde_json::json!({
                "event": "ready", "channel": channel.name(), "protocol": VERSION,
                "token": "test", "pid": 42,
            })
        )
        .unwrap();
        let accepted = accept_kakao_connection(connection.stream, "test", channel).unwrap();
        assert_eq!(accepted.pid, 42);
        assert!(accepted.idle());
    }
    KAKAO_TARGET_PID.store(0, Ordering::Release);
}
