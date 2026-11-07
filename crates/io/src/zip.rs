//! A small ZIP container, from the public ZIP file format specification (PKWARE APPNOTE):
//! the container of 3MF files.
//!
//! Writing stores each entry uncompressed (method 0) with its CRC-32. Reading takes stored
//! and deflated (method 8) entries from the central directory. ZIP files are hostile input:
//! every offset is checked, the entry count is capped, and an entry's declared size is
//! capped before anything is inflated, so a small "ZIP bomb" cannot fill memory.

use crate::IoError;

/// Most entries a ZIP may have.
pub const MAX_ENTRIES: usize = 10_000;
/// Largest uncompressed entry accepted (bytes).
pub const MAX_ENTRY_BYTES: u64 = 1024 * 1024 * 1024;

fn bad(why: impl std::fmt::Display) -> IoError {
    IoError::Format(format!("ZIP: {why}"))
}

/// The CRC-32 of ISO 3309 / ITU-T V.42 that ZIP uses (reflected polynomial 0xEDB88320,
/// initial value and final XOR 0xFFFFFFFF).
pub fn crc32(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };
    let mut c = 0xFFFF_FFFFu32;
    for &b in bytes {
        c = TABLE[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// Writes a ZIP holding `entries` (name, contents), each stored uncompressed.
pub fn write_stored(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, IoError> {
    let too_big = || bad("an entry is too large for a ZIP without ZIP64");
    if entries.len() > usize::from(u16::MAX) {
        return Err(bad("too many entries"));
    }
    let mut out = Vec::new();
    let mut central = Vec::new();
    // 1980-01-01 00:00, the earliest MS-DOS date: a fixed time keeps output reproducible.
    let (time, date) = (0u16, (1u16 << 5) | 1);
    for (name, data) in entries {
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len()).map_err(|_| bad("an entry name is too long"))?;
        let crc = crc32(data);
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed: 2.0
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Central directory header.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // made by: 2.0, MS-DOS
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = u32::try_from(out.len()).map_err(|_| too_big())?;
    let cd_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    let count = entries.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    Ok(out)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let s = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One entry of a ZIP's central directory.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    pub size: u64,
    local_offset: u64,
}

/// A ZIP read from bytes: its directory, and the bytes to read entries from.
#[derive(Debug)]
pub struct Archive<'a> {
    bytes: &'a [u8],
    pub entries: Vec<Entry>,
}

impl<'a> Archive<'a> {
    /// Reads the central directory. Nothing is inflated yet.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, IoError> {
        // The end of central directory record is the last 22 bytes, or ends a comment of at
        // most 65535 bytes.
        let last = bytes
            .len()
            .checked_sub(22)
            .ok_or_else(|| bad("too short to be a ZIP file"))?;
        let first = last.saturating_sub(usize::from(u16::MAX));
        let eocd = (first..=last)
            .rev()
            .find(|&i| u32_at(bytes, i) == Some(0x0605_4b50))
            .ok_or_else(|| bad("no end of central directory: not a ZIP file, or cut short"))?;
        let field = |off: usize| u16_at(bytes, eocd + off).ok_or_else(|| bad("cut short"));
        let count = field(10)?;
        let cd_size = u32_at(bytes, eocd + 12).ok_or_else(|| bad("cut short"))?;
        let cd_offset = u32_at(bytes, eocd + 16).ok_or_else(|| bad("cut short"))?;
        if count == u16::MAX || cd_offset == u32::MAX || cd_size == u32::MAX {
            return Err(bad("ZIP64 archives are not supported"));
        }
        if field(4)? != 0 || field(6)? != 0 || field(8)? != count {
            return Err(bad("multi-disk archives are not supported"));
        }
        let count = usize::from(count);
        if count > MAX_ENTRIES {
            return Err(bad(format!("more than {MAX_ENTRIES} entries")));
        }
        let mut at = cd_offset as usize;
        let end = at
            .checked_add(cd_size as usize)
            .filter(|&e| e <= eocd)
            .ok_or_else(|| bad("the central directory lies outside the file"))?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let short = || bad("a central directory entry is cut short");
            if at + 46 > end || u32_at(bytes, at) != Some(0x0201_4b50) {
                return Err(bad("a central directory entry is broken"));
            }
            let flags = u16_at(bytes, at + 8).ok_or_else(short)?;
            let method = u16_at(bytes, at + 10).ok_or_else(short)?;
            let crc = u32_at(bytes, at + 16).ok_or_else(short)?;
            let compressed = u32_at(bytes, at + 20).ok_or_else(short)?;
            let size = u32_at(bytes, at + 24).ok_or_else(short)?;
            let name_len = usize::from(u16_at(bytes, at + 28).ok_or_else(short)?);
            let extra_len = usize::from(u16_at(bytes, at + 30).ok_or_else(short)?);
            let comment_len = usize::from(u16_at(bytes, at + 32).ok_or_else(short)?);
            let local_offset = u32_at(bytes, at + 42).ok_or_else(short)?;
            if compressed == u32::MAX || size == u32::MAX || local_offset == u32::MAX {
                return Err(bad("ZIP64 entries are not supported"));
            }
            let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(short)?;
            entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                method,
                flags,
                crc,
                compressed: u64::from(compressed),
                size: u64::from(size),
                local_offset: u64::from(local_offset),
            });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries })
    }

    /// The entry with this name (ASCII case ignored, a leading `/` ignored), if any.
    pub fn find(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries
            .iter()
            .find(|e| e.name.trim_start_matches('/').eq_ignore_ascii_case(name))
    }

    /// The contents of an entry, at most `limit` bytes (and never above
    /// [`MAX_ENTRY_BYTES`]), checked against its CRC-32.
    pub fn read(&self, entry: &Entry, limit: u64) -> Result<Vec<u8>, IoError> {
        let limit = limit.min(MAX_ENTRY_BYTES);
        let name = &entry.name;
        if entry.flags & 1 != 0 {
            return Err(bad(format!("{name} is encrypted")));
        }
        if entry.size > limit {
            return Err(bad(format!(
                "{name} would be {} bytes, the limit is {limit}",
                entry.size
            )));
        }
        let at = usize::try_from(entry.local_offset).map_err(|_| bad("offset too large"))?;
        if u32_at(self.bytes, at) != Some(0x0403_4b50) {
            return Err(bad(format!("{name}: the local header is missing")));
        }
        let short = || bad(format!("{name} is cut short"));
        let name_len = usize::from(u16_at(self.bytes, at + 26).ok_or_else(short)?);
        let extra_len = usize::from(u16_at(self.bytes, at + 28).ok_or_else(short)?);
        let start = at + 30 + name_len + extra_len;
        let len = usize::try_from(entry.compressed).map_err(|_| short())?;
        let data = self
            .bytes
            .get(start..start.checked_add(len).ok_or_else(short)?)
            .ok_or_else(short)?;
        // The declared size was checked above, so the output is bounded by it.
        let size = entry.size as usize;
        let out = match entry.method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
                .map_err(|e| bad(format!("{name} does not inflate: {:?}", e.status)))?,
            m => {
                return Err(bad(format!(
                    "{name} uses compression method {m}; only stored and deflated entries are read"
                )));
            }
        };
        if out.len() != size {
            return Err(bad(format!("{name} is not the size its header gives")));
        }
        if crc32(&out) != entry.crc {
            return Err(bad(format!("{name} fails its CRC check")));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_hand_computed_values() {
        // "123456789" is the CRC catalogue's check string; CRC-32/ISO-HDLC gives 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // "a" (0x61), worked by hand through the bitwise definition: eight shift-and-XOR
        // steps on 0xFFFFFF9E, then the final inversion, give 0xE8B7BE43.
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_stored_zip_round_trips() {
        let zip = write_stored(&[("a.txt", b"hello"), ("dir/b.bin", &[0, 1, 2, 3])]).unwrap();
        // Local header 30 + name + data; central 46 + name; end record 22.
        assert_eq!(
            zip.len(),
            (30 + 5 + 5) + (30 + 9 + 4) + (46 + 5) + (46 + 9) + 22
        );
        let a = Archive::parse(&zip).unwrap();
        assert_eq!(a.entries.len(), 2);
        let e = a.find("/DIR/B.BIN").unwrap();
        assert_eq!(a.read(e, 100).unwrap(), vec![0, 1, 2, 3]);
        let e = a.find("a.txt").unwrap();
        assert_eq!(a.read(e, 100).unwrap(), b"hello");
        assert!(a.read(e, 4).is_err(), "over the limit");
    }

    #[test]
    fn a_deflated_entry_is_read() {
        // A raw deflate stream with one fixed-Huffman block for "aaaa", built by hand from
        // RFC 1951, bits least significant first: BFINAL 1, BTYPE 01; literal 'a' (code
        // 0x30 + 0x61, 8 bits); length 3 (code 257, 7 bits 0000001); distance 1 (5 bits
        // 00000); end of block (7 bits 0000000). Packed: 4b 04 02 00.
        let deflated = [0x4b, 0x04, 0x02, 0x00];
        let plain = b"aaaa";
        let mut zip = write_stored(&[("x", &deflated)]).unwrap();
        // Patch the method to 8 and the sizes and CRC to the plain text's, in both headers.
        let crc = crc32(plain).to_le_bytes();
        let cd = 30 + 1 + deflated.len();
        for base in [0usize, cd] {
            let (m, c, u) = if base == 0 { (8, 14, 22) } else { (10, 16, 24) };
            zip[base + m] = 8;
            zip[base + c..base + c + 4].copy_from_slice(&crc);
            zip[base + u..base + u + 4].copy_from_slice(&4u32.to_le_bytes());
        }
        let a = Archive::parse(&zip).unwrap();
        assert_eq!(a.read(&a.entries[0], 100).unwrap(), plain);
    }

    #[test]
    fn hostile_zips_are_refused_without_panicking() {
        let good = write_stored(&[("3D/3dmodel.model", b"<model/>")]).unwrap();
        // Every truncation.
        for n in 0..good.len() {
            if let Ok(a) = Archive::parse(&good[..n]) {
                for e in &a.entries {
                    let _ = a.read(e, 1000);
                }
            }
        }
        // A CRC that does not match.
        let mut broken = good.clone();
        broken[30 + 16] ^= 0xff;
        let a = Archive::parse(&broken).unwrap();
        assert!(a.read(&a.entries[0], 1000).is_err());
        // A bomb: the directory claims 4 GB uncompressed; refused before inflating.
        let mut bomb = good.clone();
        let cd = 30 + 16 + 8;
        bomb[cd + 24..cd + 28].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
        bomb[cd + 10] = 8;
        let a = Archive::parse(&bomb).unwrap();
        assert!(a.read(&a.entries[0], u64::MAX).is_err());
        // A huge entry count and offsets past the end.
        let mut lying = good.clone();
        let end = lying.len() - 22;
        lying[end + 8..end + 12].copy_from_slice(&[0xfe, 0xff, 0xfe, 0xff]);
        assert!(Archive::parse(&lying).is_err());
        let mut far = good.clone();
        far[end + 16..end + 20].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        assert!(Archive::parse(&far).is_err());
        assert!(Archive::parse(&[0xff; 100]).is_err());
    }
}
