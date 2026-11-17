//! Minimal, defensive zip reader for the release package: stored and deflate entries, no zip64,
//! no encryption, no multi-disk archives.
//!
//! Nothing is ever extracted by path. The caller names the entries it wants, every entry name
//! of the archive must be a plain file name (no directories, `..`, drive letters, streams),
//! and the data stays in memory until the caller writes it to a location it chose itself.
//! That rules out Zip-Slip by construction.

use std::collections::HashSet;

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    /// Per entry, checked against the declared size and enforced while inflating.
    pub max_entry_bytes: u64,
    /// Sum over the entries actually read.
    pub max_total_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 64,
            max_entry_bytes: 150 * 1024 * 1024,
            max_total_bytes: 200 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    Malformed(&'static str),
    Unsupported(&'static str),
    /// Entry name that could leave the target directory or is not a plain file name.
    UnsafePath(String),
    Duplicate(String),
    Missing(String),
    TooLarge(&'static str),
    Checksum(String),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(m) => write!(f, "Zip beschädigt: {m}"),
            Self::Unsupported(m) => write!(f, "Zip nicht unterstützt: {m}"),
            Self::UnsafePath(n) => write!(f, "Unsicherer Dateiname im Zip: {n:?}"),
            Self::Duplicate(n) => write!(f, "Doppelter Eintrag im Zip: {n:?}"),
            Self::Missing(n) => write!(f, "Zip enthält {n} nicht"),
            Self::TooLarge(m) => write!(f, "Zip zu groß: {m}"),
            Self::Checksum(n) => write!(f, "CRC-Fehler in {n}"),
        }
    }
}
impl std::error::Error for ZipError {}

fn u16_at(data: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(pos..pos.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32_at(data: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(pos..pos.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// A plain file name: no separators, no `.`/`..`, no drive or stream colon, no control
/// characters, no trailing dot/space (Windows strips those, which could alias another name).
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name != "."
        && name != ".."
        && !name.ends_with('.')
        && !name.ends_with(' ')
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

struct CentralEntry {
    name: String,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    local_offset: u64,
}

fn read_central_directory(zip: &[u8], limits: &Limits) -> Result<Vec<CentralEntry>, ZipError> {
    // The EOCD record sits in the last 22 + 65535 bytes (comment up to 64 KiB).
    let tail_start = zip.len().saturating_sub(22 + 65535);
    let eocd = (tail_start..=zip.len().saturating_sub(22))
        .rev()
        .find(|&i| u32_at(zip, i) == Some(EOCD_SIG))
        .ok_or(ZipError::Malformed("Dateiende nicht gefunden"))?;
    let disk = u16_at(zip, eocd + 4).ok_or(ZipError::Malformed("EOCD"))?;
    let cd_disk = u16_at(zip, eocd + 6).ok_or(ZipError::Malformed("EOCD"))?;
    let count = u16_at(zip, eocd + 10).ok_or(ZipError::Malformed("EOCD"))?;
    let cd_size = u32_at(zip, eocd + 12).ok_or(ZipError::Malformed("EOCD"))?;
    let cd_offset = u32_at(zip, eocd + 16).ok_or(ZipError::Malformed("EOCD"))?;
    if disk != 0 || cd_disk != 0 {
        return Err(ZipError::Unsupported("mehrteiliges Archiv"));
    }
    if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
        return Err(ZipError::Unsupported("Zip64"));
    }
    if usize::from(count) > limits.max_entries {
        return Err(ZipError::TooLarge("zu viele Einträge"));
    }
    let start = cd_offset as usize;
    let end = start
        .checked_add(cd_size as usize)
        .filter(|&e| e <= eocd)
        .ok_or(ZipError::Malformed("Verzeichnis außerhalb der Datei"))?;
    let directory = &zip[start..end];
    let mut pos = 0usize;
    let mut entries = Vec::with_capacity(usize::from(count));
    let mut seen = HashSet::new();
    for _ in 0..count {
        if u32_at(directory, pos) != Some(CENTRAL_SIG) {
            return Err(ZipError::Malformed("Verzeichniseintrag"));
        }
        let bad = || ZipError::Malformed("Verzeichniseintrag abgeschnitten");
        let flags = u16_at(directory, pos + 8).ok_or_else(bad)?;
        let method = u16_at(directory, pos + 10).ok_or_else(bad)?;
        let crc = u32_at(directory, pos + 16).ok_or_else(bad)?;
        let compressed = u32_at(directory, pos + 20).ok_or_else(bad)?;
        let size = u32_at(directory, pos + 24).ok_or_else(bad)?;
        let name_len = usize::from(u16_at(directory, pos + 28).ok_or_else(bad)?);
        let extra_len = usize::from(u16_at(directory, pos + 30).ok_or_else(bad)?);
        let comment_len = usize::from(u16_at(directory, pos + 32).ok_or_else(bad)?);
        let local_offset = u32_at(directory, pos + 42).ok_or_else(bad)?;
        let name_bytes = directory
            .get(pos + 46..pos + 46 + name_len)
            .ok_or_else(bad)?;
        pos = pos
            .checked_add(46 + name_len + extra_len + comment_len)
            .filter(|&p| p <= directory.len())
            .ok_or_else(bad)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        if !is_safe_file_name(&name) || std::str::from_utf8(name_bytes).is_err() {
            return Err(ZipError::UnsafePath(name));
        }
        if !seen.insert(name.to_ascii_lowercase()) {
            return Err(ZipError::Duplicate(name));
        }
        if flags & 1 != 0 {
            return Err(ZipError::Unsupported("verschlüsselter Eintrag"));
        }
        if method != 0 && method != 8 {
            return Err(ZipError::Unsupported("Kompressionsverfahren"));
        }
        if u64::from(size) > limits.max_entry_bytes {
            return Err(ZipError::TooLarge("Eintrag"));
        }
        entries.push(CentralEntry {
            name,
            method,
            crc,
            compressed: u64::from(compressed),
            size: u64::from(size),
            local_offset: u64::from(local_offset),
        });
    }
    Ok(entries)
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *slot = c;
        }
        table
    });
    !data.iter().fold(!0u32, |c, &b| {
        table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8)
    })
}

