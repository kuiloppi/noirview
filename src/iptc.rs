//! JPEG Photoshop image resources and IPTC-IIM keywords. Unrelated data stays opaque.
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

pub const PHOTOSHOP: &[u8] = b"Photoshop 3.0\0";
const IPTC: u16 = 0x0404;
const DIGEST: u16 = 0x0425;
const UTF8: &[u8] = b"\x1b%G";

struct Resource<'a> {
    id: u16,
    prefix: &'a [u8],
    data: &'a [u8],
    raw: &'a [u8],
}

fn resources(body: &[u8]) -> Result<Vec<Resource<'_>>> {
    ensure!(body.starts_with(PHOTOSHOP), "Unsupported Photoshop header");
    let mut pos = PHOTOSHOP.len();
    let mut result = Vec::new();
    while pos < body.len() {
        let start = pos;
        ensure!(
            body.get(pos..pos + 4) == Some(b"8BIM"),
            "Invalid Photoshop resource signature; original preserved"
        );
        let id = u16::from_be_bytes(
            body.get(pos + 4..pos + 6)
                .context("Truncated Photoshop resource ID")?
                .try_into()?,
        );
        pos += 6;
        let name_len = 1 + *body.get(pos).context("Truncated Photoshop resource name")? as usize;
        pos += name_len + name_len % 2;
        let prefix = body
            .get(start..pos)
            .context("Truncated Photoshop resource name")?;
        let length = u32::from_be_bytes(
            body.get(pos..pos + 4)
                .context("Truncated Photoshop resource length")?
                .try_into()?,
        ) as usize;
        pos += 4;
        let end = pos
            .checked_add(length)
            .context("Photoshop resource overflow")?;
        let data = body
            .get(pos..end)
            .context("Truncated Photoshop resource data")?;
        pos = end
            .checked_add(length % 2)
            .context("Photoshop padding overflow")?;
        let raw = body
            .get(start..pos)
            .context("Truncated Photoshop resource padding")?;
        result.push(Resource {
            id,
            prefix,
            data,
            raw,
        });
    }
    Ok(result)
}

struct Dataset<'a> {
    record: u8,
    number: u8,
    data: &'a [u8],
    raw: &'a [u8],
}

fn datasets(bytes: &[u8]) -> Result<Vec<Dataset<'_>>> {
    let mut pos = 0;
    let mut result = Vec::new();
    while pos < bytes.len() {
        let start = pos;
        let header = bytes.get(pos..pos + 5).context("Truncated IPTC dataset")?;
        ensure!(
            header[0] == 0x1c,
            "Invalid IPTC dataset marker; original preserved"
        );
        let record = header[1];
        let number = header[2];
        let encoded = u16::from_be_bytes([header[3], header[4]]);
        pos += 5;
        let length = if encoded & 0x8000 == 0 {
            encoded as usize
        } else {
            let count = (encoded & 0x7fff) as usize;
            ensure!((1..=4).contains(&count), "Unsupported IPTC extended length");
            let value = bytes
                .get(pos..pos + count)
                .context("Truncated IPTC extended length")?;
            pos += count;
            value
                .iter()
                .fold(0usize, |length, byte| (length << 8) | *byte as usize)
        };
        let end = pos.checked_add(length).context("IPTC dataset overflow")?;
        let data = bytes
            .get(pos..end)
            .context("Truncated IPTC dataset value")?;
        pos = end;
        result.push(Dataset {
            record,
            number,
            data,
            raw: &bytes[start..end],
        });
    }
    Ok(result)
}

