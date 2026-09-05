//! Each transport owns its pending replies and bounded queue.
use std::{
    collections::HashMap,
    sync::{Mutex, mpsc},
};

type Reply = Result<Option<String>, String>;
pub(super) struct Pending {
    pub(super) sender: mpsc::SyncSender<Reply>,
}

struct Session {
    ready: bool,
    pending: HashMap<u64, Pending>,
}

pub(in crate::intercept::runtime) struct ChannelState<T> {
    sender: mpsc::SyncSender<T>,
    session: Mutex<Session>,
}

impl<T> ChannelState<T> {
    pub(in crate::intercept::runtime) fn new(capacity: usize) -> (Self, mpsc::Receiver<T>) {
        let (sender, receiver) = mpsc::sync_channel(capacity);
        (
            Self {
                sender,
                session: Mutex::new(Session {
                    ready: false,
                    pending: HashMap::new(),
                }),
            },
            receiver,
        )
    }

    pub(super) fn connected(&self) {
        self.session.lock().unwrap().ready = true;
    }

    pub(super) fn is_connected(&self) -> bool {
        self.session.lock().is_ok_and(|session| session.ready)
    }

    pub(super) fn enqueue(&self, id: u64, reply: Pending, command: T) -> Result<(), String> {
        let mut session = self.session.lock().map_err(|_| "명령 채널 잠금 손상")?;
        if !session.ready {
            return Err("에이전트가 연결되지 않았습니다".into());
        }
        session.pending.insert(id, reply);
        if let Err(error) = self.sender.try_send(command) {
            session.pending.remove(&id);
            return Err(match error {
                mpsc::TrySendError::Full(_) => "명령 대기열이 가득 찼습니다",
                mpsc::TrySendError::Disconnected(_) => "명령 채널이 종료되었습니다",
            }
            .into());
        }
        Ok(())
    }

    pub(super) fn contains(&self, id: u64) -> bool {
        self.session
            .lock()
            .is_ok_and(|session| session.pending.contains_key(&id))
    }

    pub(super) fn remove(&self, id: u64) {
        if let Ok(mut session) = self.session.lock() {
            session.pending.remove(&id);
        }
    }

    pub(super) fn complete(&self, id: u64, result: Reply) {
        if let Ok(mut session) = self.session.lock()
            && let Some(pending) = session.pending.remove(&id)
        {
            let _ = pending.sender.send(result);
        }
    }

    pub(super) fn disconnect(&self, reason: &str) {
        if let Ok(mut session) = self.session.lock() {
            session.ready = false;
            for (_, pending) in session.pending.drain() {
                let _ = pending.sender.send(Err(reason.into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enqueue(state: &ChannelState<u64>, id: u64) -> Result<mpsc::Receiver<Reply>, String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        state.enqueue(id, Pending { sender }, id)?;
        Ok(receiver)
    }

    #[test]
    fn disconnection_and_backpressure_are_local_to_each_channel() {
        let (control, control_queue) = ChannelState::new(1);
        let (vox, vox_queue) = ChannelState::new(1);
        let (audio, audio_queue) = ChannelState::new(1);
        assert!(enqueue(&audio, 1).is_err());
        for state in [&control, &vox, &audio] {
            state.connected();
        }
        let control_reply = enqueue(&control, 1).unwrap();
        let vox_reply = enqueue(&vox, 2).unwrap();
        let audio_reply = enqueue(&audio, 3).unwrap();
        assert!(enqueue(&audio, 4).unwrap_err().contains("가득"));
        assert!(!audio.contains(4));
        audio.disconnect("lost audio");
        assert_eq!(audio_reply.recv().unwrap(), Err("lost audio".into()));
        assert!(enqueue(&audio, 5).is_err());
        assert!(control.is_connected());
        assert!(vox.is_connected());
        assert!(matches!(
            control_reply.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            vox_reply.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        control.complete(control_queue.recv().unwrap(), Ok(None));
        vox.complete(vox_queue.recv().unwrap(), Ok(None));
        assert_eq!(control_reply.recv().unwrap(), Ok(None));
        assert_eq!(vox_reply.recv().unwrap(), Ok(None));
        audio.connected();
        assert!(!audio.contains(audio_queue.recv().unwrap())); // No replay on reconnect.
        let next = enqueue(&audio, 6).unwrap();
        audio.complete(audio_queue.recv().unwrap(), Ok(None));
        assert_eq!(next.recv().unwrap(), Ok(None));
    }
}
