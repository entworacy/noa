use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::c_char,
    fs::File,
    io::Read,
    path::PathBuf,
};

use jni::sys::{JNIEnv, jclass, jobject, jobjectArray};
use zip::ZipArchive;

use crate::{
    call_object, call_static_object, call_static_void, check, find_class, java_string, new_string,
    object_value,
};

mod dex_format;
use dex_format::{
    MethodCode, dex_string, is_dex, little_usize, read_method_code_items, table_fits,
    unsigned_short,
};

const TARGET_SOURCES: &[&str] = &[
    "ChatRoomListManager.kt",
    "ChatRoomApiHelper.kt",
    "OpenChatMemberRepository.kt",
    "OlkManager.kt",
    "FeedType.kt",
    "OlkOpenProfileRepository.kt",
    "ConnectionOpenLinkJoin.kt",
    "OpenLinkTypes.kt",
    "ChatSendingLogManager.kt",
    "ChatSendingLogRequest.kt",
    "OpenLink.kt",
    "LocoClient.kt",
    "LocoState.kt",
    "Loco.kt",
];
const MAX_DEX_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct MethodRef {
    owner: String,
    name: String,
}

#[derive(Default)]
struct SignatureIndex {
    classes_by_source: BTreeMap<String, Vec<String>>,
    operations: BTreeMap<String, BTreeSet<MethodRef>>,
    apk_count: usize,
    dex_count: usize,
}

impl SignatureIndex {
    fn read_apks(paths: &[PathBuf]) -> Result<Self, String> {
        let targets = TARGET_SOURCES.iter().copied().collect::<BTreeSet<_>>();
        let mut index = Self::default();
        for path in paths {
            let Ok(file) = File::open(path) else {
                continue;
            };
            let Ok(mut archive) = ZipArchive::new(file) else {
                continue;
            };
            index.apk_count += 1;
            let mut dex_entries = Vec::new();
            for entry_index in 0..archive.len() {
                let Ok(entry) = archive.by_index(entry_index) else {
                    continue;
                };
                let name = entry.name();
                if name.starts_with("classes") && name.ends_with(".dex") {
                    dex_entries.push(name.to_string());
                }
            }
            dex_entries.sort();
            dex_entries.dedup();
            for name in dex_entries {
                let Ok(mut entry) = archive.by_name(&name) else {
                    continue;
                };
                if entry.size() > MAX_DEX_BYTES {
                    continue;
                }
                let mut dex = Vec::with_capacity(entry.size() as usize);
                if entry.read_to_end(&mut dex).is_err() {
                    continue;
                }
                index.dex_count += 1;
                index_sources(&dex, &targets, &mut index.classes_by_source);
                index_operations(&dex, &mut index.operations);
            }
        }
        if index.dex_count == 0 {
            return Err(format!(
                "no readable DEX files in {} application APK paths",
                paths.len()
            ));
        }
        Ok(index)
    }

    fn encode(&self) -> Result<String, String> {
        let mut rows = Vec::new();
        for (source, classes) in &self.classes_by_source {
            let mut columns = vec!["S", source.as_str()];
            columns.extend(classes.iter().map(String::as_str));
            validate_columns(&columns)?;
            rows.push(columns.join("\t"));
        }
        for (operation, methods) in &self.operations {
            let mut columns = vec!["O", operation.as_str()];
            for method in methods {
                columns.push(method.owner.as_str());
                columns.push(method.name.as_str());
            }
            validate_columns(&columns)?;
            rows.push(columns.join("\t"));
        }
        Ok(rows.join("\n"))
    }

    fn description(&self) -> String {
        let candidates = self.classes_by_source.values().map(Vec::len).sum::<usize>();
        let operations = self.operations.values().map(BTreeSet::len).sum::<usize>();
        format!(
            "apk-paths={}, dex-files={}, source-signatures={}, candidates={}, operation-targets={}",
            self.apk_count,
            self.dex_count,
            self.classes_by_source.len(),
            candidates,
            operations
        )
    }
}

