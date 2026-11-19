//! Windows XPKeywords (IFD0 0x9c9e): UTF-16LE even in a big-endian TIFF.
//! Existing EXIF offsets and opaque payloads are retained; only IFD0 is edited.
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

pub const EXIF: &[u8] = b"Exif\0\0";
const TAG: u16 = 0x9c9e;

struct Ifd<'a> {
    bytes: &'a [u8],
    little: bool,
    start: usize,
    count: usize,
    keyword: Option<usize>,
}
impl<'a> Ifd<'a> {
    fn u16(&self, offset: usize) -> Result<u16> {
        let b = self
            .bytes
            .get(offset..offset + 2)
            .context("Truncated EXIF")?
            .try_into()?;
        Ok(if self.little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }
    fn u32(&self, offset: usize) -> Result<u32> {
        let b = self
            .bytes
            .get(offset..offset + 4)
            .context("Truncated EXIF")?
            .try_into()?;
        Ok(if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
    fn parse(body: &'a [u8]) -> Result<Self> {
        ensure!(body.starts_with(EXIF), "Invalid EXIF signature");
        let bytes = &body[EXIF.len()..];
        ensure!(bytes.len() >= 8, "Truncated EXIF header");
        ensure!(
            &bytes[..2] == b"II" || &bytes[..2] == b"MM",
            "Invalid EXIF byte order"
        );
        let mut s = Self {
            bytes,
            little: &bytes[..2] == b"II",
            start: 0,
            count: 0,
            keyword: None,
        };
        ensure!(s.u16(2)? == 42, "Invalid TIFF signature");
        s.start = s.u32(4)? as usize;
        if s.start == 0 {
            return Ok(s);
        }
        ensure!(s.start >= 8, "Invalid EXIF IFD0 offset");
        s.count = s.u16(s.start)? as usize;
        ensure!(
            s.start
                .checked_add(2 + s.count * 12 + 4)
                .is_some_and(|end| end <= bytes.len()),
            "Truncated EXIF IFD0"
        );
        for n in 0..s.count {
            let entry = s.start + 2 + n * 12;
            if s.u16(entry)? == TAG {
                ensure!(
                    s.keyword.is_none(),
                    "Duplicate XPKeywords entries; original preserved"
                );
                s.keyword = Some(entry);
            }
        }
        Ok(s)
    }
    fn value(&self) -> Result<Option<&'a [u8]>> {
        let Some(entry) = self.keyword else {
            return Ok(None);
        };
        ensure!(
            self.u16(entry + 2)? == 1,
            "XPKeywords must have TIFF BYTE type"
        );
        let count = self.u32(entry + 4)? as usize;
        ensure!(
            count.is_multiple_of(2),
            "Invalid XPKeywords UTF-16 byte count"
        );
        let offset = if count <= 4 {
            entry + 8
        } else {
            self.u32(entry + 8)? as usize
        };
        ensure!(count <= 4 || offset >= 8, "Invalid XPKeywords value offset");
        let end = offset
            .checked_add(count)
            .context("XPKeywords offset overflow")?;
        Ok(Some(
            self.bytes
                .get(offset..end)
                .context("Truncated XPKeywords value")?,
        ))
    }
}

pub fn read(body: &[u8]) -> Result<BTreeSet<String>> {
    let ifd = Ifd::parse(body)?;
    let Some(bytes) = ifd.value()? else {
        return Ok(BTreeSet::new());
    };
    let mut units: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    while units.last() == Some(&0) {
        units.pop();
    }
    ensure!(!units.contains(&0), "Embedded NUL in XPKeywords");
    let text = String::from_utf16(&units).context("Invalid XPKeywords UTF-16")?;
    Ok(text
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect())
}
fn put16(out: &mut [u8], at: usize, value: u16, little: bool) {
    out[at..at + 2].copy_from_slice(&if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    });
}
fn put32(out: &mut [u8], at: usize, value: u32, little: bool) {
    out[at..at + 4].copy_from_slice(&if little {
        value.to_le_bytes()
    } else {
        value.to_be_bytes()
    });
}
pub fn write(old: Option<&[u8]>, tags: &BTreeSet<String>) -> Result<Vec<u8>> {
    ensure!(
        tags.iter()
            .all(|s| !s.contains([';', '\0']) && s.trim() == s),
        "JPEG Windows keywords cannot contain semicolons, NUL or leading/trailing spaces"
    );
    let empty = b"Exif\0\0II\x2a\0\0\0\0\0";
    let body = old.unwrap_or(empty);
    // Refuse malformed existing values rather than silently discarding them.
    let existing = read(body)?;
    if old.is_some() && existing == *tags && Ifd::parse(body)?.keyword.is_some() {
        return Ok(body.to_vec());
    }
    let ifd = Ifd::parse(body)?;
    let mut out = ifd.bytes.to_vec();
    let little = ifd.little;
    let entry = if let Some(entry) = ifd.keyword {
        entry
    } else {
        ensure!(ifd.count < u16::MAX as usize, "EXIF IFD0 is full");
        if out.len() % 2 == 1 {
            out.push(0);
        }
        let start = out.len();
        out.resize(start + 2 + (ifd.count + 1) * 12 + 4, 0);
        put16(&mut out, start, (ifd.count + 1) as u16, little);
        // Keep tag order sorted for Windows readers, preserving all original entry bytes.
        let mut entries: Vec<Vec<u8>> = (0..ifd.count)
            .map(|n| ifd.bytes[ifd.start + 2 + n * 12..ifd.start + 14 + n * 12].to_vec())
            .collect();
        let mut new = vec![0; 12];
        put16(&mut new, 0, TAG, little);
        entries.push(new);
        entries.sort_by_key(|e| {
            if little {
                u16::from_le_bytes([e[0], e[1]])
            } else {
                u16::from_be_bytes([e[0], e[1]])
            }
        });
        let mut selected = 0;
        for (n, value) in entries.iter().enumerate() {
            let at = start + 2 + n * 12;
            out[at..at + 12].copy_from_slice(value);
            if value[..2]
                == if little {
                    TAG.to_le_bytes()
                } else {
                    TAG.to_be_bytes()
                }
            {
                selected = at;
            }
        }
        if ifd.start != 0 {
            let next = ifd.start + 2 + ifd.count * 12;
            let dest = start + 2 + (ifd.count + 1) * 12;
            out[dest..dest + 4].copy_from_slice(&ifd.bytes[next..next + 4]);
        }
        put32(&mut out, 4, start as u32, little);
        selected
    };
    let text = tags
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(";");
    let value: Vec<u8> = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    put16(&mut out, entry + 2, 1, little);
    put32(&mut out, entry + 4, value.len() as u32, little);
    if value.len() <= 4 {
        out[entry + 8..entry + 12].fill(0);
        out[entry + 8..entry + 8 + value.len()].copy_from_slice(&value);
    } else {
        // Existing values may share storage with unknown tags. Never overwrite them.
        if out.len() % 2 == 1 {
            out.push(0);
        }
        let at = out.len();
        out.extend_from_slice(&value);
        put32(&mut out, entry + 8, at as u32, little);
    }
    ensure!(
        out.len() + EXIF.len() <= 65533,
        "Windows keywords exceed JPEG EXIF capacity; original preserved"
    );
    let mut result = EXIF.to_vec();
    result.extend(out);
    ensure!(
        read(&result)? == *tags,
        "Windows keyword verification failed"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Built independently of write(): orientation, MakerNote pointer and IFD1 thumbnail pointer.
    fn fixture(little: bool, value: &[u8]) -> Vec<u8> {
        let mut tiff = vec![0; 92];
        tiff[..2].copy_from_slice(if little { b"II" } else { b"MM" });
        put16(&mut tiff, 2, 42, little);
        put32(&mut tiff, 4, 8, little);
        put16(&mut tiff, 8, 3, little);
        for (at, tag, kind, count, data) in [
            (10, 0x112, 3, 1, 6),
            (22, 0x927c, 7, 8, 68),
            (34, TAG, 1, value.len() as u32, 92),
        ] {
            put16(&mut tiff, at, tag, little);
            put16(&mut tiff, at + 2, kind, little);
            put32(&mut tiff, at + 4, count, little);
            if tag == 0x112 {
                put16(&mut tiff, at + 8, 6, little);
            } else {
                put32(&mut tiff, at + 8, data, little);
            }
        }
        put32(&mut tiff, 46, 50, little);
        put16(&mut tiff, 50, 1, little);
        put16(&mut tiff, 52, 0x201, little);
        put16(&mut tiff, 54, 4, little);
        put32(&mut tiff, 56, 1, little);
        put32(&mut tiff, 60, 76, little);
        tiff[68..76].copy_from_slice(b"MAKER123");
        tiff[76..92].copy_from_slice(b"thumbnail-pixels");
        tiff.extend_from_slice(value);
        [EXIF, &tiff].concat()
    }
    #[test]
    fn independent_endian_fixtures_and_edits_preserve_other_exif_bytes() {
        let value: Vec<_> = "järvi; 日本語;😀\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let expected = BTreeSet::from(["järvi".into(), "日本語".into(), "😀".into()]);
        for little in [true, false] {
            let old = fixture(little, &value);
            assert_eq!(read(&old).unwrap(), expected);
            for tags in [
                BTreeSet::from(["new crème brûlée".into(), "🌅".into()]),
                BTreeSet::from(["a".into()]),
                BTreeSet::new(),
            ] {
                let result = write(Some(&old), &tags).unwrap();
                assert_eq!(read(&result).unwrap(), tags);
                // Only the XPKeywords entry and appended value may change.
                assert_eq!(&result[..40], &old[..40]);
                assert_eq!(&result[52..old.len()], &old[52..]);
            }
        }
    }
    #[test]
    fn adding_missing_entry_preserves_original_ifd_links_and_opaque_data() {
        for little in [true, false] {
            let mut old = fixture(little, b"o\0l\0d\0\0\0");
            // Turn the XP entry into an unrelated opaque BYTE tag.
            put16(&mut old[6..], 34, 0x9c9f, little);
            let tags = BTreeSet::from(["ajouté".into()]);
            let out = write(Some(&old), &tags).unwrap();
            let ifd = Ifd::parse(&out).unwrap();
            assert_eq!(ifd.count, 4);
            assert_eq!(read(&out).unwrap(), tags);
            assert_eq!(&out[14..old.len()], &old[14..]);
            assert_eq!(ifd.u32(ifd.start + 2 + ifd.count * 12).unwrap(), 50);
        }
    }
    #[test]
    fn malformed_unicode_offsets_duplicates_and_capacity_are_rejected() {
        for value in [
            vec![0, 0xd8, 0x61, 0, 0x62, 0, 0, 0],
            vec![0x61, 0, 0, 0, 0x62, 0],
        ] {
            let old = fixture(true, &value);
            assert!(read(&old).is_err());
            assert!(write(Some(&old), &BTreeSet::new()).is_err());
        }
        let old = fixture(true, b"a\0b\0\0\0");
        for offset in [0, 0xfffffff0] {
            let mut bad = old.clone();
            put32(&mut bad[6..], 42, offset, true);
            // offset zero points to a malformed UTF-16 signature; huge offset is out of bounds.
            assert!(read(&bad).is_err());
        }
        let mut duplicate = old.clone();
        put16(&mut duplicate[6..], 22, TAG, true);
        assert!(write(Some(&duplicate), &BTreeSet::new()).is_err());
        let huge = BTreeSet::from(["x".repeat(33000)]);
        assert!(write(None, &huge).is_err());
        for text in ["one;two", "NUL\0here", " trailing "] {
            assert!(write(None, &BTreeSet::from([text.into()])).is_err());
        }
    }
    #[test]
    fn repeated_identical_saves_do_not_grow_exif() {
        let tags = BTreeSet::from(["järvi".into()]);
        let mut body = write(None, &tags).unwrap();
        let expected = body.clone();
        for _ in 0..100 {
            body = write(Some(&body), &tags).unwrap();
        }
        assert_eq!(body, expected);
    }
}
