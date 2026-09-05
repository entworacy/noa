//! Shared session signals; feature policy stays in each adapter.
use std::sync::atomic::{AtomicU32, AtomicU64};
pub(super) static NEXT_COMMAND_ID: AtomicU64 = AtomicU64::new(1);
pub(super) static IRIS_FAILED_PID: AtomicU32 = AtomicU32::new(0);
pub(super) static KAKAO_FATAL_PID: AtomicU32 = AtomicU32::new(0);
pub(super) static KAKAO_TARGET_PID: AtomicU32 = AtomicU32::new(0);
