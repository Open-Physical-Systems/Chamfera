//! Version history: snapshots of a design kept in a folder beside its file.
//!
//! `part.chamfera` keeps its versions in `part.chamfera.history/`. Version 12 is two
//! files (three with a thumbnail):
//! - `v000012.chamfera.z`: the design as saved, zlib-compressed JSON;
//! - `v000012.json`: what the list shows: time, note, kind and a summary of the design
//!   (volume, feature count). It is written last, so a version is listed only once its
//!   snapshot is complete;
//! - `v000012.png`: a small picture of the model, when the app could draw one.
//!
//! Every file is written atomically ([`crate::atomic`]). The folder is hostile input like
//! any file: entries that are links, folders, too big or broken are skipped (and listed as
//! skipped), never followed, and a history folder that is itself a link is refused, so a
//! history can never write outside the folder its design is in.

use crate::IoError;
use crate::atomic::write_atomic;
use chamfera_doc::Document;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The history folder's suffix after the design's file name.
pub const FOLDER_SUFFIX: &str = ".history";
/// The most folder entries looked at (hostile-input limit).
pub const MAX_ENTRIES: usize = 20_000;
/// Largest metadata file read (bytes).
pub const MAX_META_BYTES: u64 = 64 * 1024;
/// Largest compressed snapshot read (bytes).
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
/// Largest thumbnail read (bytes).
pub const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;
/// Longest note (characters).
pub const MAX_NOTE_CHARS: usize = 500;
/// The highest version id (ids are written with six digits at least; more are fine).
pub const MAX_ID: u32 = 999_999_999;

/// How many versions, and how many bytes of snapshots, a history keeps. The newest
/// version is always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_count: usize,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_count: 50,
            max_bytes: 500 * 1024 * 1024,
        }
    }
}

/// Why a version was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An explicit save.
    Save,
    /// "Save version with note".
    Note,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Save => "save",
            Kind::Note => "note",
        }
    }

    fn parse(s: &str) -> Option<Kind> {
        match s {
            "save" => Some(Kind::Save),
            "note" => Some(Kind::Note),
            _ => None,
        }
    }
}

/// What the list shows of a design without opening the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Summary {
    /// Total volume of the solids (mm^3), if it could be worked out.
    pub volume: Option<f64>,
    pub features: u32,
    pub sketches: u32,
    pub bodies: u32,
    pub parameters: u32,
}

impl Summary {
    pub fn to_json(self) -> Value {
        json!({
            "volume": self.volume,
            "features": self.features,
            "sketches": self.sketches,
            "bodies": self.bodies,
            "parameters": self.parameters,
        })
    }

    fn from_json(v: &Value) -> Summary {
        let count = |k: &str| {
            v.get(k)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0)
        };
        Summary {
            volume: v
                .get("volume")
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite()),
            features: count("features"),
            sketches: count("sketches"),
            bodies: count("bodies"),
            parameters: count("parameters"),
        }
    }
}

/// One kept version.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub id: u32,
    /// Seconds since 1970-01-01 UTC.
    pub time: u64,
    pub note: String,
    pub kind: Kind,
    /// The compressed snapshot's size.
    pub bytes: u64,
    /// A hash of the design's JSON, to skip saving the same design twice in a row.
    pub hash: u64,
    pub summary: Summary,
    /// A thumbnail file is there.
    pub thumbnail: bool,
}

