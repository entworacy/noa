use std::{
    io::Write,
    net::{SocketAddr, TcpStream},
    sync::{OnceLock, mpsc},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use jni::sys::{JNIEnv, jint, jobject};

use crate::{
    KIND_LOCO_SEND, instance_field, instance_field_by_type, log, object_text, unbox_number,
};

type Event = serde_json::Value;

static EVENTS: OnceLock<mpsc::SyncSender<Event>> = OnceLock::new();

pub(crate) fn channel() -> (mpsc::SyncSender<Event>, mpsc::Receiver<Event>) {
    mpsc::sync_channel(1024)
}

pub(crate) fn install(sender: mpsc::SyncSender<Event>) {
    let _ = EVENTS.set(sender);
}

pub(crate) fn run(address: SocketAddr, token: String, receiver: mpsc::Receiver<Event>) {
    let mut stream = None;
    while let Ok(mut value) = receiver.recv() {
        let Some(object) = value.as_object_mut() else {
            continue;
        };
        object.insert(
            "token".to_string(),
            serde_json::Value::String(token.clone()),
        );
        object
            .entry("event".to_string())
            .or_insert_with(|| serde_json::Value::String("loco".to_string()));
        let line = value.to_string() + "\n";
        loop {
            if stream.is_none() {
                stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).ok();
            }
            let Some(active) = stream.as_mut() else {
                thread::sleep(Duration::from_millis(500));
                continue;
            };
            if active
                .write_all(line.as_bytes())
                .and_then(|_| active.flush())
                .is_ok()
            {
                break;
            }
            stream = None;
        }
    }
}

pub(crate) unsafe fn capture(env: *mut JNIEnv, kind: jint, packet: jobject) {
    if packet.is_null() {
        return;
    }
    let result = (|| unsafe {
        let header = instance_field_by_type(env, packet, "mt.b")?;
        let body = instance_field_by_type(env, packet, "mt.a")?;
        let packet_id = unbox_number(env, instance_field(env, header, "a")?)?;
        let status = unbox_number(env, instance_field(env, header, "b")?)?;
        let method = object_text(env, instance_field(env, header, "c")?)?;
        let body_length = unbox_number(env, instance_field(env, header, "d")?)?;
        let bson = instance_field(env, body, "a")?;
        let body = object_text(env, bson)?;
        let direction = if kind == KIND_LOCO_SEND {
            "send"
        } else {
            "receive"
        };
        emit(serde_json::json!({
            "direction": direction,
            "method": method,
            "packetId": packet_id,
            "status": status,
            "bodyLength": body_length,
            "body": body,
            "capturedAt": now_millis(),
        }));
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        log(crate::LOG_ERROR, &format!("LOCO capture failed: {error}"));
    }
}

pub(crate) fn database_invalidated(database: String, table: String) {
    emit(serde_json::json!({
        "event": "database-invalidated",
        "database": database,
        "table": table,
        "capturedAt": now_millis(),
    }));
}

fn emit(value: serde_json::Value) {
    if let Some(sender) = EVENTS.get() {
        let _ = sender.try_send(value);
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