fn utf8_declared(items: &[Dataset<'_>]) -> Result<bool> {
    let mut charset = None;
    for item in items.iter().filter(|d| (d.record, d.number) == (1, 90)) {
        ensure!(
            charset.is_none(),
            "Duplicate IPTC character-set declarations; original preserved"
        );
        ensure!(
            item.data == UTF8,
            "Unsupported IPTC character encoding; original preserved"
        );
        charset = Some(true);
    }
    Ok(charset.unwrap_or(false))
}

fn keywords(items: &[Dataset<'_>]) -> Result<BTreeSet<String>> {
    let utf8 = utf8_declared(items)?;
    let mut tags = BTreeSet::new();
    for item in items.iter().filter(|d| (d.record, d.number) == (2, 25)) {
        ensure!(
            utf8 || item.data.is_ascii(),
            "IPTC keywords have ambiguous character encoding; original preserved"
        );
        let word = std::str::from_utf8(item.data)
            .context("Invalid UTF-8 IPTC keyword; original preserved")?;
        ensure!(
            !word.contains('\0'),
            "NUL in IPTC keyword; original preserved"
        );
        ensure!(
            word.chars()
                .all(|c| c >= ' ' || matches!(c, '\t' | '\n' | '\r')),
            "Control or legacy escape sequence in IPTC keyword; original preserved"
        );
        if !word.trim().is_empty() {
            tags.insert(word.trim().into());
        }
    }
    Ok(tags)
}

pub fn read(body: &[u8]) -> Result<Option<BTreeSet<String>>> {
    let resources = resources(body)?;
    let mut tags = None;
    for resource in resources.iter().filter(|r| r.id == IPTC) {
        ensure!(
            tags.is_none(),
            "Duplicate Photoshop IPTC resources; original preserved"
        );
        tags = Some(keywords(&datasets(resource.data)?)?);
    }
    Ok(tags)
}

fn dataset(out: &mut Vec<u8>, record: u8, number: u8, data: &[u8]) -> Result<()> {
    ensure!(data.len() <= 0x7fff, "IPTC dataset exceeds capacity");
    out.extend_from_slice(&[0x1c, record, number]);
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
    Ok(())
}

fn edit_iim(old: &[u8], tags: &BTreeSet<String>) -> Result<Vec<u8>> {
    let items = datasets(old)?;
    // Even replacement must not silently discard words whose encoding is unknown.
    keywords(&items)?;
    let utf8 = utf8_declared(&items)?;
    ensure!(
        tags.iter().all(|s| s.len() <= 64),
        "IPTC keywords must fit 64 UTF-8 bytes each; original preserved"
    );
    let needs_utf8 = tags.iter().any(|s| !s.is_ascii());
    if !utf8 && needs_utf8 {
        // A declaration applies globally. Never reinterpret opaque legacy text.
        ensure!(
            items.iter().all(|d| d.data.is_ascii() && !d.data.iter().any(|b| matches!(b, 0x1b | 0x0e | 0x0f))),
            "Cannot add UTF-8 IPTC keywords beside unmarked non-ASCII or escaped metadata; original preserved"
        );
    }
    let mut out = Vec::new();
    if !utf8 && needs_utf8 {
        dataset(&mut out, 1, 90, UTF8)?;
    }
    if old.is_empty() {
        dataset(&mut out, 2, 0, &[0, 4])?;
    }
    // Retain every other dataset byte-for-byte, including extended lengths.
    let mut inserted = false;
    for item in &items {
        if !inserted && item.record >= 2 && (item.record > 2 || item.number >= 25) {
            for tag in tags {
                dataset(&mut out, 2, 25, tag.as_bytes())?;
            }
            inserted = true;
        }
        if (item.record, item.number) != (2, 25) {
            out.extend_from_slice(item.raw);
        }
    }
    if !inserted {
        for tag in tags {
            dataset(&mut out, 2, 25, tag.as_bytes())?;
        }
    }
    Ok(out)
}

fn resource(out: &mut Vec<u8>, prefix: &[u8], data: &[u8]) {
    out.extend_from_slice(prefix);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

#[derive(Debug)]
pub struct Edited {
    /// One entry for each original Photoshop APP13, and optionally a new last one.
    pub bodies: Vec<Vec<u8>>,
    pub digest: Option<String>,
}

pub fn write(bodies: &[&[u8]], tags: &BTreeSet<String>) -> Result<Edited> {
    let parsed = bodies
        .iter()
        .map(|body| resources(body))
        .collect::<Result<Vec<_>>>()?;
    let mut old = None;
    let mut digest_seen = false;
    for r in parsed.iter().flatten() {
        if r.id == IPTC {
            ensure!(
                old.is_none(),
                "Duplicate Photoshop IPTC resources; original preserved"
            );
            old = Some(r.data);
        } else if r.id == DIGEST {
            ensure!(
                !digest_seen && r.data.len() == 16,
                "Invalid or duplicate Photoshop IPTC digest; original preserved"
            );
            digest_seen = true;
        }
    }
    ensure!(
        !digest_seen || old.is_some(),
        "Photoshop IPTC digest has no IPTC resource; original preserved"
    );
    if old.is_none() && tags.is_empty() {
        return Ok(Edited {
            bodies: bodies.iter().map(|b| b.to_vec()).collect(),
            digest: None,
        });
    }
    let iim = edit_iim(old.unwrap_or_default(), tags)?;
    let digest = md5::compute(&iim);
    let mut rewritten = Vec::new();
    for group in &parsed {
        let mut out = PHOTOSHOP.to_vec();
        for r in group {
            let new_data = match r.id {
                IPTC => iim.as_slice(),
                DIGEST => digest.0.as_slice(),
                _ => r.data,
            };
            if new_data == r.data {
                out.extend_from_slice(r.raw);
            } else {
                resource(&mut out, r.prefix, new_data);
            }
        }
        ensure!(
            out.len() <= 65533,
            "IPTC exceeds JPEG APP13 capacity; original preserved"
        );
        rewritten.push(out);
    }
    if old.is_none() {
        let mut out = PHOTOSHOP.to_vec();
        resource(&mut out, b"8BIM\x04\x04\0\0", &iim);
        ensure!(
            out.len() <= 65533,
            "IPTC exceeds JPEG APP13 capacity; original preserved"
        );
        rewritten.push(out);
    }
    Ok(Edited {
        bodies: rewritten,
        digest: Some(format!("{digest:x}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    fn body(iim: &[u8]) -> Vec<u8> {
        let mut out = PHOTOSHOP.to_vec();
        resource(&mut out, b"8BIM\x04\x04\0\0", iim);
        out
    }
    #[test]
    fn unicode_create_replace_remove_and_idempotent_save() {
        let tags = words(&["järvi", "日本語", "😀", "two words"]);
        let first = write(&[], &tags).unwrap();
        assert_eq!(read(&first.bodies[0]).unwrap(), Some(tags.clone()));
        let same = write(&[&first.bodies[0]], &tags).unwrap();
        assert_eq!(same.bodies, first.bodies);
        let second = write(&[&first.bodies[0]], &words(&["korvattu"])).unwrap();
        assert_eq!(read(&second.bodies[0]).unwrap(), Some(words(&["korvattu"])));
        let cleared = write(&[&second.bodies[0]], &BTreeSet::new()).unwrap();
        assert_eq!(read(&cleared.bodies[0]).unwrap(), Some(BTreeSet::new()));
        assert!(write(&[], &BTreeSet::new()).unwrap().bodies.is_empty());
    }
    #[test]
    fn preserve_named_odd_resources_extended_datasets_and_update_existing_digest() {
        let mut iim = Vec::new();
        dataset(&mut iim, 1, 90, UTF8).unwrap();
        dataset(&mut iim, 2, 25, b"old").unwrap();
        // An unrelated caption with an extended length is kept verbatim.
        let caption = [
            b"\x1c\x02\x78\x80\x04\0\0\0\x05".as_slice(),
            "ää!".as_bytes(),
        ]
        .concat();
        iim.extend_from_slice(&caption);
        let mut original = body(&iim);
        let keep_start = original.len();
        resource(&mut original, b"8BIM\x04\x0f\x03ICC", b"odd");
        let keep = original[keep_start..].to_vec();
        resource(&mut original, b"8BIM\x04\x25\0\0", &[0; 16]);
        let edit = write(&[&original], &words(&["uusi"])).unwrap();
        let parsed = resources(&edit.bodies[0]).unwrap();
        assert_eq!(parsed[1].raw, keep);
        let fields = datasets(parsed[0].data).unwrap();
        assert_eq!(
            fields.iter().find(|d| d.number == 120).unwrap().raw,
            caption
        );
        assert_eq!(parsed[2].data, md5::compute(parsed[0].data).0);
        assert_eq!(
            edit.digest.unwrap(),
            format!("{:x}", md5::compute(parsed[0].data))
        );
    }
    #[test]
    fn unmarked_ascii_is_readable_but_unknown_non_ascii_is_never_guessed() {
        let mut iim = Vec::new();
        dataset(&mut iim, 2, 25, b"ASCII").unwrap();
        let ascii = body(&iim);
        assert_eq!(read(&ascii).unwrap(), Some(words(&["ASCII"])));
        let edit = write(&[&ascii], &words(&["järvi"])).unwrap();
        assert_eq!(read(&edit.bodies[0]).unwrap(), Some(words(&["järvi"])));
        for raw in [b"j\xe4rvi".as_slice(), "järvi".as_bytes()] {
            let mut iim = Vec::new();
            dataset(&mut iim, 2, 25, raw).unwrap();
            let ambiguous = body(&iim);
            assert!(read(&ambiguous)
                .unwrap_err()
                .to_string()
                .contains("ambiguous"));
            assert!(write(&[&ambiguous], &BTreeSet::new()).is_err());
        }
    }
    #[test]
    fn legacy_caption_is_not_reinterpreted_by_a_new_charset() {
        // ISO-2022 escape/shift sequences can be entirely 7-bit: ASCII byte
        // admission alone must not reinterpret them as ordinary UTF-8 text.
        for caption in [b"ol\xe9".as_slice(), b"\x1b$B;3\x1b(B", b"\x0eAB\x0f"] {
            let mut iim = Vec::new();
            dataset(&mut iim, 2, 25, b"old").unwrap();
            dataset(&mut iim, 2, 120, caption).unwrap();
            let original = body(&iim);
            assert!(write(&[&original], &words(&["ää"])).is_err());
            let ascii = write(&[&original], &words(&["new"])).unwrap();
            let parsed = resources(&ascii.bodies[0]).unwrap();
            let fields = datasets(parsed[0].data).unwrap();
            assert!(!utf8_declared(&fields).unwrap());
            assert_eq!(
                fields.iter().find(|d| d.number == 120).unwrap().data,
                caption
            );
        }
    }
    #[test]
    fn malformed_lengths_encodings_duplicate_fields_and_resources_fail() {
        let mut valid = Vec::new();
        dataset(&mut valid, 1, 90, UTF8).unwrap();
        dataset(&mut valid, 2, 25, b"hello").unwrap();
        let original = body(&valid);
        for length in 1..original.len() {
            if length == PHOTOSHOP.len() {
                continue;
            }
            assert!(
                read(&original[..length]).is_err(),
                "accepted truncation {length}"
            );
        }
        for bad in [
            b"\x1c\x02\x19\x80\x00".as_slice(),
            b"\x1c\x02\x19\x80\x05\0\0\0\0\0",
            b"\x1c\x02\x19\x80\x04\xff\xff\xff\xff",
            b"\x1c\x01\x5a\0\x03\x1b-A",
            b"\x1c\x01\x5a\0\x03\x1b%G\x1c\x02\x19\0\x01\xff",
            b"\x1c\x02\x19\0\x01\0",
            b"\x1c\x02\x19\0\x03\x1b$B",
            b"\x1c\x02\x19\0\x03\x0eA\x0f",
        ] {
            assert!(read(&body(bad)).is_err());
        }
        let mut duplicate = valid.clone();
        dataset(&mut duplicate, 1, 90, UTF8).unwrap();
        assert!(read(&body(&duplicate)).is_err());
        let mut duplicate = original.clone();
        duplicate.extend_from_slice(&original[PHOTOSHOP.len()..]);
        assert!(read(&duplicate).is_err());
        assert!(write(&[&original, &original], &words(&["new"])).is_err());
    }
    #[test]
    fn byte_capacity_and_multiple_photoshop_segments_are_bounded() {
        assert!(write(&[], &words(&[&"ä".repeat(32)])).is_ok());
        assert!(write(&[], &words(&[&"ä".repeat(33)])).is_err());
        let tags = (0..1100)
            .map(|i| format!("{i:04}{}", "x".repeat(60)))
            .collect();
        assert!(write(&[], &tags).unwrap_err().to_string().contains("APP13"));
        let mut unrelated = PHOTOSHOP.to_vec();
        resource(&mut unrelated, b"8BIM\x04\x0f\0\0", b"ICC");
        let first = write(&[], &words(&["old"])).unwrap();
        let edit = write(&[&unrelated, &first.bodies[0]], &words(&["new"])).unwrap();
        assert_eq!(edit.bodies[0], unrelated);
        assert_eq!(read(&edit.bodies[1]).unwrap(), Some(words(&["new"])));
    }
}