impl Version {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "time": self.time,
            "when": format_utc(self.time),
            "note": self.note,
            "kind": self.kind.key(),
            "bytes": self.bytes,
            "summary": self.summary.to_json(),
            "thumbnail": self.thumbnail,
        })
    }

    fn meta_json(&self) -> String {
        let v = json!({
            "format": "chamfera-version",
            "version": 1,
            "id": self.id,
            "time": self.time,
            "note": self.note,
            "kind": self.kind.key(),
            "bytes": self.bytes,
            "hash": format!("{:016x}", self.hash),
            "summary": self.summary.to_json(),
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    fn from_meta(text: &str, id: u32) -> Result<Version, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        if v.get("format").and_then(Value::as_str) != Some("chamfera-version") {
            return Err("not a version file".into());
        }
        if v.get("id").and_then(Value::as_u64) != Some(u64::from(id)) {
            return Err("its id does not match its file name".into());
        }
        let time = v.get("time").and_then(Value::as_u64).ok_or("no time")?;
        let kind = v
            .get("kind")
            .and_then(Value::as_str)
            .and_then(Kind::parse)
            .ok_or("unknown kind")?;
        let note = clean_note(v.get("note").and_then(Value::as_str).unwrap_or(""));
        let hash = v
            .get("hash")
            .and_then(Value::as_str)
            .and_then(|h| u64::from_str_radix(h, 16).ok())
            .unwrap_or(0);
        Ok(Version {
            id,
            time,
            note,
            kind,
            bytes: v.get("bytes").and_then(Value::as_u64).unwrap_or(0),
            hash,
            summary: v.get("summary").map(Summary::from_json).unwrap_or_default(),
            thumbnail: false,
        })
    }
}

/// A note as kept: at most [`MAX_NOTE_CHARS`] characters, without control characters
/// other than line breaks, trimmed.
pub fn clean_note(note: &str) -> String {
    note.chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .take(MAX_NOTE_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// What a history folder holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listing {
    /// Oldest first.
    pub versions: Vec<Version>,
    /// Entries left out, with the reason.
    pub skipped: Vec<String>,
}

/// The history folder of a design file: `<file name>.history` beside it.
pub fn folder_for(design: &Path) -> Result<PathBuf, IoError> {
    let name = design
        .file_name()
        .ok_or_else(|| IoError::Format(format!("{} does not name a file", design.display())))?;
    let mut folder = name.to_os_string();
    folder.push(FOLDER_SUFFIX);
    Ok(design.with_file_name(folder))
}

/// Checks the folder: it must be a real folder (not a link), or missing. Returns whether
/// it exists.
fn check_folder(dir: &Path) -> Result<bool, IoError> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
        Ok(m) if m.file_type().is_symlink() => Err(IoError::Format(format!(
            "{} is a link: a history is only kept in a real folder",
            dir.display()
        ))),
        Ok(m) if !m.is_dir() => Err(IoError::Format(format!(
            "{} is not a folder",
            dir.display()
        ))),
        Ok(_) => Ok(true),
    }
}

fn meta_name(id: u32) -> String {
    format!("v{id:06}.json")
}

fn snapshot_name(id: u32) -> String {
    format!("v{id:06}.chamfera.z")
}

fn thumbnail_name(id: u32) -> String {
    format!("v{id:06}.png")
}

/// The id in a history file name (`v000012.json` and its siblings), if it is one.
fn id_of(name: &str) -> Option<(u32, &str)> {
    let rest = name.strip_prefix('v')?;
    let (digits, ext) = rest.split_once('.')?;
    if digits.is_empty() || digits.len() > 9 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: u32 = digits.parse().ok()?;
    (1..=MAX_ID).contains(&id).then_some((id, ext))
}

/// A plain file (not a link, folder or device) in the history folder.
fn regular_file(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(path).ok().filter(|m| m.is_file())
}

/// The history folder's entries that look like history files: (id, extension, path).
fn entries(dir: &Path) -> Result<Vec<(u32, String, PathBuf)>, IoError> {
    if !check_folder(dir)? {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)?.take(MAX_ENTRIES).flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some((id, ext)) = id_of(name) {
            out.push((id, ext.to_owned(), entry.path()));
        }
    }
    Ok(out)
}

