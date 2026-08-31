use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct MethodCode {
    pub(super) fields: BTreeSet<usize>,
    pub(super) calls: BTreeSet<usize>,
}

pub(super) fn read_method_code_items(
    dex: &[u8],
    class_data: usize,
    methods: &mut BTreeMap<usize, MethodCode>,
) {
    let mut cursor = class_data;
    let Some(static_fields) = read_uleb128(dex, &mut cursor) else {
        return;
    };
    let Some(instance_fields) = read_uleb128(dex, &mut cursor) else {
        return;
    };
    let Some(direct_methods) = read_uleb128(dex, &mut cursor) else {
        return;
    };
    let Some(virtual_methods) = read_uleb128(dex, &mut cursor) else {
        return;
    };
    let Some(field_count) = static_fields.checked_add(instance_fields) else {
        return;
    };
    if field_count > dex.len() || direct_methods > dex.len() || virtual_methods > dex.len() {
        return;
    }
    for _ in 0..field_count {
        if read_uleb128(dex, &mut cursor).is_none() || read_uleb128(dex, &mut cursor).is_none() {
            return;
        }
    }
    if !read_encoded_methods(dex, &mut cursor, direct_methods, methods) {
        return;
    }
    read_encoded_methods(dex, &mut cursor, virtual_methods, methods);
}

fn read_encoded_methods(
    dex: &[u8],
    cursor: &mut usize,
    count: usize,
    methods: &mut BTreeMap<usize, MethodCode>,
) -> bool {
    let mut method_index = 0usize;
    for _ in 0..count {
        let Some(diff) = read_uleb128(dex, cursor) else {
            return false;
        };
        let Some(next_index) = method_index.checked_add(diff) else {
            return false;
        };
        method_index = next_index;
        if read_uleb128(dex, cursor).is_none() {
            return false;
        }
        let Some(code_offset) = read_uleb128(dex, cursor) else {
            return false;
        };
        if code_offset > 0 {
            methods.insert(method_index, scan_code(dex, code_offset));
        }
    }
    true
}

fn scan_code(dex: &[u8], code_offset: usize) -> MethodCode {
    let mut result = MethodCode::default();
    let Some(count) = little_usize(dex, code_offset.saturating_add(12)) else {
        return result;
    };
    let Some(start) = code_offset.checked_add(16) else {
        return result;
    };
    let end = start.saturating_add(count.saturating_mul(2)).min(dex.len());
    let mut cursor = start;
    while cursor.saturating_add(3) < end {
        let opcode = dex[cursor];
        if let Some(reference) = unsigned_short(dex, cursor + 2) {
            if opcode == 0x62 {
                result.fields.insert(reference);
            } else if (0x6e..=0x72).contains(&opcode) || (0x74..=0x78).contains(&opcode) {
                result.calls.insert(reference);
            }
        }
        cursor += 2;
    }
    result
}

fn read_uleb128(value: &[u8], cursor: &mut usize) -> Option<usize> {
    let mut result = 0usize;
    let mut shift = 0;
    for _ in 0..5 {
        let next = *value.get(*cursor)?;
        *cursor += 1;
        result |= usize::from(next & 0x7f) << shift;
        if next & 0x80 == 0 {
            return Some(result);
        }
        shift += 7;
    }
    None
}

pub(super) fn dex_string(
    dex: &[u8],
    string_offset: usize,
    index: usize,
    cache: &mut HashMap<usize, String>,
) -> String {
    if let Some(value) = cache.get(&index) {
        return value.clone();
    }
    let Some(item_offset) = index
        .checked_mul(4)
        .and_then(|value| string_offset.checked_add(value))
    else {
        return String::new();
    };
    let Some(data) = little_usize(dex, item_offset).filter(|value| *value < dex.len()) else {
        return String::new();
    };
    let mut cursor = data;
    while let Some(next) = dex.get(cursor) {
        cursor += 1;
        if next & 0x80 == 0 {
            break;
        }
    }
    let end = dex[cursor..]
        .iter()
        .position(|value| *value == 0)
        .map_or(dex.len(), |offset| cursor + offset);
    let value = String::from_utf8_lossy(&dex[cursor..end]).into_owned();
    cache.insert(index, value.clone());
    value
}

pub(super) fn is_dex(value: &[u8]) -> bool {
    value.len() >= 112 && value.starts_with(b"dex\n")
}

pub(super) fn little_usize(value: &[u8], offset: usize) -> Option<usize> {
    let bytes: [u8; 4] = value.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes) as usize)
}

pub(super) fn unsigned_short(value: &[u8], offset: usize) -> Option<usize> {
    let bytes: [u8; 2] = value.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes) as usize)
}

pub(super) fn table_fits(value: &[u8], offset: usize, count: usize, width: usize) -> bool {
    count
        .checked_mul(width)
        .and_then(|size| offset.checked_add(size))
        .is_some_and(|end| end <= value.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_uleb128_and_tables_are_rejected() {
        assert_eq!(read_uleb128(&[0x80; 5], &mut 0), None);
        assert!(!table_fits(&[0; 8], usize::MAX, 1, 4));
        assert!(!is_dex(&[0; 112]));
    }
}
