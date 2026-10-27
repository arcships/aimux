//! Tool call id normalization for Mistral models on Bedrock (normalize-tool-call-id.ts).

#[must_use]
pub fn normalize_tool_call_id(id: &str, is_mistral: bool) -> String {
    if !is_mistral || (id.len() == 9 && id.bytes().all(|c| c.is_ascii_alphanumeric())) {
        return id.to_owned();
    }
    let mut hash = 14_695_981_039_346_656_037_u64;
    for unit in id.encode_utf16() {
        hash = (hash ^ u64::from(unit)).wrapping_mul(1_099_511_628_211);
    }
    let mut value = hash % 62_u64.pow(9);
    let mut bytes = [b'0'; 9];
    let alphabet = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    for byte in bytes.iter_mut().rev() {
        *byte = alphabet[(value % 62) as usize];
        value /= 62;
    }
    String::from_utf8(bytes.to_vec()).unwrap()
}