/// Lists the versions in a history folder, oldest first. A missing folder has none.
pub fn list(dir: &Path) -> Result<Listing, IoError> {
    let mut listing = Listing::default();
    let all = entries(dir)?;
    for (id, ext, path) in &all {
        if ext != "json" {
            continue;
        }
        let name = meta_name(*id);
        let Some(meta) = regular_file(path) else {
            listing
                .skipped
                .push(format!("{name}: not a plain file (a link or a folder)"));
            continue;
        };
        if meta.len() > MAX_META_BYTES {
            listing
                .skipped
                .push(format!("{name}: {} bytes, too big", meta.len()));
            continue;
        }
        let text = match crate::read_limited(path, MAX_META_BYTES) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                listing.skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        let mut version = match Version::from_meta(&text, *id) {
            Ok(v) => v,
            Err(e) => {
                listing.skipped.push(format!("{name}: {e}"));
                continue;
            }
        };
        match regular_file(&dir.join(snapshot_name(*id))) {
            Some(m) => version.bytes = m.len(),
            None => {
                listing
                    .skipped
                    .push(format!("{name}: its snapshot is missing"));
                continue;
            }
        }
        version.thumbnail = regular_file(&dir.join(thumbnail_name(*id))).is_some();
        listing.versions.push(version);
    }
    listing.versions.sort_by_key(|v| v.id);
    listing.skipped.sort();
    Ok(listing)
}

/// FNV-1a, 64 bit: a quick hash of a design's text, to notice an unchanged design.
pub fn hash(text: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Seconds since 1970 now (0 if the clock is before it).
pub fn now() -> u64 {
    crate::atomic::now_ms() / 1000
}

/// The file holding the last version id handed out.
const LAST_ID_FILE: &str = "last-id";

/// The last version id handed out (0 when unknown).
fn last_id(dir: &Path) -> u32 {
    let path = dir.join(LAST_ID_FILE);
    if regular_file(&path).is_none() {
        return 0;
    }
    crate::read_limited(&path, 32)
        .ok()
        .and_then(|b| String::from_utf8_lossy(&b).trim().parse::<u32>().ok())
        .filter(|id| *id <= MAX_ID)
        .unwrap_or(0)
}

/// What [`add`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    pub version: Version,
    /// The design was the same as the newest version, which was kept instead.
    pub unchanged: bool,
    /// Versions removed to stay within the limits.
    pub pruned: Vec<u32>,
}

/// Keeps a version of `doc` in the history folder `dir` (made if missing), then removes
/// the oldest versions over the limits. A save of the same design as the newest version
/// adds nothing; a version with a note always is kept.
pub fn add(
    dir: &Path,
    doc: &Document,
    kind: Kind,
    note: &str,
    summary: Summary,
    limits: Limits,
) -> Result<Added, IoError> {
    let text = doc.to_json().map_err(|e| IoError::Format(e.to_string()))?;
    let h = hash(&text);
    if !check_folder(dir)? {
        std::fs::create_dir_all(dir)?;
        check_folder(dir)?;
    }
    let listing = list(dir)?;
    if kind == Kind::Save
        && let Some(last) = listing.versions.last()
        && last.hash == h
    {
        return Ok(Added {
            version: last.clone(),
            unchanged: true,
            pruned: Vec::new(),
        });
    }
    // Ids are never reused: past every file that looks like a version, kept or not, and
    // past the last id handed out (versions deleted since).
    let last = entries(dir)?
        .iter()
        .map(|(id, _, _)| *id)
        .max()
        .unwrap_or(0)
        .max(last_id(dir));
    let id = last
        .checked_add(1)
        .filter(|id| *id <= MAX_ID)
        .ok_or_else(|| IoError::Format("the history has run out of version ids".into()))?;
    let packed = miniz_oxide::deflate::compress_to_vec_zlib(text.as_bytes(), 6);
    write_atomic(&dir.join(snapshot_name(id)), &packed)?;
    let version = Version {
        id,
        time: now(),
        note: clean_note(note),
        kind,
        bytes: packed.len() as u64,
        hash: h,
        summary,
        thumbnail: false,
    };
    write_atomic(&dir.join(meta_name(id)), version.meta_json().as_bytes())?;
    write_atomic(&dir.join(LAST_ID_FILE), id.to_string().as_bytes())?;
    let pruned = prune(dir, limits)?;
    Ok(Added {
        version,
        unchanged: false,
        pruned,
    })
}

