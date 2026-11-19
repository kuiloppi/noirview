//! Lossless metadata editing: image payload bytes are never re-encoded.
use anyhow::{bail, ensure, Context, Result};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use xmltree::{Element, XMLNode};
const XMP_KEY: &str = "XML:com.adobe.xmp";
const JPEG_XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
pub fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".xmp");
    name.into()
}
fn is_element(e: &Element, name: &str) -> bool {
    e.name == name || e.name.rsplit(':').next() == Some(name)
}
fn extract(e: &Element, tags: &mut BTreeSet<String>, subject: bool) {
    let in_subject = subject || is_element(e, "subject");
    if in_subject && is_element(e, "li") {
        if let Some(text) = e.get_text() {
            let t = text.trim();
            if !t.is_empty() {
                tags.insert(t.into());
            }
        }
    }
    for n in &e.children {
        if let XMLNode::Element(child) = n {
            extract(child, tags, in_subject);
        }
    }
}
fn merge_xmp(old: Option<&[u8]>, tags: &BTreeSet<String>) -> Result<Vec<u8>> {
    use quick_xml::{
        events::{BytesEnd, BytesStart, BytesText, Event},
        name::ResolveResult,
        NsReader, Writer,
    };
    let base=b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description rdf:about='' xmlns:dc='http://purl.org/dc/elements/1.1/'/></rdf:RDF></x:xmpmeta>";
    let input = old.unwrap_or(base);
    Element::parse(input).context("Existing XMP is malformed; no changes were made")?;
    let mut reader = NsReader::from_reader(input);
    let mut writer = Writer::new(Vec::new());
    let mut skip = 0usize;
    let mut inserted = false;
    fn subject(writer: &mut Writer<Vec<u8>>, tags: &BTreeSet<String>) -> Result<()> {
        let mut start = BytesStart::new("dc:subject");
        start.push_attribute(("xmlns:dc", "http://purl.org/dc/elements/1.1/"));
        start.push_attribute(("xmlns:rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"));
        writer.write_event(Event::Start(start))?;
        writer.write_event(Event::Start(BytesStart::new("rdf:Bag")))?;
        for tag in tags {
            writer.write_event(Event::Start(BytesStart::new("rdf:li")))?;
            writer.write_event(Event::Text(BytesText::new(tag)))?;
            writer.write_event(Event::End(BytesEnd::new("rdf:li")))?;
        }
        writer.write_event(Event::End(BytesEnd::new("rdf:Bag")))?;
        writer.write_event(Event::End(BytesEnd::new("dc:subject")))?;
        Ok(())
    }
    loop {
        let (ns, event) = reader.read_resolved_event()?;
        let dc = matches!(&ns,ResolveResult::Bound(n) if n.as_ref()==b"http://purl.org/dc/elements/1.1/");
        let rdf = matches!(&ns,ResolveResult::Bound(n) if n.as_ref()==b"http://www.w3.org/1999/02/22-rdf-syntax-ns#");
        if skip > 0 {
            match event {
                Event::Start(_) => skip += 1,
                Event::End(_) => skip -= 1,
                Event::Eof => bail!("Incomplete XMP"),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(e) if dc && e.local_name().as_ref() == b"subject" => skip = 1,
            Event::Empty(e) if dc && e.local_name().as_ref() == b"subject" => {}
            Event::Start(e) if !inserted && rdf && e.local_name().as_ref() == b"Description" => {
                writer.write_event(Event::Start(e))?;
                subject(&mut writer, tags)?;
                inserted = true;
            }
            Event::Empty(e) if !inserted && rdf && e.local_name().as_ref() == b"Description" => {
                let end = e.to_end().into_owned();
                writer.write_event(Event::Start(e))?;
                subject(&mut writer, tags)?;
                writer.write_event(Event::End(end))?;
                inserted = true;
            }
            Event::Eof => break,
            other => writer.write_event(other)?,
        }
    }
    ensure!(inserted, "XMP has no RDF Description; original preserved");
    Ok(writer.into_inner())
}

// Keep existing reconciliation hints current without adding a new XMP property.
// Namespace resolution matters: unrelated properties with this local name survive.
fn sync_iptc_digest(input: &[u8], digest: Option<&str>) -> Result<Vec<u8>> {
    use quick_xml::{
        events::{BytesStart, BytesText, Event},
        name::ResolveResult,
        NsReader, Writer,
    };
    let Some(digest) = digest else {
        return Ok(input.to_vec());
    };
    let digest = digest.to_ascii_uppercase();
    let mut reader = NsReader::from_reader(input);
    let mut writer = Writer::new(Vec::new());
    let mut skip = 0usize;
    loop {
        let (namespace, event) = reader.read_resolved_event()?;
        let photoshop = matches!(&namespace, ResolveResult::Bound(n) if n.as_ref() == b"http://ns.adobe.com/photoshop/1.0/");
        if skip > 0 {
            match event {
                Event::Start(_) | Event::Empty(_) => {
                    bail!("Structured XMP IPTC digest is unsupported; original preserved")
                }
                Event::End(_) => skip -= 1,
                Event::Eof => bail!("Incomplete XMP digest property"),
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let property = photoshop && e.local_name().as_ref() == b"LegacyIPTCDigest";
                let mut changed =
                    BytesStart::new(std::str::from_utf8(e.name().as_ref())?.to_owned());
                for attribute in e.attributes() {
                    let attribute = attribute?;
                    if property {
                        ensure!(
                            attribute.key.as_ref() == b"xmlns"
                                || attribute.key.as_ref().starts_with(b"xmlns:"),
                            "Qualified XMP IPTC digest is unsupported; original preserved"
                        );
                    }
                    let (namespace, name) = reader.resolver().resolve_attribute(attribute.key);
                    if matches!(namespace, ResolveResult::Bound(n) if n.as_ref() == b"http://ns.adobe.com/photoshop/1.0/")
                        && name.as_ref() == b"LegacyIPTCDigest"
                    {
                        changed.push_attribute((attribute.key.as_ref(), digest.as_bytes()));
                    } else {
                        changed.push_attribute(attribute);
                    }
                }
                if property {
                    writer.write_event(Event::Start(changed.clone()))?;
                    writer.write_event(Event::Text(BytesText::new(&digest)))?;
                    writer.write_event(Event::End(changed.to_end()))?;
                    if !empty {
                        skip = 1;
                    }
                } else if empty {
                    writer.write_event(Event::Empty(changed))?;
                } else {
                    writer.write_event(Event::Start(changed))?;
                }
            }
            Event::Eof => break,
            other => writer.write_event(other)?,
        }
    }
    Ok(writer.into_inner())
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
}
fn png_parts(data: &[u8]) -> Result<Vec<(&[u8], &[u8])>> {
    ensure!(data.starts_with(PNG_SIG), "Invalid PNG signature");
    let mut i = 8;
    let mut parts = vec![];
    let mut ended = false;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes(data[i..i + 4].try_into()?) as usize;
        let end = i.checked_add(len + 12).context("PNG chunk overflow")?;
        ensure!(end <= data.len(), "Truncated PNG chunk");
        let kind = &data[i + 4..i + 8];
        let body = &data[i + 8..i + 8 + len];
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(body);
        ensure!(
            crc.finalize() == u32::from_be_bytes(data[end - 4..end].try_into()?),
            "PNG CRC mismatch"
        );
        parts.push((kind, body));
        i = end;
        if kind == b"IEND" {
            ended = true;
            break;
        }
    }
    ensure!(
        ended && i == data.len(),
        "PNG is incomplete or has unsupported trailing data"
    );
    Ok(parts)
}
fn png_xmp<'a>(parts: &[(&[u8], &'a [u8])]) -> Result<Option<&'a [u8]>> {
    for (kind, body) in parts {
        if *kind == b"iTXt" {
            if let Some(z) = body.iter().position(|b| *b == 0) {
                if &body[..z] == XMP_KEY.as_bytes() {
                    ensure!(
                        body.get(z + 1) == Some(&0),
                        "Compressed XMP is not supported; original preserved"
                    );
                    let rest = body.get(z + 3..).context("Malformed iTXt")?;
                    let lang = rest
                        .iter()
                        .position(|b| *b == 0)
                        .context("Malformed iTXt language")?;
                    let rest = &rest[lang + 1..];
                    let translated = rest
                        .iter()
                        .position(|b| *b == 0)
                        .context("Malformed iTXt label")?;
                    return Ok(Some(&rest[translated + 1..]));
                }
            }
        }
    }
    Ok(None)
}
struct JpegSegment<'a> {
    marker: u8,
    raw: &'a [u8],
    body: &'a [u8],
}
fn jpeg_parts(data: &[u8]) -> Result<(Vec<JpegSegment<'_>>, &[u8])> {
    ensure!(data.starts_with(b"\xff\xd8"), "Invalid JPEG");
    let mut i = 2;
    let mut parts = vec![];
    loop {
        let start = i;
        ensure!(data.get(i) == Some(&0xff), "Invalid JPEG marker");
        while data.get(i) == Some(&0xff) {
            i += 1;
        }
        let marker = *data.get(i).context("Truncated JPEG marker")?;
        i += 1;
        if marker == 0xda || marker == 0xd9 {
            return Ok((parts, &data[start..]));
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            parts.push(JpegSegment {
                marker,
                raw: &data[start..i],
                body: &[],
            });
            continue;
        }
        ensure!(marker != 0 && marker != 0xd8, "Invalid JPEG marker");
        let len = u16::from_be_bytes(
            data.get(i..i + 2)
                .context("Truncated JPEG length")?
                .try_into()?,
        ) as usize;
        ensure!(len >= 2 && i + len <= data.len(), "Invalid JPEG segment");
        parts.push(JpegSegment {
            marker,
            raw: &data[start..i + len],
            body: &data[i + 2..i + len],
        });
        i += len;
    }
}
fn jpeg_app1(out: &mut Vec<u8>, body: &[u8]) -> Result<()> {
    jpeg_app(out, 0xe1, body)
}
fn jpeg_app(out: &mut Vec<u8>, marker: u8, body: &[u8]) -> Result<()> {
    ensure!(
        body.len() <= 65533,
        "Keywords exceed JPEG metadata capacity"
    );
    out.extend_from_slice(&[0xff, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
    Ok(())
}
fn webp_xmp(data: &[u8]) -> Result<Option<&[u8]>> {
    ensure!(
        data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP",
        "Invalid WebP"
    );
    let mut i = 12;
    while i + 8 <= data.len() {
        let len = u32::from_le_bytes(data[i + 4..i + 8].try_into()?) as usize;
        ensure!(i + 8 + len <= data.len(), "Truncated WebP");
        if &data[i..i + 4] == b"XMP " {
            return Ok(Some(&data[i + 8..i + 8 + len]));
        }
        i += 8 + len + (len % 2);
    }
    Ok(None)
}
// Keyword indexing must never read or CRC-check compressed image payloads.
// The full validation path in write() is intentionally separate.
const MAX_XMP_BYTES: u64 = 4 * 1024 * 1024;
fn stream_len(reader: &mut (impl std::io::Read + std::io::Seek)) -> Result<u64> {
    use std::io::SeekFrom;
    let end = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    Ok(end)
}
fn bounded_xmp(reader: &mut impl std::io::Read, length: u64) -> Result<Vec<u8>> {
    ensure!(
        length <= MAX_XMP_BYTES,
        "XMP exceeds the 4 MiB indexing limit"
    );
    let mut bytes = vec![0; length as usize];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn side_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::File::open(path) {
        Ok(mut file) => {
            let length = file.metadata()?.len();
            Ok(Some(bounded_xmp(&mut file, length)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn png_stream(reader: &mut (impl std::io::Read + std::io::Seek)) -> Result<Option<Vec<u8>>> {
    use std::io::SeekFrom;
    let end = stream_len(reader)?;
    let mut signature = [0; 8];
    reader.read_exact(&mut signature)?;
    ensure!(signature == PNG_SIG, "Invalid PNG signature");
    loop {
        let start = reader.stream_position()?;
        ensure!(start + 12 <= end, "Incomplete PNG");
        let mut header = [0; 8];
        reader.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header[..4].try_into()?) as u64;
        let next = start
            .checked_add(12 + length)
            .context("PNG chunk overflow")?;
        ensure!(next <= end, "Truncated PNG chunk");
        let kind = &header[4..];
        if kind == b"iTXt" && length > XMP_KEY.len() as u64 {
            let mut prefix = vec![0; XMP_KEY.len() + 1];
            reader.read_exact(&mut prefix)?;
            if prefix[..XMP_KEY.len()] == *XMP_KEY.as_bytes() && prefix[XMP_KEY.len()] == 0 {
                let rest = bounded_xmp(reader, length - prefix.len() as u64)?;
                let mut body = prefix;
                body.extend(rest);
                let mut crc_bytes = [0; 4];
                reader.read_exact(&mut crc_bytes)?;
                let mut crc = crc32fast::Hasher::new();
                crc.update(kind);
                crc.update(&body);
                ensure!(
                    crc.finalize() == u32::from_be_bytes(crc_bytes),
                    "PNG XMP CRC mismatch"
                );
                return Ok(png_xmp(&[(b"iTXt".as_slice(), body.as_slice())])?.map(Vec::from));
            }
        }
        if kind == b"IEND" {
            ensure!(length == 0, "Invalid PNG end chunk");
            return Ok(None);
        }
        reader.seek(SeekFrom::Start(next))?;
    }
}
fn jpeg_stream(
    reader: &mut (impl std::io::Read + std::io::Seek),
) -> Result<(Option<Vec<u8>>, BTreeSet<String>)> {
    use std::io::SeekFrom;
    let end = stream_len(reader)?;
    let mut signature = [0; 2];
    reader.read_exact(&mut signature)?;
    ensure!(signature == [0xff, 0xd8], "Invalid JPEG signature");
    let mut xmp = None;
    let mut tags = BTreeSet::new();
    let mut exif_seen = false;
    let mut iptc_seen = false;
    let mut metadata_bytes = 0;
    loop {
        let mut marker = [0; 2];
        reader.read_exact(&mut marker)?;
        ensure!(marker[0] == 0xff, "Invalid JPEG marker");
        while marker[1] == 0xff {
            reader.read_exact(&mut marker[1..])?;
        }
        if marker[1] == 0xda || marker[1] == 0xd9 {
            return Ok((xmp, tags));
        }
        if marker[1] == 0x01 || (0xd0..=0xd7).contains(&marker[1]) {
            continue;
        }
        let mut length_bytes = [0; 2];
        reader.read_exact(&mut length_bytes)?;
        let length = u16::from_be_bytes(length_bytes) as u64;
        ensure!(length >= 2, "Invalid JPEG segment length");
        let start = reader.stream_position()?;
        let next = start + length - 2;
        ensure!(next <= end, "Truncated JPEG segment");
        if matches!(marker[1], 0xe1 | 0xed) {
            metadata_bytes += length - 2;
            ensure!(
                metadata_bytes <= MAX_XMP_BYTES,
                "JPEG metadata exceeds the 4 MiB indexing limit"
            );
            let body = bounded_xmp(reader, length - 2)?;
            if marker[1] == 0xed {
                if body.starts_with(crate::iptc::PHOTOSHOP) {
                    if let Some(words) = crate::iptc::read(&body)? {
                        ensure!(!iptc_seen, "Duplicate JPEG IPTC resources");
                        iptc_seen = true;
                        tags.extend(words);
                    }
                }
            } else if body.starts_with(JPEG_XMP) {
                ensure!(xmp.is_none(), "Duplicate JPEG XMP segments");
                xmp = Some(body[JPEG_XMP.len()..].to_vec());
            } else if body.starts_with(crate::xpkeywords::EXIF) {
                ensure!(!exif_seen, "Duplicate JPEG EXIF segments");
                exif_seen = true;
                tags.extend(crate::xpkeywords::read(&body)?);
            }
        }
        reader.seek(SeekFrom::Start(next))?;
    }
}
fn webp_stream(reader: &mut (impl std::io::Read + std::io::Seek)) -> Result<Option<Vec<u8>>> {
    use std::io::SeekFrom;
    let file_end = stream_len(reader)?;
    let mut header = [0; 12];
    reader.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..] == b"WEBP",
        "Invalid WebP signature"
    );
    let end = 8 + u32::from_le_bytes(header[4..8].try_into()?) as u64;
    ensure!(end >= 12 && end <= file_end, "Truncated WebP");
    while reader.stream_position()? + 8 <= end {
        let mut chunk = [0; 8];
        reader.read_exact(&mut chunk)?;
        let length = u32::from_le_bytes(chunk[4..].try_into()?) as u64;
        let next = reader.stream_position()? + length + length % 2;
        ensure!(next <= end, "Truncated WebP chunk");
        if &chunk[..4] == b"XMP " {
            return Ok(Some(bounded_xmp(reader, length)?));
        }
        reader.seek(SeekFrom::Start(next))?;
    }
    Ok(None)
}
pub fn read(path: &Path) -> Result<BTreeSet<String>> {
    let side = sidecar(path);
    let side_bytes = side_bytes(&side)?.unwrap_or_default();
    let mut tags = BTreeSet::new();
    let xmp = if !side_bytes.is_empty() {
        Some(side_bytes)
    } else {
        match extension(path).as_str() {
            "png" => png_stream(&mut fs::File::open(path)?)?,
            "jpg" | "jpeg" => {
                let (xmp, xp) = jpeg_stream(&mut fs::File::open(path)?)?;
                tags.extend(xp);
                xmp
            }
            "webp" => webp_stream(&mut fs::File::open(path)?)?,
            // Formats using sidecars need no access to image pixels at all.
            _ => None,
        }
    };
    if let Some(bytes) = xmp {
        let root = Element::parse(bytes.as_slice())?;
        extract(&root, &mut tags, false);
    }
    Ok(tags)
}
fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
pub fn write(path: &Path, tags: &BTreeSet<String>) -> Result<()> {
    ensure!(
        tags.iter().all(|t| !t.is_empty()
            && t.trim() == t
            && t.len() <= 1024
            && t.chars().all(|c| matches!(c, '\t' | '\n' | '\r')
                || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}'))),
        "Tags must contain 1–1024 bytes, without surrounding spaces or invalid XML characters"
    );
    write_with_check(path, tags, || {})
}
fn write_with_check(
    path: &Path,
    tags: &BTreeSet<String>,
    before_commit: impl FnOnce(),
) -> Result<()> {
    use std::io::{Read, Write};
    let mut source = fs::File::open(path)?;
    let before = source.metadata()?;
    let mut original = vec![];
    source.read_to_end(&mut original)?;
    // Windows cannot replace a destination while this source handle remains open.
    // Keep the snapshot, then reopen independently for the final content check.
    drop(source);
    // Parse and stage every sidecar before publishing any image changes.
    let side = sidecar(path);
    let old_side = side_bytes(&side)?;
    let ext = extension(path);
    let mut jpeg_digest = None;
    let output = match ext.as_str() {
        "png" => {
            let parts = png_parts(&original)?;
            let xmp = merge_xmp(png_xmp(&parts)?, tags)?;
            let mut out = PNG_SIG.to_vec();
            let mut body = XMP_KEY.as_bytes().to_vec();
            body.extend_from_slice(&[0, 0, 0, 0, 0]);
            body.extend_from_slice(&xmp);
            for (kind, data) in parts {
                if kind == b"IEND" {
                    chunk(&mut out, b"iTXt", &body);
                }
                if kind == b"iTXt"
                    && data.starts_with(XMP_KEY.as_bytes())
                    && data.get(XMP_KEY.len()) == Some(&0)
                {
                    continue;
                }
                chunk(&mut out, kind.try_into()?, data);
            }
            Some(out)
        }
        "jpg" | "jpeg" => {
            let (parts, scan) = jpeg_parts(&original)?;
            let mut old_xmp = None;
            let mut old_exif = None;
            for p in &parts {
                if p.marker == 0xe1 && p.body.starts_with(JPEG_XMP) {
                    ensure!(
                        old_xmp.is_none(),
                        "Duplicate JPEG XMP segments; original preserved"
                    );
                    old_xmp = Some(&p.body[JPEG_XMP.len()..]);
                } else if p.marker == 0xe1 && p.body.starts_with(crate::xpkeywords::EXIF) {
                    ensure!(
                        old_exif.is_none(),
                        "Duplicate JPEG EXIF segments; original preserved"
                    );
                    old_exif = Some(p.body);
                }
            }
            let xmp = merge_xmp(old_xmp, tags)?;
            let exif = crate::xpkeywords::write(old_exif, tags)?;
            let photoshop = parts
                .iter()
                .filter(|p| p.marker == 0xed && p.body.starts_with(crate::iptc::PHOTOSHOP))
                .map(|p| p.body)
                .collect::<Vec<_>>();
            let iptc = crate::iptc::write(&photoshop, tags)?;
            jpeg_digest = iptc.digest;
            let xmp = sync_iptc_digest(&xmp, jpeg_digest.as_deref())?;
            let mut photoshop_out = iptc.bodies.iter();
            let mut xmp_body = JPEG_XMP.to_vec();
            xmp_body.extend(xmp);
            let mut out = b"\xff\xd8".to_vec();
            // Preserve JFIF/ICC and EXIF placement; add missing metadata before the scan.
            let mut wrote_exif = false;
            let mut wrote_xmp = false;
            for p in parts {
                if p.marker == 0xe1 && p.body.starts_with(JPEG_XMP) {
                    jpeg_app1(&mut out, &xmp_body)?;
                    wrote_xmp = true;
                } else if p.marker == 0xe1 && p.body.starts_with(crate::xpkeywords::EXIF) {
                    jpeg_app1(&mut out, &exif)?;
                    wrote_exif = true;
                } else if p.marker == 0xed && p.body.starts_with(crate::iptc::PHOTOSHOP) {
                    jpeg_app(
                        &mut out,
                        0xed,
                        photoshop_out
                            .next()
                            .context("Missing prepared Photoshop resource")?,
                    )?;
                } else {
                    out.extend_from_slice(p.raw);
                }
            }
            if !wrote_exif {
                jpeg_app1(&mut out, &exif)?;
            }
            if !wrote_xmp {
                jpeg_app1(&mut out, &xmp_body)?;
            }
            for body in photoshop_out {
                jpeg_app(&mut out, 0xed, body)?;
            }
            out.extend_from_slice(scan);
            Some(out)
        }
        "webp" => {
            let xmp = merge_xmp(webp_xmp(&original)?, tags)?;
            let (w, h) = image::image_dimensions(path)?;
            ensure!(
                w > 0 && h > 0 && w <= 0x1000000 && h <= 0x1000000,
                "WebP dimensions out of range"
            );
            let mut payload = b"WEBP".to_vec();
            let mut i = 12;
            let has_vp8x = original.get(12..16) == Some(b"VP8X");
            if !has_vp8x {
                let alpha = if image::load_from_memory(&original)?.color().has_alpha() {
                    0x10
                } else {
                    0
                };
                let mut vp = vec![0x04 | alpha, 0, 0, 0];
                vp.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
                vp.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
                payload.extend_from_slice(b"VP8X");
                payload.extend_from_slice(&10u32.to_le_bytes());
                payload.extend_from_slice(&vp);
            }
            while i + 8 <= original.len() {
                let len = u32::from_le_bytes(original[i + 4..i + 8].try_into()?) as usize;
                ensure!(
                    i + 8 + len + (len % 2) <= original.len(),
                    "Invalid WebP chunk"
                );
                let kind = &original[i..i + 4];
                if kind != b"XMP " {
                    let start = payload.len();
                    payload.extend_from_slice(&original[i..i + 8 + len + (len % 2)]);
                    if kind == b"VP8X" && len >= 10 {
                        payload[start + 8] |= 0x04;
                    }
                }
                i += 8 + len + (len % 2);
            }
            payload.extend_from_slice(b"XMP ");
            payload.extend_from_slice(&(xmp.len() as u32).to_le_bytes());
            payload.extend_from_slice(&xmp);
            if xmp.len() % 2 == 1 {
                payload.push(0);
            }
            let mut out = b"RIFF".to_vec();
            out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&payload);
            Some(out)
        }
        _ => None,
    };
    let side_output = if output.is_none() || old_side.is_some() {
        Some(sync_iptc_digest(
            &merge_xmp(old_side.as_deref(), tags)?,
            jpeg_digest.as_deref(),
        )?)
    } else {
        None
    };
    ensure!(
        output.is_none() || !before.permissions().readonly(),
        "Image is read-only; originals preserved"
    );
    if side_output.is_some() && old_side.is_some() {
        ensure!(
            !fs::metadata(&side)?.permissions().readonly(),
            "XMP sidecar is read-only; originals preserved"
        );
    }
    // Verify the future authoritative keyword set before publishing either file.
    let mut prepared_tags = BTreeSet::new();
    let prepared_xmp = if let Some(side) = &side_output {
        Some(side.clone())
    } else if let Some(bytes) = &output {
        match ext.as_str() {
            "jpg" | "jpeg" => {
                let (xmp, xp) = jpeg_stream(&mut std::io::Cursor::new(bytes))?;
                prepared_tags.extend(xp);
                xmp
            }
            "png" => png_stream(&mut std::io::Cursor::new(bytes))?,
            "webp" => webp_stream(&mut std::io::Cursor::new(bytes))?,
            _ => None,
        }
    } else {
        None
    };
    if let Some(xmp) = prepared_xmp {
        ensure!(
            xmp.len() as u64 <= MAX_XMP_BYTES,
            "XMP exceeds the 4 MiB editing limit"
        );
        extract(&Element::parse(xmp.as_slice())?, &mut prepared_tags, false);
    }
    ensure!(
        prepared_tags == *tags,
        "Prepared keyword verification failed; originals preserved"
    );
    if let Some(bytes) = &output {
        let a = image::load_from_memory(&original).context("Cannot validate original image")?;
        let b =
            image::load_from_memory(bytes).context("Metadata output failed image validation")?;
        ensure!(
            a.to_rgba8() == b.to_rgba8(),
            "Image pixels changed during metadata update; original preserved"
        );
    }
    fn stage(path: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
        let mut temp =
            tempfile::NamedTempFile::new_in(path.parent().context("Missing parent directory")?)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        Ok(temp)
    }
    let image_temp = output
        .as_ref()
        .map(|bytes| stage(path, bytes))
        .transpose()?;
    let side_temp = side_output
        .as_ref()
        .map(|bytes| stage(&side, bytes))
        .transpose()?;
    before_commit();
    // A byte comparison also detects same-size/time replacements by another editor.
    ensure_unchanged(path, &before, &original)?;
    match &old_side {
        Some(bytes) => ensure!(
            side_bytes(&side)?.as_ref() == Some(bytes),
            "XMP sidecar changed while editing; retry"
        ),
        None => ensure!(
            !side.try_exists()?,
            "XMP sidecar appeared while editing; retry"
        ),
    }
    if let Some(temp) = image_temp {
        temp.persist(path).map_err(|e| e.error)?;
    }
    if let Some(temp) = side_temp {
        temp.persist(&side).map_err(|e| e.error)?;
    }
    #[cfg(unix)]
    fs::File::open(path.parent().context("Missing parent directory")?)?.sync_all()?;
    if read(path)? != *tags {
        bail!("Keyword verification failed");
    }
    Ok(())
}
fn ensure_unchanged(path: &Path, before: &fs::Metadata, original: &[u8]) -> Result<()> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let after = file.metadata()?;
    ensure!(
        before.len() == after.len() && before.modified()? == after.modified()?,
        "Image changed while editing; retry"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            (
                before.dev(),
                before.ino(),
                before.ctime(),
                before.ctime_nsec()
            ) == (after.dev(), after.ino(), after.ctime(), after.ctime_nsec()),
            "Image replaced while editing; retry"
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            before.creation_time() == after.creation_time(),
            "Image replaced while editing; retry"
        );
    }
    let mut offset = 0;
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ensure!(
            original.get(offset..offset + n) == Some(&buf[..n]),
            "Image contents changed while editing; retry"
        );
        offset += n;
    }
    ensure!(
        offset == original.len(),
        "Image contents changed while editing; retry"
    );
    let current = fs::metadata(path)?;
    ensure!(
        current.len() == after.len() && current.modified()? == after.modified()?,
        "Image changed during final check; retry"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            (
                current.dev(),
                current.ino(),
                current.ctime(),
                current.ctime_nsec()
            ) == (after.dev(), after.ino(), after.ctime(), after.ctime_nsec()),
            "Image replaced during final check; retry"
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            current.creation_time() == after.creation_time(),
            "Image replaced during final check; retry"
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn jpeg(path: &Path) {
        image::RgbImage::from_fn(31, 23, |x, y| {
            image::Rgb([(x * 7) as u8, (y * 9) as u8, 67])
        })
        .save(path)
        .unwrap();
    }
    fn insert(path: &Path, body: &[u8]) {
        let original = fs::read(path).unwrap();
        let mut out = original[..2].to_vec();
        jpeg_app1(&mut out, body).unwrap();
        out.extend_from_slice(&original[2..]);
        fs::write(path, out).unwrap();
    }
    #[test]
    fn jpeg_xp_and_xmp_union_add_replace_clear_and_exact_payload_preservation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("windows.jpg");
        jpeg(&path);
        let xp = BTreeSet::from(["järvi".into(), "日本語".into(), "😀".into()]);
        insert(&path, &crate::xpkeywords::write(None, &xp).unwrap());
        let old_xmp = merge_xmp(None, &BTreeSet::from(["XMP only".into()])).unwrap();
        insert(&path, &[JPEG_XMP, &old_xmp].concat());
        let original = fs::read(&path).unwrap();
        let (old_parts, old_scan) = jpeg_parts(&original).unwrap();
        let mut expected = xp;
        expected.insert("XMP only".into());
        assert_eq!(read(&path).unwrap(), expected);
        // The same fresh-read additive merge used by save_tags.
        let mut additive = read(&path).unwrap();
        additive.insert("naïve é ç".into());
        for tags in [
            additive,
            BTreeSet::from(["korvattu".into()]),
            BTreeSet::new(),
        ] {
            write(&path, &tags).unwrap();
            assert_eq!(read(&path).unwrap(), tags);
            let bytes = fs::read(&path).unwrap();
            let (parts, scan) = jpeg_parts(&bytes).unwrap();
            assert_eq!(scan, old_scan);
            let keep = |p: &&JpegSegment<'_>| {
                !(p.marker == 0xe1
                    && (p.body.starts_with(JPEG_XMP)
                        || p.body.starts_with(crate::xpkeywords::EXIF))
                    || p.marker == 0xed && p.body.starts_with(crate::iptc::PHOTOSHOP))
            };
            assert_eq!(
                parts.iter().filter(keep).map(|p| p.raw).collect::<Vec<_>>(),
                old_parts
                    .iter()
                    .filter(keep)
                    .map(|p| p.raw)
                    .collect::<Vec<_>>()
            );
            let exif = parts
                .iter()
                .find(|p| p.body.starts_with(crate::xpkeywords::EXIF))
                .unwrap();
            assert_eq!(crate::xpkeywords::read(exif.body).unwrap(), tags);
        }
    }
    #[test]
    fn sidecar_is_authoritative_and_both_keyword_fields_are_synchronized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpeg");
        jpeg(&path);
        insert(
            &path,
            &crate::xpkeywords::write(None, &BTreeSet::from(["embedded".into()])).unwrap(),
        );
        fs::write(
            sidecar(&path),
            merge_xmp(None, &BTreeSet::from(["sidecar".into()])).unwrap(),
        )
        .unwrap();
        assert_eq!(read(&path).unwrap(), BTreeSet::from(["sidecar".into()]));
        write(&path, &BTreeSet::new()).unwrap();
        assert!(read(&path).unwrap().is_empty());
        fs::remove_file(sidecar(&path)).unwrap();
        assert!(read(&path).unwrap().is_empty());
    }
    #[test]
    fn independent_iptc_fixture_add_replace_delete_clear_reopen_and_preserve() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iptc.jpg");
        let original = include_bytes!("../tests/fixtures/iptc-keywords.jpg");
        fs::write(&path, original).unwrap();
        let mut expected = BTreeSet::from([
            "järvi".into(),
            "日本語".into(),
            "😀".into(),
            "XMP only".into(),
            "XP only".into(),
        ]);
        assert_eq!(read(&path).unwrap(), expected);
        expected.insert("ajouté".into());
        let mut removed = expected.clone();
        removed.remove("järvi");
        let (_, old_scan) = jpeg_parts(original).unwrap();
        for tags in [
            expected,
            removed,
            BTreeSet::from(["korvattu".into()]),
            BTreeSet::new(),
        ] {
            write(&path, &tags).unwrap();
            assert_eq!(read(&path).unwrap(), tags);
            let bytes = fs::read(&path).unwrap();
            let (parts, scan) = jpeg_parts(&bytes).unwrap();
            assert_eq!(scan, old_scan);
            let iptc = parts
                .iter()
                .find(|p| p.marker == 0xed && p.body.starts_with(crate::iptc::PHOTOSHOP))
                .unwrap();
            assert_eq!(crate::iptc::read(iptc.body).unwrap(), Some(tags.clone()));
            assert!(iptc
                .body
                .windows("Säilytä kuvateksti".len())
                .any(|w| w == "Säilytä kuvateksti".as_bytes()));
            assert!(iptc
                .body
                .windows(b"Test Photographer".len())
                .any(|w| w == b"Test Photographer"));
            let exif = parts
                .iter()
                .find(|p| p.marker == 0xe1 && p.body.starts_with(crate::xpkeywords::EXIF))
                .unwrap();
            assert_eq!(crate::xpkeywords::read(exif.body).unwrap(), tags);
            let xmp = parts
                .iter()
                .find(|p| p.marker == 0xe1 && p.body.starts_with(JPEG_XMP))
                .unwrap();
            let mut xmp_words = BTreeSet::new();
            extract(
                &Element::parse(&xmp.body[JPEG_XMP.len()..]).unwrap(),
                &mut xmp_words,
                false,
            );
            assert_eq!(xmp_words, tags);
            assert!(!String::from_utf8_lossy(xmp.body).contains("00000000000000000000000000000000"));
        }
        // A valid, authoritative empty subject must not let IPTC words reappear.
        fs::write(&path, original).unwrap();
        fs::write(sidecar(&path), merge_xmp(None, &BTreeSet::new()).unwrap()).unwrap();
        assert!(read(&path).unwrap().is_empty());
        write(&path, &BTreeSet::new()).unwrap();
        fs::remove_file(sidecar(&path)).unwrap();
        assert!(read(&path).unwrap().is_empty());
    }
    #[test]
    fn ambiguous_malformed_duplicate_and_oversized_iptc_preserve_image_and_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("iptc.jpg");
        let original = include_bytes!("../tests/fixtures/iptc-keywords.jpg");
        let (parts, scan) = jpeg_parts(original).unwrap();
        let body = parts.iter().find(|p| p.marker == 0xed).unwrap().body;
        // Strip the UTF-8 declaration without changing byte lengths: an unknown
        // dataset retains its bytes but no longer declares a character set.
        let mut ambiguous = body.to_vec();
        let charset = ambiguous
            .windows(3)
            .position(|w| w == [0x1c, 1, 90])
            .unwrap();
        ambiguous[charset + 2] = 89;
        for bodies in [
            vec![ambiguous.as_slice()],
            vec![&body[..body.len() - 1]],
            vec![body, body],
            vec![body],
        ] {
            let mut input = b"\xff\xd8".to_vec();
            for p in parts.iter().filter(|p| p.marker != 0xed) {
                input.extend_from_slice(p.raw);
            }
            for b in &bodies {
                jpeg_app(&mut input, 0xed, b).unwrap();
            }
            input.extend_from_slice(scan);
            fs::write(&path, &input).unwrap();
            let side = merge_xmp(None, &BTreeSet::from(["authoritative".into()])).unwrap();
            fs::write(sidecar(&path), &side).unwrap();
            let tags = if bodies.len() == 1 && bodies[0] == body {
                BTreeSet::from(["ä".repeat(33)])
            } else {
                BTreeSet::new()
            };
            assert!(write(&path, &tags).is_err());
            assert_eq!(fs::read(&path).unwrap(), input);
            assert_eq!(fs::read(sidecar(&path)).unwrap(), side);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
        }
    }
    #[test]
    fn digest_sync_resolves_namespaces_and_handles_attributes_and_elements() {
        let input = br#"<x:xmpmeta xmlns:x='adobe:ns:meta/' xmlns:p='http://ns.adobe.com/photoshop/1.0/' xmlns:o='urn:other'><root p:LegacyIPTCDigest='OLD' o:LegacyIPTCDigest='KEEP'><p:LegacyIPTCDigest>OLD</p:LegacyIPTCDigest><p:LegacyIPTCDigest/><o:LegacyIPTCDigest>KEEP</o:LegacyIPTCDigest></root></x:xmpmeta>"#;
        let output = sync_iptc_digest(input, Some("aabbcc")).unwrap();
        Element::parse(output.as_slice()).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.matches("AABBCC").count(), 3);
        assert_eq!(text.matches("KEEP").count(), 2);
        assert!(!text.contains("OLD"));
        for input in [
            br#"<p:LegacyIPTCDigest xmlns:p='http://ns.adobe.com/photoshop/1.0/'><other>keep</other></p:LegacyIPTCDigest>"#.as_slice(),
            br#"<p:LegacyIPTCDigest xmlns:p='http://ns.adobe.com/photoshop/1.0/' resource='keep'/>"#,
        ] {
            assert!(sync_iptc_digest(input, Some("new")).is_err());
        }
    }
    #[test]
    fn invalid_sidecar_never_partially_edits_any_embedded_format() {
        let dir = tempfile::tempdir().unwrap();
        for ext in ["png", "jpg", "webp", "bmp"] {
            let path = dir.path().join(format!("photo.{ext}"));
            image::RgbImage::new(8, 8).save(&path).unwrap();
            let before = fs::read(&path).unwrap();
            fs::write(sidecar(&path), b"<broken").unwrap();
            let error = write(&path, &BTreeSet::from(["new".into()])).unwrap_err();
            assert!(error.to_string().contains("malformed"), "{error}");
            assert_eq!(fs::read(&path).unwrap(), before);
            assert_eq!(fs::read(sidecar(&path)).unwrap(), b"<broken");
            assert_eq!(
                fs::read_dir(dir.path())
                    .unwrap()
                    .filter_map(Result::ok)
                    .count(),
                2 * (1 + ["png", "jpg", "webp", "bmp"]
                    .iter()
                    .position(|e| *e == ext)
                    .unwrap())
            );
        }
    }
    #[test]
    fn concurrent_image_or_sidecar_edits_abort_before_either_publication() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        let tags = BTreeSet::from(["new".into()]);
        for change_side in [false, true] {
            jpeg(&path);
            let before = fs::read(&path).unwrap();
            let side_before = merge_xmp(None, &BTreeSet::new()).unwrap();
            fs::write(sidecar(&path), &side_before).unwrap();
            let modified = fs::metadata(&path).unwrap().modified().unwrap();
            let result = write_with_check(&path, &tags, || {
                if change_side {
                    fs::write(sidecar(&path), b"externally changed sidecar").unwrap();
                } else {
                    let mut changed = before.clone();
                    *changed.last_mut().unwrap() ^= 1;
                    fs::write(&path, &changed).unwrap();
                    fs::File::options()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_modified(modified)
                        .unwrap();
                }
            });
            assert!(result.is_err());
            if change_side {
                assert_eq!(fs::read(&path).unwrap(), before);
                assert_eq!(
                    fs::read(sidecar(&path)).unwrap(),
                    b"externally changed sidecar"
                );
            } else {
                assert_ne!(fs::read(&path).unwrap(), before);
                assert_eq!(fs::read(sidecar(&path)).unwrap(), side_before);
            }
        }
    }
    #[test]
    fn invalid_exif_or_unrepresentable_keyword_preserves_original_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        jpeg(&path);
        let original = fs::read(&path).unwrap();
        assert!(write(&path, &BTreeSet::from(["one;two".into()])).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        insert(&path, b"Exif\0\0broken");
        let malformed = fs::read(&path).unwrap();
        assert!(write(&path, &BTreeSet::new()).is_err());
        assert_eq!(fs::read(&path).unwrap(), malformed);
    }
    #[test]
    fn keywords_roundtrip_without_changing_pixels() {
        let dir = tempfile::tempdir().unwrap();
        for ext in ["png", "jpg", "webp", "bmp", "tiff"] {
            let path = dir.path().join(format!("test.{ext}"));
            image::RgbImage::from_fn(20, 15, |x, y| {
                image::Rgb([(x * 5) as u8, (y * 8) as u8, 91])
            })
            .save(&path)
            .unwrap();
            let original = image::open(&path).unwrap().to_rgba8();
            let tags = BTreeSet::from(["night & city".into(), "日本語".into()]);
            write(&path, &tags).unwrap();
            assert_eq!(read(&path).unwrap(), tags);
            assert_eq!(image::open(&path).unwrap().to_rgba8(), original);
            write(&path, &BTreeSet::new()).unwrap();
            assert!(read(&path).unwrap().is_empty());
        }
    }
    #[test]
    fn xmp_edit_preserves_other_metadata() {
        let old=b"<x:xmpmeta xmlns:x='adobe:ns:meta/'><rdf:RDF xmlns:rdf='http://www.w3.org/1999/02/22-rdf-syntax-ns#'><rdf:Description rdf:about='photo' xmlns:dc='http://purl.org/dc/elements/1.1/'><dc:title>Keep me</dc:title><dc:subject><rdf:Bag><rdf:li>old</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>";
        let out = merge_xmp(Some(old), &BTreeSet::from(["new".into()])).unwrap();
        assert!(String::from_utf8(out.clone()).unwrap().contains("Keep me"));
        assert!(String::from_utf8(out.clone())
            .unwrap()
            .contains("rdf:about='photo'"));
        let root = Element::parse(out.as_slice()).unwrap();
        let mut tags = BTreeSet::new();
        extract(&root, &mut tags, false);
        assert_eq!(tags, BTreeSet::from(["new".into()]));
    }
    #[test]
    fn damaged_png_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.png");
        fs::write(&path, b"not a PNG").unwrap();
        assert!(write(&path, &BTreeSet::new()).is_err());
        assert_eq!(fs::read(path).unwrap(), b"not a PNG");
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    struct CountReads<R> {
        inner: R,
        bytes: usize,
    }
    impl<R: Read> Read for CountReads<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.bytes += n;
            Ok(n)
        }
    }
    impl<R: Seek> Seek for CountReads<R> {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }
    #[test]
    fn png_keywords_after_large_image_payload_are_read_without_scanning_pixels() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(PNG_SIG).unwrap();
        let payload = 512 * 1024 * 1024u32;
        file.write_all(&payload.to_be_bytes()).unwrap();
        file.write_all(b"IDAT").unwrap();
        file.seek(SeekFrom::Start(16 + payload as u64 + 4)).unwrap();
        let tags = BTreeSet::from(["large image".into()]);
        let xmp = merge_xmp(None, &tags).unwrap();
        let mut body = XMP_KEY.as_bytes().to_vec();
        body.extend_from_slice(&[0, 0, 0, 0, 0]);
        body.extend_from_slice(&xmp);
        let mut trailer = vec![];
        chunk(&mut trailer, b"iTXt", &body);
        chunk(&mut trailer, b"IEND", &[]);
        file.write_all(&trailer).unwrap();
        file.rewind().unwrap();
        let mut reader = CountReads {
            inner: file,
            bytes: 0,
        };
        let result = png_stream(&mut reader).unwrap().unwrap();
        let root = Element::parse(result.as_slice()).unwrap();
        let mut extracted = BTreeSet::new();
        extract(&root, &mut extracted, false);
        assert_eq!(extracted, tags);
        assert!(
            reader.bytes < 4096,
            "read {} bytes of compressed data",
            reader.bytes
        );
    }
    #[test]
    fn jpeg_indexing_stops_at_the_compressed_scan() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"\xff\xd8\xff\xda").unwrap();
        file.set_len(512 * 1024 * 1024).unwrap();
        file.rewind().unwrap();
        let mut reader = CountReads {
            inner: file,
            bytes: 0,
        };
        let (xmp, tags) = jpeg_stream(&mut reader).unwrap();
        assert!(xmp.is_none() && tags.is_empty());
        assert_eq!(reader.bytes, 4);
    }
    #[test]
    fn iptc_indexing_reads_headers_only_and_bounds_aggregate_metadata() {
        let tags = BTreeSet::from(["järvi".into()]);
        let iptc = crate::iptc::write(&[], &tags).unwrap();
        let mut headers = b"\xff\xd8".to_vec();
        jpeg_app(&mut headers, 0xed, &iptc.bodies[0]).unwrap();
        headers.extend_from_slice(b"\xff\xda");
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&headers).unwrap();
        file.set_len(512 * 1024 * 1024).unwrap();
        file.rewind().unwrap();
        let mut reader = CountReads {
            inner: file,
            bytes: 0,
        };
        assert_eq!(jpeg_stream(&mut reader).unwrap().1, tags);
        assert_eq!(reader.bytes, headers.len());
        let mut headers = b"\xff\xd8".to_vec();
        for _ in 0..65 {
            jpeg_app(&mut headers, 0xed, &[0; 65533]).unwrap();
        }
        headers.extend_from_slice(b"\xff\xda");
        let mut reader = CountReads {
            inner: std::io::Cursor::new(headers),
            bytes: 0,
        };
        assert!(jpeg_stream(&mut reader)
            .unwrap_err()
            .to_string()
            .contains("4 MiB"));
        assert!(reader.bytes <= MAX_XMP_BYTES as usize + 400);
    }
    #[test]
    fn webp_indexing_seeks_over_large_image_chunks() {
        let mut file = tempfile::tempfile().unwrap();
        let payload = 512 * 1024 * 1024u32;
        let tags = BTreeSet::from(["webp".into()]);
        let xmp = merge_xmp(None, &tags).unwrap();
        let padding = xmp.len() % 2;
        file.write_all(b"RIFF").unwrap();
        file.write_all(&(4 + 8 + payload + 8 + xmp.len() as u32 + padding as u32).to_le_bytes())
            .unwrap();
        file.write_all(b"WEBPVP8 ").unwrap();
        file.write_all(&payload.to_le_bytes()).unwrap();
        file.seek(SeekFrom::Start(20 + payload as u64)).unwrap();
        file.write_all(b"XMP ").unwrap();
        file.write_all(&(xmp.len() as u32).to_le_bytes()).unwrap();
        file.write_all(&xmp).unwrap();
        if padding == 1 {
            file.write_all(&[0]).unwrap();
        }
        file.rewind().unwrap();
        let mut reader = CountReads {
            inner: file,
            bytes: 0,
        };
        assert_eq!(webp_stream(&mut reader).unwrap().unwrap(), xmp);
        assert!(reader.bytes < 4096);
    }
    #[test]
    fn oversized_xmp_is_rejected_without_allocating_the_payload() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(PNG_SIG).unwrap();
        let length = 16 * 1024 * 1024u32;
        file.write_all(&length.to_be_bytes()).unwrap();
        file.write_all(b"iTXtXML:com.adobe.xmp\0").unwrap();
        file.set_len(16 + length as u64 + 4).unwrap();
        file.rewind().unwrap();
        let mut reader = CountReads {
            inner: file,
            bytes: 0,
        };
        assert!(png_stream(&mut reader).is_err());
        assert!(reader.bytes < 100);
    }
}
