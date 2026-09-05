//! Independent reconnecting sessions for chat, VOX control, and playback.
use crate::{
    Bootstrap, FailureHello, Hello, LOG_ERROR, commands, ensure_event_bridge,
    initialization_failed, initialize_runtime, log, vox, write_json, write_response,
};
use noa_agent_protocol::{Channel, VERSION};
use std::{
    io::{BufRead, BufReader},
    net::{SocketAddr, TcpStream},
    sync::Once,
    thread,
    time::Duration,
};

static VOX_START: Once = Once::new();

pub(crate) fn serve(config: Bootstrap) {
    serve_channel(config, Channel::Control);
}

fn start_vox(config: &Bootstrap) {
    VOX_START.call_once(|| {
        for channel in [Channel::Vox, Channel::Audio] {
            let config = config.clone();
            if let Err(error) = thread::Builder::new()
                .name(format!("noa-kakao-{}", channel.name()))
                .spawn(move || serve_channel(config, channel))
            {
                log(
                    LOG_ERROR,
                    &format!("{} channel start failed: {error}", channel.name()),
                );
            }
        }
    });
}

fn serve_channel(config: Bootstrap, channel: Channel) {
    let port = match channel {
        Channel::Control => config.port,
        Channel::Vox => config.vox_port,
        Channel::Audio => config.audio_port,
    };
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    loop {
        match TcpStream::connect_timeout(&address, Duration::from_secs(2)) {
            Ok(mut stream) => {
                if let Err(error) = session(&mut stream, &config, channel) {
                    if channel == Channel::Audio {
                        vox::discard_audio();
                    }
                    log(LOG_ERROR, &format!("{} channel: {error}", channel.name()));
                    if channel == Channel::Control && initialization_failed() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(250));
                }
            }
            Err(_) => thread::sleep(Duration::from_millis(250)),
        }
    }
}

fn session(stream: &mut TcpStream, config: &Bootstrap, channel: Channel) -> Result<(), String> {
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    if channel == Channel::Control {
        if let Err(error) = initialize_runtime() {
            let failure = FailureHello {
                event: "error",
                token: &config.token,
                pid: std::process::id(),
                protocol: VERSION,
                channel: channel.name(),
                error: &error,
                retryable: !initialization_failed(),
            };
            let _ = write_json(stream, &failure);
            return Err(error);
        }
        ensure_event_bridge(config);
        start_vox(config);
    }
    command_session(stream, config, channel, commands::execute)
}

fn command_session(
    stream: &mut TcpStream,
    config: &Bootstrap,
    channel: Channel,
    mut execute: impl FnMut(commands::Request) -> (u64, commands::CommandResult),
) -> Result<(), String> {
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    write_json(
        stream,
        &Hello {
            event: "ready",
            token: &config.token,
            pid: std::process::id(),
            protocol: VERSION,
            channel: channel.name(),
        },
    )?;
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("Noa bridge disconnected".into());
        }
        let request = match serde_json::from_str::<commands::Request>(line.trim()) {
            Ok(request) => request,
            Err(error) => {
                write_response(stream, 0, Err(format!("invalid command: {error}")))?;
                continue;
            }
        };
        if request.token != config.token {
            write_response(stream, request.id, Err("authentication failed".into()))?;
            continue;
        }
        if request.channel() != Some(channel) {
            write_response(
                stream,
                request.id,
                Err("command is not allowed on this channel".into()),
            )?;
            continue;
        }
        let (id, result) = execute(request);
        write_response(stream, id, result)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        io::Write,
        net::{Shutdown, TcpListener},
    };

    #[test]
    fn audio_session_authenticates_and_rejects_other_channel_actions() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let worker = thread::spawn(move || {
            command_session(
                &mut server,
                &Bootstrap {
                    port: 0,
                    event_port: 0,
                    vox_port: 0,
                    audio_port: 0,
                    token: "test".into(),
                },
                Channel::Audio,
                |request| {
                    assert_eq!(request.channel(), Some(Channel::Audio));
                    (request.id, Ok(Some("accepted".into())))
                },
            )
        });
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let hello: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(hello["protocol"], VERSION);
        assert_eq!(hello["channel"], "audio");
        for (action, token, expected) in [
            ("vox-audio-start", "wrong", "authentication failed"),
            (
                "send-custom",
                "test",
                "command is not allowed on this channel",
            ),
            (
                "vox-create-room",
                "test",
                "command is not allowed on this channel",
            ),
            ("unknown", "test", "command is not allowed on this channel"),
        ] {
            writeln!(
                client,
                "{}",
                json!({"id": 42, "token": token, "action": action, "mode": "replace"})
            )
            .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            let response: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(response["id"], 42);
            assert_eq!(response["ok"], false);
            assert_eq!(response["error"], expected);
        }
        writeln!(
            client,
            "{}",
            json!({"id": 43, "token": "test", "action": "vox-status"})
        )
        .unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 43);
        assert_eq!(response["value"], "accepted");
        assert_eq!(response["ok"], true);
        client.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            "Noa bridge disconnected"
        );
    }
}