/// Removes the oldest versions until at most `max_count` are left and their snapshots
/// take at most `max_bytes` (the newest is always kept). Returns the removed ids.
pub fn prune(dir: &Path, limits: Limits) -> Result<Vec<u32>, IoError> {
    let listing = list(dir)?;
    let mut kept: Vec<&Version> = listing.versions.iter().collect();
    let mut total: u64 = kept.iter().map(|v| v.bytes).sum();
    let mut removed = Vec::new();
    while kept.len() > 1 && (kept.len() > limits.max_count.max(1) || total > limits.max_bytes) {
        let oldest = kept.remove(0);
        total = total.saturating_sub(oldest.bytes);
        delete(dir, oldest.id)?;
        removed.push(oldest.id);
    }
    Ok(removed)
}

/// Deletes a version: its metadata first, so it is gone from the list at once, then its
/// snapshot and thumbnail.
pub fn delete(dir: &Path, id: u32) -> Result<(), IoError> {
    check_folder(dir)?;
    let meta = dir.join(meta_name(id));
    if regular_file(&meta).is_none() {
        return Err(IoError::Format(format!("there is no version {id}")));
    }
    std::fs::remove_file(&meta)?;
    for name in [snapshot_name(id), thumbnail_name(id)] {
        let p = dir.join(name);
        if regular_file(&p).is_some() {
            std::fs::remove_file(&p)?;
        }
    }
    Ok(())
}

/// Reads a version's design back, checked like any design file.
pub fn read(dir: &Path, id: u32) -> Result<Document, IoError> {
    if !check_folder(dir)? || regular_file(&dir.join(meta_name(id))).is_none() {
        return Err(IoError::Format(format!("there is no version {id}")));
    }
    let path = dir.join(snapshot_name(id));
    if regular_file(&path).is_none() {
        return Err(IoError::Format(format!(
            "version {id}'s snapshot is missing or not a plain file"
        )));
    }
    let packed = crate::read_limited(&path, MAX_SNAPSHOT_BYTES)?;
    let limit = usize::try_from(crate::MAX_DESIGN_BYTES).unwrap_or(usize::MAX);
    let bytes =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&packed, limit).map_err(|e| {
            IoError::Format(format!(
                "version {id}'s snapshot is damaged (could not uncompress it: {:?})",
                e.status
            ))
        })?;
    let text = String::from_utf8(bytes)
        .map_err(|_| IoError::Format(format!("version {id}'s snapshot is not UTF-8 text")))?;
    Ok(Document::from_json(&text)?)
}

/// Stores a version's thumbnail (PNG bytes).
pub fn write_thumbnail(dir: &Path, id: u32, png: &[u8]) -> Result<(), IoError> {
    if !check_folder(dir)? || regular_file(&dir.join(meta_name(id))).is_none() {
        return Err(IoError::Format(format!("there is no version {id}")));
    }
    if png.len() as u64 > MAX_THUMBNAIL_BYTES {
        return Err(IoError::Format("the thumbnail is too big".into()));
    }
    write_atomic(&dir.join(thumbnail_name(id)), png)
}

/// Reads a version's thumbnail (PNG bytes).
pub fn read_thumbnail(dir: &Path, id: u32) -> Result<Vec<u8>, IoError> {
    check_folder(dir)?;
    let path = dir.join(thumbnail_name(id));
    if regular_file(&path).is_none() {
        return Err(IoError::Format(format!("version {id} has no thumbnail")));
    }
    crate::read_limited(&path, MAX_THUMBNAIL_BYTES)
}

