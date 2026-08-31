use std::{
    collections::VecDeque,
    ffi::c_void,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Mutex, MutexGuard, OnceLock},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use jni::sys::{JNIEnv, jclass, jint, jobject};
use serde::Serialize;

use crate::{
    RUNTIME, app_class, check, find_class, find_exact_method, new_object, noa_lsplant_hook,
};

const MAX_AUDIO_QUEUE_BYTES: usize = 192_000;
const MAX_AUDIO_PUSH_BYTES: usize = 96_000;
static HOOK_READY: AtomicBool = AtomicBool::new(false);
static AUDIO: OnceLock<Mutex<AudioState>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum AudioMode {
    Replace,
    Mix,
}

#[derive(Debug)]
struct AudioState {
    active: bool,
    mode: AudioMode,
    queue: VecDeque<u8>,
    pushed_bytes: u64,
    dropped_bytes: u64,
    underflow_bytes: u64,
    processed_frames: u64,
    last_frame_bytes: usize,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            active: false,
            mode: AudioMode::Replace,
            queue: VecDeque::with_capacity(MAX_AUDIO_QUEUE_BYTES),
            pushed_bytes: 0,
            dropped_bytes: 0,
            underflow_bytes: 0,
            processed_frames: 0,
            last_frame_bytes: 0,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AudioStatus {
    hook_ready: bool,
    active: bool,
    mode: AudioMode,
    queued_bytes: usize,
    max_queue_bytes: usize,
    pushed_bytes: u64,
    dropped_bytes: u64,
    underflow_bytes: u64,
    processed_frames: u64,
    last_frame_bytes: usize,
    format: &'static str,
}

pub(crate) unsafe fn install_hook(env: *mut JNIEnv) -> Result<(), String> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| "runtime is unavailable".to_string())?;
    let audio_record = unsafe { find_class(env, "android/media/AudioRecord")? };
    let target =
        unsafe { find_exact_method(env, audio_record, "read", &["java.nio.ByteBuffer", "int"])? };
    let hooker_class = unsafe { app_class(env, "dev.noa.kakao.VoxAudioHooker")? };
    let hooker = unsafe { new_object(env, hooker_class, "()V", &[])? };
    let callback =
        unsafe { find_exact_method(env, hooker_class, "callback", &["[Ljava.lang.Object;"])? };
    let backup = unsafe {
        noa_lsplant_hook(
            env,
            runtime.lsplant as *mut c_void,
            target,
            hooker,
            callback,
        )
    };
    unsafe { check(env, "install VOX AudioRecord hook")? };
    if backup.is_null() {
        return Err("LSPlant returned no backup for VOX AudioRecord.read".to_string());
    }
    let field = unsafe {
        ((**env).v1_4.GetFieldID)(
            env,
            hooker_class,
            c"backup".as_ptr(),
            c"Ljava/lang/reflect/Method;".as_ptr(),
        )
    };
    unsafe { check(env, "resolve VoxAudioHooker.backup")? };
    unsafe { ((**env).v1_4.SetObjectField)(env, hooker, field, backup) };
    unsafe { check(env, "store VoxAudioHooker.backup")? };
    let retained = unsafe { ((**env).v1_4.NewGlobalRef)(env, hooker) };
    unsafe { check(env, "retain VOX audio hook")? };
    if retained.is_null() {
        return Err("VOX audio hook global reference is null".to_string());
    }
    HOOK_READY.store(true, Ordering::Release);
    Ok(())
}

pub(crate) fn start_audio(mode: &str) -> Result<String, String> {
    if !HOOK_READY.load(Ordering::Acquire) {
        return Err("VOX audio hook is unavailable".to_string());
    }
    let mode = match mode {
        "replace" => AudioMode::Replace,
        "mix" => AudioMode::Mix,
        _ => return Err("VOX audio mode must be replace or mix".to_string()),
    };
    let mut state = audio();
    if state.active {
        return Err("VOX audio injection is already active".to_string());
    }
    *state = AudioState {
        active: true,
        mode,
        ..AudioState::default()
    };
    serialize(&state)
}

pub(crate) fn push_audio(encoded: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| format!("decode VOX PCM: {error}"))?;
    if bytes.is_empty() {
        return Err("VOX PCM chunk is empty".to_string());
    }
    if bytes.len() > MAX_AUDIO_PUSH_BYTES {
        return Err(format!(
            "VOX PCM chunk exceeds {MAX_AUDIO_PUSH_BYTES} bytes"
        ));
    }
    if bytes.len() % 2 != 0 {
        return Err("VOX PCM chunk must contain complete 16-bit samples".to_string());
    }
    let mut state = audio();
    if !state.active {
        return Err("VOX audio injection is not active".to_string());
    }
    state.pushed_bytes = state.pushed_bytes.saturating_add(bytes.len() as u64);
    let overflow = state
        .queue
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(MAX_AUDIO_QUEUE_BYTES);
    for _ in 0..overflow {
        state.queue.pop_front();
    }
    state.dropped_bytes = state.dropped_bytes.saturating_add(overflow as u64);
    state.queue.extend(bytes);
    serialize(&state)
}