fn validate_columns(columns: &[&str]) -> Result<(), String> {
    if let Some(value) = columns
        .iter()
        .find(|value| value.contains(['\t', '\n', '\0']))
    {
        return Err(format!(
            "DEX index value contains a reserved delimiter: {value:?}"
        ));
    }
    Ok(())
}

pub(crate) unsafe fn install(env: *mut JNIEnv, resolver: jclass) -> Result<String, String> {
    let paths = unsafe { application_apk_paths(env)? };
    let index = SignatureIndex::read_apks(&paths)?;
    let encoded = index.encode()?;
    let encoded = unsafe { new_string(env, &encoded)? };
    unsafe {
        call_static_void(
            env,
            resolver,
            "installIndex",
            "(Ljava/lang/String;)V",
            &[object_value(encoded.cast())],
        )?;
    }
    Ok(index.description())
}

unsafe fn application_apk_paths(env: *mut JNIEnv) -> Result<Vec<PathBuf>, String> {
    let activity_thread = unsafe { find_class(env, "android/app/ActivityThread")? };
    let application = unsafe {
        call_static_object(
            env,
            activity_thread,
            "currentApplication",
            "()Landroid/app/Application;",
            &[],
        )?
    };
    if application.is_null() {
        return Err("Android application is not ready".to_string());
    }
    let info = unsafe {
        call_object(
            env,
            application,
            "getApplicationInfo",
            "()Landroid/content/pm/ApplicationInfo;",
            &[],
        )?
    };
    let source = unsafe {
        object_field(
            env,
            info,
            c"sourceDir".as_ptr(),
            c"Ljava/lang/String;".as_ptr(),
        )?
    };
    if source.is_null() {
        return Err("ApplicationInfo.sourceDir is null".to_string());
    }
    let mut paths = vec![PathBuf::from(unsafe { java_string(env, source.cast())? })];
    let splits = unsafe {
        object_field(
            env,
            info,
            c"splitSourceDirs".as_ptr(),
            c"[Ljava/lang/String;".as_ptr(),
        )?
    } as jobjectArray;
    if !splits.is_null() {
        let count = unsafe { ((**env).v1_4.GetArrayLength)(env, splits) };
        unsafe { check(env, "read split APK paths")? };
        for index in 0..count {
            let value = unsafe { ((**env).v1_4.GetObjectArrayElement)(env, splits, index) };
            unsafe { check(env, "read split APK path")? };
            if !value.is_null() {
                paths.push(PathBuf::from(unsafe { java_string(env, value.cast())? }));
                unsafe { ((**env).v1_4.DeleteLocalRef)(env, value) };
            }
        }
    }
    paths.retain(|path| !path.as_os_str().is_empty());
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    Ok(unique)
}

unsafe fn object_field(
    env: *mut JNIEnv,
    target: jobject,
    name: *const c_char,
    signature: *const c_char,
) -> Result<jobject, String> {
    let class = unsafe { ((**env).v1_4.GetObjectClass)(env, target) };
    unsafe { check(env, "resolve ApplicationInfo class")? };
    let field = unsafe { ((**env).v1_4.GetFieldID)(env, class, name, signature) };
    unsafe { check(env, "resolve ApplicationInfo field")? };
    if field.is_null() {
        return Err("ApplicationInfo field was not found".to_string());
    }
    let value = unsafe { ((**env).v1_4.GetObjectField)(env, target, field) };
    unsafe { check(env, "read ApplicationInfo field")? };
    Ok(value)
}

