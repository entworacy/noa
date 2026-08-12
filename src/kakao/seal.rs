use std::{
    collections::HashMap,
    fmt::Write,
    sync::{Arc, RwLock},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::{
    hash::{Hasher, MessageDigest},
    pkcs5,
    symm::{Cipher, decrypt},
};
use prost::Message;

const ENC_NAMESPACES: [&str; 32] = [
    "",
    "",
    "12",
    "24",
    "18",
    "30",
    "36",
    "12",
    "48",
    "7",
    "35",
    "40",
    "17",
    "23",
    "29",
    "isabel",
    "kale",
    "sulli",
    "van",
    "merry",
    "kyle",
    "james",
    "maddux",
    "tony",
    "hayden",
    "paul",
    "elijah",
    "dorothy",
    "sally",
    "bran",
    "extr.ursra",
    "veil",
];
const MESSAGE_SECRET: [u8; 34] = [
    0, 22, 0, 8, 0, 9, 0, 111, 0, 2, 0, 23, 0, 43, 0, 8, 0, 33, 0, 33, 0, 10, 0, 16, 0, 3, 0, 3, 0,
    7, 0, 6, 0, 0,
];
const MESSAGE_VECTOR: [u8; 16] = [
    15, 8, 1, 0, 25, 71, 37, 220, 21, 245, 23, 224, 225, 21, 12, 53,
];
const PROFILE_DATABASE_SECRET: [u8; 16] = [
    4, 15, 81, 123, 77, 5, 23, 99, 2, 111, 10, 31, 54, 29, 109, 97,
];

#[derive(Clone, PartialEq, Message)]
struct PreferenceLedger {
    #[prost(map = "string, message", tag = "1")]
    entries: HashMap<String, EncodedPreference>,
}

#[derive(Clone, PartialEq, Message)]
struct EncodedPreference {
    #[prost(oneof = "preference_payload::Payload", tags = "1, 2, 3, 4, 5, 6, 7, 8")]
    payload: Option<preference_payload::Payload>,
}

mod preference_payload {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Payload {
        #[prost(bool, tag = "1")]
        Boolean(bool),
        #[prost(float, tag = "2")]
        Float(f32),
        #[prost(int32, tag = "3")]
        Integer(i32),
        #[prost(int64, tag = "4")]
        Long(i64),
        #[prost(string, tag = "5")]
        String(String),
        #[prost(message, tag = "6")]
        Strings(super::StringGroup),
        #[prost(double, tag = "7")]
        Double(f64),
        #[prost(bytes, tag = "8")]
        Bytes(Vec<u8>),
    }
}

#[derive(Clone, PartialEq, Message)]
struct StringGroup {
    #[prost(string, repeated, tag = "1")]
    values: Vec<String>,
}

pub fn profile_database_password(preferences: &[u8]) -> Result<String, String> {
    let ledger = PreferenceLedger::decode(preferences)
        .map_err(|error| format!("KakaoTalk 설정 해석 실패: {error}"))?;
    let seed = match ledger
        .entries
        .get("userDbPassPhraseSalt")
        .and_then(|entry| entry.payload.as_ref())
    {
        Some(preference_payload::Payload::Long(seed)) => *seed,
        _ => return Err("사용자 DB salt를 찾지 못했습니다".to_string()),
    };
    let mut salt = format!("se{seed}ed").into_bytes();
    salt.resize(16, 0);
    let mut derived = [0_u8; 32];
    pkcs5::pbkdf2_hmac(
        &PROFILE_DATABASE_SECRET,
        &salt,
        4096,
        MessageDigest::sha256(),
        &mut derived,
    )
    .map_err(|error| format!("사용자 DB 암호 생성 실패: {error}"))?;
    let mut encoded = String::with_capacity(64);
    for byte in derived {
        write!(&mut encoded, "{byte:02x}").map_err(|error| error.to_string())?;
    }
    Ok(encoded)
}

type ContentKey = [u8; 32];

#[derive(Clone)]
pub struct SealedText {
    owner: i64,
    cache: Arc<RwLock<HashMap<(u32, i64), ContentKey>>>,
}

impl SealedText {
    pub fn for_owner(owner: i64) -> Self {
        Self {
            owner,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn profile(&self, value: &str, scheme: u32) -> String {
        self.reveal(value, scheme, self.owner)
    }

    pub fn reveal(&self, value: &str, scheme: u32, owner: i64) -> String {
        self.open(value, scheme, owner)
            .unwrap_or_else(|_| value.to_string())
    }

    fn open(&self, value: &str, scheme: u32, owner: i64) -> Result<String, String> {
        if value.is_empty() || matches!(value, "{}" | "[]") {
            return Err("복호화할 값이 없습니다".to_string());
        }
        let encrypted = STANDARD.decode(value).map_err(|error| error.to_string())?;
        if encrypted.is_empty() {
            return Err("복호화할 값이 없습니다".to_string());
        }
        let key = self.content_key(scheme, owner)?;
        let clear = decrypt(
            Cipher::aes_256_cbc(),
            &key,
            Some(&MESSAGE_VECTOR),
            &encrypted,
        )
        .map_err(|error| error.to_string())?;
        Ok(String::from_utf8_lossy(&clear).into_owned())
    }

    fn content_key(&self, scheme: u32, owner: i64) -> Result<ContentKey, String> {
        if let Some(existing) = self
            .cache
            .read()
            .map_err(|_| "복호화 키 캐시를 읽을 수 없습니다".to_string())?
            .get(&(scheme, owner))
            .copied()
        {
            return Ok(existing);
        }
        let namespace = ENC_NAMESPACES
            .get(scheme as usize)
            .ok_or_else(|| format!("지원하지 않는 enc 값: {scheme}"))?;
        let mut salt = format!("{namespace}{owner}").into_bytes();
        salt.resize(16, 0);
        let material = expand_legacy_material(&MESSAGE_SECRET, &salt, 32)?;
        let key: ContentKey = material
            .try_into()
            .map_err(|_| "복호화 키 길이가 올바르지 않습니다".to_string())?;
        self.cache
            .write()
            .map_err(|_| "복호화 키 캐시를 갱신할 수 없습니다".to_string())?
            .insert((scheme, owner), key);
        Ok(key)
    }
}

fn expand_legacy_material(password: &[u8], salt: &[u8], length: usize) -> Result<Vec<u8>, String> {
    const DIGEST_SIZE: usize = 20;
    const CHUNK_SIZE: usize = 64;

    let diversifier = [1_u8; CHUNK_SIZE];
    let mut state = repeat_to_multiple(salt, CHUNK_SIZE);
    state.extend(repeat_to_multiple(password, CHUNK_SIZE));
    let mut output = Vec::with_capacity(length.div_ceil(DIGEST_SIZE) * DIGEST_SIZE);
    while output.len() < length {
        let digest = sha1_twice(&diversifier, &state)?;
        let adjustment: Vec<u8> = digest.iter().copied().cycle().take(CHUNK_SIZE).collect();
        for chunk in state.chunks_exact_mut(CHUNK_SIZE) {
            add_big_endian(chunk, &adjustment);
        }
        output.extend_from_slice(&digest);
    }
    output.truncate(length);
    Ok(output)
}

fn repeat_to_multiple(value: &[u8], width: usize) -> Vec<u8> {
    if value.is_empty() {
        return Vec::new();
    }
    let length = value.len().div_ceil(width) * width;
    value.iter().copied().cycle().take(length).collect()
}

fn sha1_twice(prefix: &[u8], state: &[u8]) -> Result<Vec<u8>, String> {
    let mut first = Hasher::new(MessageDigest::sha1()).map_err(|error| error.to_string())?;
    first.update(prefix).map_err(|error| error.to_string())?;
    first.update(state).map_err(|error| error.to_string())?;
    let first = first.finish().map_err(|error| error.to_string())?;
    let mut second = Hasher::new(MessageDigest::sha1()).map_err(|error| error.to_string())?;
    second.update(&first).map_err(|error| error.to_string())?;
    Ok(second.finish().map_err(|error| error.to_string())?.to_vec())
}

fn add_big_endian(target: &mut [u8], increment: &[u8]) {
    let mut carry = 1_u16;
    for (target, increment) in target.iter_mut().rev().zip(increment.iter().rev()) {
        let value = u16::from(*target) + u16::from(*increment) + carry;
        *target = value as u8;
        carry = value >> 8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_material_matches_known_vector() {
        let mut salt = b"12123".to_vec();
        salt.resize(16, 0);
        let material = expand_legacy_material(&MESSAGE_SECRET, &salt, 32).unwrap();
        assert_eq!(
            material,
            [
                0x5c, 0x18, 0xdf, 0xa5, 0x7a, 0x8c, 0x31, 0x16, 0x54, 0x1b, 0x91, 0x9f, 0x94, 0x8a,
                0xef, 0x6f, 0x9a, 0xe8, 0xe6, 0xcc, 0xe7, 0xd9, 0x48, 0xa3, 0x60, 0x3a, 0x01, 0x13,
                0xbc, 0xaf, 0xc2, 0x28,
            ]
        );
    }
}
