//! SHA-256 与十六进制辅助。

use crate::error::Result;
use sha2::{Digest, Sha256};
use std::path::Path;

/// 小写十六进制编码。
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// 计算字节的 SHA-256（小写十六进制）。
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    to_hex(&h.finalize())
}

/// 计算文件的 SHA-256（小写十六进制）。
pub fn sha256_hex_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(to_hex(&h.finalize()))
}

/// 是否为 64 位小写十六进制。
pub fn is_valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digest_of_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn validator_rejects_uppercase_and_wrong_length() {
        assert!(is_valid_sha256(&sha256_hex(b"abc")));
        assert!(!is_valid_sha256(&sha256_hex(b"abc").to_uppercase()));
        assert!(!is_valid_sha256("abc"));
    }
}