fn index_sources(
    dex: &[u8],
    targets: &BTreeSet<&str>,
    classes_by_source: &mut BTreeMap<String, Vec<String>>,
) {
    if !is_dex(dex) {
        return;
    }
    let Some(string_count) = little_usize(dex, 0x38) else {
        return;
    };
    let Some(string_offset) = little_usize(dex, 0x3c) else {
        return;
    };
    let Some(type_count) = little_usize(dex, 0x40) else {
        return;
    };
    let Some(type_offset) = little_usize(dex, 0x44) else {
        return;
    };
    let Some(class_count) = little_usize(dex, 0x60) else {
        return;
    };
    let Some(class_offset) = little_usize(dex, 0x64) else {
        return;
    };
    if !table_fits(dex, string_offset, string_count, 4)
        || !table_fits(dex, type_offset, type_count, 4)
        || !table_fits(dex, class_offset, class_count, 32)
    {
        return;
    }

    let mut strings = HashMap::new();
    for index in 0..class_count {
        let item = class_offset + index * 32;
        let Some(class_index) = little_usize(dex, item) else {
            continue;
        };
        let Some(source_index) = little_usize(dex, item + 16) else {
            continue;
        };
        if class_index >= type_count || source_index >= string_count {
            continue;
        }
        let source = dex_string(dex, string_offset, source_index, &mut strings);
        if !targets.contains(source.as_str()) {
            continue;
        }
        let Some(descriptor_index) = little_usize(dex, type_offset + class_index * 4) else {
            continue;
        };
        if descriptor_index >= string_count {
            continue;
        }
        let descriptor = dex_string(dex, string_offset, descriptor_index, &mut strings);
        let Some(class_name) = descriptor
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
        else {
            continue;
        };
        let class_name = class_name.replace('/', ".");
        let classes = classes_by_source.entry(source).or_default();
        if !classes.contains(&class_name) {
            classes.push(class_name);
        }
    }
}

fn index_operations(dex: &[u8], operations: &mut BTreeMap<String, BTreeSet<MethodRef>>) {
    if !is_dex(dex) {
        return;
    }
    let Some(string_count) = little_usize(dex, 0x38) else {
        return;
    };
    let Some(string_offset) = little_usize(dex, 0x3c) else {
        return;
    };
    let Some(type_count) = little_usize(dex, 0x40) else {
        return;
    };
    let Some(type_offset) = little_usize(dex, 0x44) else {
        return;
    };
    let Some(field_count) = little_usize(dex, 0x50) else {
        return;
    };
    let Some(field_offset) = little_usize(dex, 0x54) else {
        return;
    };
    let Some(method_count) = little_usize(dex, 0x58) else {
        return;
    };
    let Some(method_offset) = little_usize(dex, 0x5c) else {
        return;
    };
    let Some(class_count) = little_usize(dex, 0x60) else {
        return;
    };
    let Some(class_offset) = little_usize(dex, 0x64) else {
        return;
    };
    if !table_fits(dex, string_offset, string_count, 4)
        || !table_fits(dex, type_offset, type_count, 4)
        || !table_fits(dex, field_offset, field_count, 8)
        || !table_fits(dex, method_offset, method_count, 8)
        || !table_fits(dex, class_offset, class_count, 32)
    {
        return;
    }

    let mut strings = HashMap::new();
    let mut protocol_fields = HashMap::new();
    for index in 0..field_count {
        let Some(name_index) = little_usize(dex, field_offset + index * 8 + 4) else {
            continue;
        };
        if name_index >= string_count {
            continue;
        }
        let name = dex_string(dex, string_offset, name_index, &mut strings);
        if name == "CHATONROOM" || name == "JOINLINK" {
            protocol_fields.insert(index, name);
        }
    }

    let mut method_refs = vec![None; method_count];
    for (index, slot) in method_refs.iter_mut().enumerate() {
        let item = method_offset + index * 8;
        let Some(owner_index) = unsigned_short(dex, item) else {
            continue;
        };
        let Some(name_index) = little_usize(dex, item + 4) else {
            continue;
        };
        if owner_index >= type_count || name_index >= string_count {
            continue;
        }
        let Some(descriptor_index) = little_usize(dex, type_offset + owner_index * 4) else {
            continue;
        };
        let descriptor = dex_string(dex, string_offset, descriptor_index, &mut strings);
        let Some(owner) = descriptor
            .strip_prefix('L')
            .and_then(|value| value.strip_suffix(';'))
        else {
            continue;
        };
        *slot = Some(MethodRef {
            owner: owner.replace('/', "."),
            name: dex_string(dex, string_offset, name_index, &mut strings),
        });
    }

    let mut helper_methods = BTreeMap::new();
    let mut sending_methods = BTreeMap::new();
    for index in 0..class_count {
        let item = class_offset + index * 32;
        let Some(source_index) = little_usize(dex, item + 16) else {
            continue;
        };
        let Some(class_data) = little_usize(dex, item + 24) else {
            continue;
        };
        if source_index >= string_count || class_data == 0 {
            continue;
        }
        match dex_string(dex, string_offset, source_index, &mut strings).as_str() {
            "ChatRoomApiHelper.kt" => read_method_code_items(dex, class_data, &mut helper_methods),
            "ChatSendingLogManager.kt" => {
                read_method_code_items(dex, class_data, &mut sending_methods)
            }
            _ => {}
        }
    }

    let mut direct = BTreeMap::<String, BTreeSet<usize>>::new();
    for (method_index, code) in &helper_methods {
        for field in &code.fields {
            if let Some(semantic) = protocol_fields.get(field) {
                direct
                    .entry(semantic.clone())
                    .or_default()
                    .insert(*method_index);
            }
        }
    }
    add_callers(
        "chat-on-room",
        direct.get("CHATONROOM"),
        &helper_methods,
        &method_refs,
        operations,
    );
    add_callers(
        "join-link",
        direct.get("JOINLINK"),
        &helper_methods,
        &method_refs,
        operations,
    );
    find_resend_operations(&sending_methods, &method_refs, operations);
}