/// Reads the entries named in `wanted` (exact, case-sensitive names). Names that are not
/// wanted are validated but never read. A wanted name missing from the archive is an error
/// unless it is listed in `optional`. Returned in the order of `wanted`.
pub fn read_entries(
    zip: &[u8],
    wanted: &[&str],
    optional: &[&str],
    limits: &Limits,
) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
    let entries = read_central_directory(zip, limits)?;
    let mut out = Vec::new();
    let mut total = 0u64;
    for &name in wanted {
        let Some(entry) = entries.iter().find(|e| e.name == name) else {
            if optional.contains(&name) {
                continue;
            }
            return Err(ZipError::Missing(name.to_owned()));
        };
        total = total.saturating_add(entry.size);
        if total > limits.max_total_bytes {
            return Err(ZipError::TooLarge("Gesamtgröße"));
        }
        out.push((entry.name.clone(), read_entry(zip, entry)?));
    }
    Ok(out)
}

fn read_entry(zip: &[u8], entry: &CentralEntry) -> Result<Vec<u8>, ZipError> {
    let bad = || ZipError::Malformed("lokaler Header");
    let offset = usize::try_from(entry.local_offset).map_err(|_| bad())?;
    if u32_at(zip, offset) != Some(LOCAL_SIG) {
        return Err(bad());
    }
    let name_len = usize::from(u16_at(zip, offset + 26).ok_or_else(bad)?);
    let extra_len = usize::from(u16_at(zip, offset + 28).ok_or_else(bad)?);
    // The local name must agree with the central directory (which was validated).
    if zip.get(offset + 30..offset + 30 + name_len) != Some(entry.name.as_bytes()) {
        return Err(ZipError::Malformed("Name im lokalen Header weicht ab"));
    }
    let start = offset + 30 + name_len + extra_len;
    let end = usize::try_from(entry.compressed)
        .ok()
        .and_then(|c| start.checked_add(c))
        .ok_or_else(bad)?;
    let raw = zip
        .get(start..end)
        .ok_or(ZipError::Malformed("Daten außerhalb der Datei"))?;
    let data = match entry.method {
        0 => {
            if entry.compressed != entry.size {
                return Err(ZipError::Malformed("Größe eines gespeicherten Eintrags"));
            }
            raw.to_vec()
        }
        _ => miniz_oxide::inflate::decompress_to_vec_with_limit(
            raw,
            usize::try_from(entry.size).map_err(|_| ZipError::TooLarge("Eintrag"))?,
        )
        .map_err(|_| ZipError::Malformed("Deflate-Daten ungültig oder größer als angegeben"))?,
    };
    if data.len() as u64 != entry.size {
        return Err(ZipError::Malformed("entpackte Größe weicht ab"));
    }
    if crc32(&data) != entry.crc {
        return Err(ZipError::Checksum(entry.name.clone()));
    }
    Ok(data)
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::crc32;

    pub enum Method {
        Stored,
        Deflate,
    }

    /// Builds a zip in memory; `files` are `(name, data, method)`. Names are written as given,
    /// even when unsafe, so tests can feed hostile archives.
    pub fn build(files: &[(&str, &[u8], Method)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, method) in files {
            let (code, payload) = match method {
                Method::Stored => (0u16, data.to_vec()),
                Method::Deflate => (8u16, miniz_oxide::deflate::compress_to_vec(data, 6)),
            };
            let offset = out.len() as u32;
            let crc = crc32(data);
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&code.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0x21, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&payload);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&code.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0x21, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::{Method, build};
    use super::*;

    fn read(zip: &[u8], wanted: &[&str]) -> Result<Vec<(String, Vec<u8>)>, ZipError> {
        read_entries(zip, wanted, &[], &Limits::default())
    }

    #[test]
    fn reads_stored_and_deflated_entries_in_requested_order() {
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let zip = build(&[
            ("a.txt", b"hello", Method::Stored),
            ("nexpanel.exe", &big, Method::Deflate),
            ("VERSION.txt", b"0.2.0", Method::Deflate),
        ]);
        let got = read(&zip, &["VERSION.txt", "nexpanel.exe"]).unwrap();
        assert_eq!(got[0], ("VERSION.txt".to_string(), b"0.2.0".to_vec()));
        assert_eq!(got[1], ("nexpanel.exe".to_string(), big));
    }

    #[test]
    fn reads_archive_made_by_powershell_compress_archive() {
        let zip = include_bytes!("testdata/compress-archive.zip");
        let got =
            read_entries(zip, &["VERSION.txt", "hello.txt"], &[], &Limits::default()).unwrap();
        assert_eq!(String::from_utf8_lossy(&got[0].1).trim(), "0.2.0");
        assert_eq!(String::from_utf8_lossy(&got[1].1).trim(), "hello zip");
    }

    #[test]
    fn rejects_zip_slip_and_other_unsafe_names() {
        for bad in [
            "../evil.exe",
            "..\\evil.exe",
            "dir/file.txt",
            "dir\\file.txt",
            "/abs.txt",
            "C:\\x.txt",
            "C:x.txt",
            "a.txt:stream",
            "..",
            ".",
            "",
            "trailing.",
            "trailing ",
            "ctl\u{1}.txt",
            "folder/",
            "con*.txt",
        ] {
            let zip = build(&[
                ("ok.txt", b"x", Method::Stored),
                (bad, b"x", Method::Stored),
            ]);
            assert!(
                matches!(read(&zip, &["ok.txt"]), Err(ZipError::UnsafePath(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn unsafe_name_is_rejected_even_when_not_requested() {
        let zip = build(&[
            ("nexpanel.exe", b"x", Method::Stored),
            ("../x", b"x", Method::Stored),
        ]);
        assert!(read(&zip, &["nexpanel.exe"]).is_err());
    }

    #[test]
    fn rejects_duplicates_case_insensitively() {
        let zip = build(&[
            ("A.txt", b"1", Method::Stored),
            ("a.TXT", b"2", Method::Stored),
        ]);
        assert!(matches!(
            read(&zip, &["A.txt"]),
            Err(ZipError::Duplicate(_))
        ));
    }

    #[test]
    fn missing_wanted_entry_is_error_unless_optional() {
        let zip = build(&[("a.txt", b"1", Method::Stored)]);
        assert!(matches!(read(&zip, &["b.txt"]), Err(ZipError::Missing(_))));
        let got = read_entries(&zip, &["a.txt", "b.txt"], &["b.txt"], &Limits::default()).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn detects_corruption() {
        let mut zip = build(&[("a.txt", b"hello world", Method::Stored)]);
        // Flip a data byte (after the 30-byte local header + name).
        zip[30 + 5] ^= 0xFF;
        assert!(matches!(read(&zip, &["a.txt"]), Err(ZipError::Checksum(_))));
        assert!(read(&zip[..zip.len() - 10], &["a.txt"]).is_err());
        assert!(read(b"", &["a.txt"]).is_err());
        assert!(read(b"PK\x05\x06", &["a.txt"]).is_err());
        assert!(read(&[0u8; 100], &["a.txt"]).is_err());
    }

    #[test]
    fn enforces_size_limits_and_inflate_bomb() {
        let data = vec![0u8; 100_000];
        let zip = build(&[("a.bin", &data, Method::Deflate)]);
        let tight = Limits {
            max_entry_bytes: 50_000,
            ..Limits::default()
        };
        assert!(matches!(
            read_entries(&zip, &["a.bin"], &[], &tight),
            Err(ZipError::TooLarge(_))
        ));
        let total = Limits {
            max_total_bytes: 50_000,
            ..Limits::default()
        };
        assert!(matches!(
            read_entries(&zip, &["a.bin"], &[], &total),
            Err(ZipError::TooLarge(_))
        ));
        // Declared size lies (central directory says 10 bytes, data inflates to 100000).
        let mut lying = zip.clone();
        let cd = lying.len() - 22 - (46 + 5);
        lying[cd + 24..cd + 28].copy_from_slice(&10u32.to_le_bytes());
        assert!(read(&lying, &["a.bin"]).is_err());
        let many = Limits {
            max_entries: 0,
            ..Limits::default()
        };
        assert!(matches!(
            read_entries(&zip, &["a.bin"], &[], &many),
            Err(ZipError::TooLarge(_))
        ));
    }

    #[test]
    fn rejects_encrypted_zip64_and_unknown_methods() {
        let zip = build(&[("a.txt", b"hello", Method::Stored)]);
        let cd = zip.len() - 22 - (46 + 5);
        let mut encrypted = zip.clone();
        encrypted[cd + 8] = 1;
        assert!(matches!(
            read(&encrypted, &["a.txt"]),
            Err(ZipError::Unsupported(_))
        ));
        let mut method = zip.clone();
        method[cd + 10] = 12;
        assert!(matches!(
            read(&method, &["a.txt"]),
            Err(ZipError::Unsupported(_))
        ));
        let mut zip64 = zip.clone();
        let eocd = zip.len() - 22;
        zip64[eocd + 10..eocd + 12].copy_from_slice(&0xFFFFu16.to_le_bytes());
        assert!(matches!(
            read(&zip64, &["a.txt"]),
            Err(ZipError::Unsupported(_))
        ));
    }

    #[test]
    fn local_header_name_must_match_central_directory() {
        let mut zip = build(&[("a.txt", b"hello", Method::Stored)]);
        zip[30] = b'b';
        assert!(read(&zip, &["a.txt"]).is_err());
    }

    #[test]
    fn crc32_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