/// Days since 1970-01-01 to a civil date (proleptic Gregorian), after H. Hinnant's
/// public-domain `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A time as `YYYY-MM-DD hh:mm UTC`.
pub fn format_utc(secs: u64) -> String {
    let secs = i64::try_from(secs).unwrap_or(i64::MAX / 2);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        s / 3600,
        (s % 3600) / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("chamfera-history-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn doc_with(w: f64) -> Document {
        let mut d = Document::default();
        d.set_parameter("w", w, chamfera_doc::Unit::Length).unwrap();
        d
    }

    #[test]
    fn the_history_folder_sits_beside_the_design() {
        let f = folder_for(Path::new("/a/b/part.chamfera")).unwrap();
        assert_eq!(f, PathBuf::from("/a/b/part.chamfera.history"));
    }

    #[test]
    fn versions_are_capped_by_count_keeping_the_newest() {
        let dir = temp_dir("count").join("p.chamfera.history");
        let limits = Limits {
            max_count: 3,
            max_bytes: u64::MAX,
        };
        for i in 1..=5 {
            add(
                &dir,
                &doc_with(f64::from(i)),
                Kind::Save,
                "",
                Summary::default(),
                limits,
            )
            .unwrap();
        }
        let ids: Vec<u32> = list(&dir).unwrap().versions.iter().map(|v| v.id).collect();
        // Five saves, three kept: the last three, 3, 4 and 5.
        assert_eq!(ids, vec![3, 4, 5]);
        // Version 3 holds the third design, w = 3.
        let d = read(&dir, 3).unwrap();
        assert_eq!(d.parameters["w"].value, 3.0);
    }

    #[test]
    fn versions_are_capped_by_size_but_the_newest_stays() {
        let dir = temp_dir("size").join("p.chamfera.history");
        let limits = Limits {
            max_count: 100,
            max_bytes: 1,
        };
        for i in 1..=3 {
            add(
                &dir,
                &doc_with(f64::from(i)),
                Kind::Save,
                "",
                Summary::default(),
                limits,
            )
            .unwrap();
        }
        let ids: Vec<u32> = list(&dir).unwrap().versions.iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![3]);
    }

    #[test]
    fn saving_the_same_design_twice_keeps_one_version_unless_it_has_a_note() {
        let dir = temp_dir("same").join("p.chamfera.history");
        let l = Limits::default();
        let d = doc_with(1.0);
        let a = add(&dir, &d, Kind::Save, "", Summary::default(), l).unwrap();
        let b = add(&dir, &d, Kind::Save, "", Summary::default(), l).unwrap();
        assert!(!a.unchanged && b.unchanged);
        assert_eq!(b.version.id, a.version.id);
        let c = add(
            &dir,
            &d,
            Kind::Note,
            "  first\u{7} draft ",
            Summary::default(),
            l,
        )
        .unwrap();
        assert_eq!(c.version.id, 2);
        assert_eq!(c.version.note, "first draft");
        // Deleted ids are not handed out again.
        delete(&dir, 2).unwrap();
        let e = add(&dir, &d, Kind::Note, "x", Summary::default(), l).unwrap();
        assert_eq!(e.version.id, 3);
    }

    #[test]
    fn hostile_history_folders_never_panic() {
        let base = temp_dir("hostile");
        let dir = base.join("p.chamfera.history");
        std::fs::create_dir_all(&dir).unwrap();
        add(
            &dir,
            &doc_with(1.0),
            Kind::Save,
            "",
            Summary::default(),
            Limits::default(),
        )
        .unwrap();
        // A corrupt snapshot with a valid metadata file.
        std::fs::write(dir.join("v000002.chamfera.z"), b"\x78\x9cnot zlib at all").unwrap();
        std::fs::write(
            dir.join("v000002.json"),
            r#"{"format":"chamfera-version","id":2,"time":1,"kind":"save"}"#,
        )
        .unwrap();
        assert!(read(&dir, 2).is_err());
        // Valid zlib that holds no design.
        let junk = miniz_oxide::deflate::compress_to_vec_zlib(b"{\"format\":1}", 6);
        std::fs::write(dir.join("v000003.chamfera.z"), junk).unwrap();
        std::fs::write(
            dir.join("v000003.json"),
            r#"{"format":"chamfera-version","id":3,"time":1,"kind":"save"}"#,
        )
        .unwrap();
        assert!(read(&dir, 3).is_err());
        // A huge (sparse) snapshot and a huge metadata file.
        let huge = std::fs::File::create(dir.join("v000004.chamfera.z")).unwrap();
        huge.set_len(MAX_SNAPSHOT_BYTES + 1).unwrap();
        std::fs::write(
            dir.join("v000004.json"),
            r#"{"format":"chamfera-version","id":4,"time":1,"kind":"save"}"#,
        )
        .unwrap();
        assert!(read(&dir, 4).is_err());
        let big = std::fs::File::create(dir.join("v000005.json")).unwrap();
        big.set_len(MAX_META_BYTES + 1).unwrap();
        // Broken metadata: not JSON, the wrong id, negative and huge numbers.
        std::fs::write(dir.join("v000006.json"), b"\xff\xfe{{{").unwrap();
        std::fs::write(
            dir.join("v000007.json"),
            r#"{"format":"chamfera-version","id":8,"time":1,"kind":"save"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("v000008.chamfera.z"), b"").unwrap();
        std::fs::write(
            dir.join("v000008.json"),
            r#"{"format":"chamfera-version","id":8,"time":-1,"kind":"save",
                "summary":{"volume":"x","features":1e300}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("v99999999999999.json"), b"{}").unwrap();
        // A folder and links, one of them a loop, named like versions.
        std::fs::create_dir_all(dir.join("v000009.json")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("v000010.json", dir.join("v000010.json")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", dir.join("v000011.json")).unwrap();
            std::os::unix::fs::symlink("/dev/zero", dir.join("v000012.chamfera.z")).unwrap();
            std::fs::write(
                dir.join("v000012.json"),
                r#"{"format":"chamfera-version","id":12,"time":1,"kind":"save"}"#,
            )
            .unwrap();
            assert!(read(&dir, 12).is_err());
        }
        let listing = list(&dir).unwrap();
        // Version 1 and the three with readable metadata and a plain snapshot file are
        // listed; reading the broken ones fails without a panic.
        let ids: Vec<u32> = listing.versions.iter().map(|v| v.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        assert!(!listing.skipped.is_empty());
        assert_eq!(read(&dir, 1).unwrap().parameters["w"].value, 1.0);
        assert!(read(&dir, 99).is_err());
        // Adding goes on past every id seen; pruning copes with the broken entries.
        let a = add(
            &dir,
            &doc_with(2.0),
            Kind::Save,
            "",
            Summary::default(),
            Limits {
                max_count: 2,
                max_bytes: u64::MAX,
            },
        )
        .unwrap();
        assert_eq!(a.version.id, 13);
        // A history folder that is a link, here to itself (a loop), is refused.
        #[cfg(unix)]
        {
            let looped = base.join("q.chamfera.history");
            std::os::unix::fs::symlink(&looped, &looped).unwrap();
            assert!(list(&looped).is_err());
            assert!(
                add(
                    &looped,
                    &doc_with(1.0),
                    Kind::Save,
                    "",
                    Summary::default(),
                    Limits::default()
                )
                .is_err()
            );
            assert!(read(&looped, 1).is_err());
        }
        // A history "folder" that is a file.
        let file = base.join("r.chamfera.history");
        std::fs::write(&file, b"x").unwrap();
        assert!(list(&file).is_err());
    }

    #[test]
    fn times_are_shown_as_utc_dates() {
        // 2026-11-07 00:00 UTC is day 20 764 since 1970: 56 years with 14 leap days
        // (1972..2024) = 20 454 days, then 310 days into 2026 (Jan 1 to Nov 7).
        assert_eq!(
            format_utc(20_764 * 86_400 + 3_600 * 14 + 60 * 3),
            "2026-11-07 14:03 UTC"
        );
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
    }
}
