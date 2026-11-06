//! Base64 (RFC 4648, standard alphabet, padded) for image content. Encoding only; a few
//! lines are simpler than a dependency.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextets = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        // 1 input byte gives 2 output characters, 2 give 3, 3 give 4; the rest is padding.
        for (i, s) in sextets.iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[*s as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::encode;

    /// The test vectors of RFC 4648, section 10.
    #[test]
    fn encodes_the_rfc_4648_test_vectors() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()), coded, "{plain:?}");
        }
    }

    /// 0xFB 0xFF uses the last two alphabet characters: 111110 111111 1111(00) -> "+/8=".
    #[test]
    fn uses_plus_and_slash_for_62_and_63() {
        assert_eq!(encode(&[0xFB, 0xFF]), "+/8=");
    }
}