pub(crate) fn stop_audio() -> Result<String, String> {
    let mut state = audio();
    state.active = false;
    state.queue.clear();
    serialize(&state)
}

pub(crate) unsafe extern "system" fn process_audio(
    env: *mut JNIEnv,
    _: jclass,
    buffer: jobject,
    size: jint,
) {
    if buffer.is_null() || size <= 0 {
        return;
    }
    let mut state = audio();
    if !state.active || state.queue.is_empty() {
        return;
    }
    let address = unsafe { ((**env).v1_4.GetDirectBufferAddress)(env, buffer) } as *mut u8;
    let capacity = unsafe { ((**env).v1_4.GetDirectBufferCapacity)(env, buffer) };
    if address.is_null() || capacity <= 0 {
        return;
    }
    let count = (size as usize).min(capacity as usize);
    state.processed_frames = state.processed_frames.saturating_add(1);
    state.last_frame_bytes = count;
    match state.mode {
        AudioMode::Replace => replace(&mut state, address, count),
        AudioMode::Mix => mix(&mut state, address, count),
    }
}

fn replace(state: &mut AudioState, address: *mut u8, count: usize) {
    let mut missing = 0_u64;
    for index in 0..count {
        let value = match state.queue.pop_front() {
            Some(value) => value,
            None => {
                missing += 1;
                0
            }
        };
        unsafe { ptr::write(address.add(index), value) };
    }
    state.underflow_bytes = state.underflow_bytes.saturating_add(missing);
}

fn mix(state: &mut AudioState, address: *mut u8, count: usize) {
    let mut index = 0;
    while index + 1 < count {
        let Some(low) = state.queue.pop_front() else {
            state.underflow_bytes = state.underflow_bytes.saturating_add((count - index) as u64);
            break;
        };
        let Some(high) = state.queue.pop_front() else {
            state.queue.push_front(low);
            state.underflow_bytes = state.underflow_bytes.saturating_add((count - index) as u64);
            break;
        };
        let original = i16::from_le_bytes(unsafe {
            [
                ptr::read(address.add(index)),
                ptr::read(address.add(index + 1)),
            ]
        });
        let injected = i16::from_le_bytes([low, high]);
        let mixed =
            (original as i32 + injected as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let bytes = mixed.to_le_bytes();
        unsafe {
            ptr::write(address.add(index), bytes[0]);
            ptr::write(address.add(index + 1), bytes[1]);
        }
        index += 2;
    }
}

pub(super) fn status() -> AudioStatus {
    let state = audio();
    status_for(&state)
}

fn serialize(state: &AudioState) -> Result<String, String> {
    serde_json::to_string(&status_for(state)).map_err(|error| error.to_string())
}

fn status_for(state: &AudioState) -> AudioStatus {
    AudioStatus {
        hook_ready: HOOK_READY.load(Ordering::Acquire),
        active: state.active,
        mode: state.mode,
        queued_bytes: state.queue.len(),
        max_queue_bytes: MAX_AUDIO_QUEUE_BYTES,
        pushed_bytes: state.pushed_bytes,
        dropped_bytes: state.dropped_bytes,
        underflow_bytes: state.underflow_bytes,
        processed_frames: state.processed_frames,
        last_frame_bytes: state.last_frame_bytes,
        format: "s16le/48000/mono",
    }
}

fn audio() -> MutexGuard<'static, AudioState> {
    AUDIO
        .get_or_init(|| Mutex::new(AudioState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{
        AudioMode, AudioState, MAX_AUDIO_QUEUE_BYTES, mix, replace, start_audio, stop_audio,
    };

    #[test]
    fn audio_queue_has_a_bounded_realtime_capacity() {
        assert_eq!(MAX_AUDIO_QUEUE_BYTES, 192_000);
        // The platform hook is intentionally required before activation.
        assert_eq!(
            start_audio("replace").unwrap_err(),
            "VOX audio hook is unavailable"
        );
        assert!(stop_audio().is_ok());
    }

    #[test]
    fn replace_uses_silence_on_underflow() {
        let mut state = AudioState {
            active: true,
            queue: VecDeque::from([1, 2]),
            ..AudioState::default()
        };
        let mut frame = [9_u8; 4];
        replace(&mut state, frame.as_mut_ptr(), frame.len());
        assert_eq!(frame, [1, 2, 0, 0]);
        assert_eq!(state.underflow_bytes, 2);
    }

    #[test]
    fn mix_saturates_signed_16_bit_samples() {
        let mut state = AudioState {
            active: true,
            mode: AudioMode::Mix,
            queue: VecDeque::from(i16::MAX.to_le_bytes()),
            ..AudioState::default()
        };
        let mut frame = 1_i16.to_le_bytes();
        mix(&mut state, frame.as_mut_ptr(), frame.len());
        assert_eq!(i16::from_le_bytes(frame), i16::MAX);
    }
}