fn find_resend_operations(
    methods: &BTreeMap<usize, MethodCode>,
    refs: &[Option<MethodRef>],
    operations: &mut BTreeMap<String, BTreeSet<MethodRef>>,
) {
    for (index, code) in methods {
        let Some(owner) = ref_at(refs, *index) else {
            continue;
        };
        if owner.owner.contains('$') {
            continue;
        }
        let mut chat_log_call = false;
        let mut own_bridge = false;
        let mut inner_call = false;
        for called_index in &code.calls {
            let Some(called) = ref_at(refs, *called_index) else {
                continue;
            };
            if called.owner == "com.kakao.talk.manager.send.sending.ChatSendingLog" {
                chat_log_call = true;
            } else if called.owner == owner.owner {
                own_bridge = true;
            } else if called.owner.starts_with(&format!("{}$", owner.owner)) {
                inner_call = true;
            }
        }
        if chat_log_call && own_bridge && inner_call {
            operations
                .entry("prepare-resend".to_string())
                .or_default()
                .insert(owner.clone());
        }
    }
}

fn add_callers(
    operation: &str,
    direct: Option<&BTreeSet<usize>>,
    methods: &BTreeMap<usize, MethodCode>,
    refs: &[Option<MethodRef>],
    operations: &mut BTreeMap<String, BTreeSet<MethodRef>>,
) {
    let Some(direct) = direct.filter(|value| !value.is_empty()) else {
        return;
    };
    let mut callers = BTreeSet::new();
    for (index, code) in methods {
        if direct.contains(index) {
            continue;
        }
        if code.calls.iter().any(|called| direct.contains(called))
            && let Some(caller) = ref_at(refs, *index)
        {
            callers.insert(caller.clone());
        }
    }
    if callers.is_empty() {
        for index in direct {
            if let Some(method) = ref_at(refs, *index) {
                callers.insert(method.clone());
            }
        }
    }
    operations
        .entry(operation.to_string())
        .or_default()
        .extend(callers);
}

fn ref_at(refs: &[Option<MethodRef>], index: usize) -> Option<&MethodRef> {
    refs.get(index).and_then(Option::as_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rejects_archives_without_dex_files() {
        let missing = Path::new("/definitely/missing/noa-test.apk").to_path_buf();
        assert!(SignatureIndex::read_apks(&[missing]).is_err());
    }

    #[test]
    fn encoded_index_is_deterministic_and_delimited() {
        let mut index = SignatureIndex::default();
        index
            .classes_by_source
            .entry("Room.kt".to_string())
            .or_default()
            .extend(["a.Room".to_string(), "b.Room".to_string()]);
        index
            .operations
            .entry("send".to_string())
            .or_default()
            .insert(MethodRef {
                owner: "a.Room".to_string(),
                name: "x".to_string(),
            });
        assert_eq!(
            index.encode().unwrap(),
            "S\tRoom.kt\ta.Room\tb.Room\nO\tsend\ta.Room\tx"
        );
    }
}
