//! Writes photo details into a JPEG as EXIF, so File Explorer shows them on
//! the Details tab, and hashes only the picture data so the hash doesn't
//! change when metadata is added or edited.

use crate::info::Info;

const SOI: [u8; 2] = [0xFF, 0xD8];
const APP0: u8 = 0xE0;
const APP1: u8 = 0xE1;
const SOS: u8 = 0xDA;
const EXIF_ID: &[u8] = b"Exif\0\0";

/// A segment before the image data: (marker, start offset, end offset).
struct Segment {
    marker: u8,
    start: usize,
    end: usize,
}

/// The header segments of a JPEG, and where the image data (SOS) starts.
fn segments(bytes: &[u8]) -> Option<(Vec<Segment>, usize)> {
    if !bytes.starts_with(&SOI) {
        return None;
    }
    let mut segs = Vec::new();
    let mut pos = 2;
    loop {
        if *bytes.get(pos)? != 0xFF {
            return None;
        }
        let mut marker_pos = pos + 1;
        while *bytes.get(marker_pos)? == 0xFF {
            marker_pos += 1; // Fill bytes.
        }
        let marker = bytes[marker_pos];
        if marker == SOS {
            return Some((segs, pos));
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            segs.push(Segment { marker, start: pos, end: marker_pos + 1 });
            pos = marker_pos + 1;
            continue;
        }
        let len = u16::from_be_bytes([*bytes.get(marker_pos + 1)?, *bytes.get(marker_pos + 2)?]) as usize;
        let end = marker_pos + 1 + len;
        if len < 2 || end > bytes.len() {
            return None;
        }
        segs.push(Segment { marker, start: pos, end });
        pos = end;
    }
}

/// The part of the file that metadata edits never touch: for a JPEG, the
/// image data from the start-of-scan marker on; otherwise the whole file.
pub fn picture_data(bytes: &[u8]) -> &[u8] {
    match segments(bytes) {
        Some((_, sos)) => &bytes[sos..],
        None => bytes,
    }
}

/// Returns the JPEG with its EXIF block replaced by one holding `info`.
/// Anything that isn't a JPEG we understand is returned unchanged.
pub fn with_info(bytes: &[u8], info: &Info) -> Vec<u8> {
    let Some((segs, sos)) = segments(bytes) else { return bytes.to_vec() };
    let Some(exif) = exif_segment(info) else { return bytes.to_vec() };

    let mut out = Vec::with_capacity(bytes.len() + exif.len());
    out.extend_from_slice(&SOI);
    let mut placed = false;
    for seg in &segs {
        let data = &bytes[seg.start..seg.end];
        if seg.marker == APP1 && data.get(4..10) == Some(EXIF_ID) {
            continue; // Drop the old EXIF block; ours replaces it.
        }
        // EXIF goes first, or straight after a JFIF header.
        if !placed && seg.marker != APP0 {
            out.extend_from_slice(&exif);
            placed = true;
        }
        out.extend_from_slice(data);
    }
    if !placed {
        out.extend_from_slice(&exif);
    }
    out.extend_from_slice(&bytes[sos..]);
    out
}

// EXIF tags that Windows shows in File Explorer, in ascending order as TIFF requires.
const TAG_COPYRIGHT: u16 = 0x8298; // "Copyright" (ASCII)
const TAG_XP_TITLE: u16 = 0x9C9B; // "Title"
const TAG_XP_COMMENT: u16 = 0x9C9C; // "Comments"
const TAG_XP_AUTHOR: u16 = 0x9C9D; // "Authors"
const TAG_XP_KEYWORDS: u16 = 0x9C9E; // "Tags"
const TAG_XP_SUBJECT: u16 = 0x9C9F; // "Subject"

const TYPE_BYTE: u16 = 1;
const TYPE_ASCII: u16 = 2;

/// An APP1 EXIF segment with the details Explorer can show, or None if there
/// is nothing to write.
fn exif_segment(info: &Info) -> Option<Vec<u8>> {
    let mut comment = info.description.clone().unwrap_or_default();
    if let Some(link) = &info.link {
        if !comment.is_empty() {
            comment.push_str("\r\n\r\n");
        }
        comment.push_str("Learn more: ");
        comment.push_str(link);
    }
    let keywords = match &info.location {
        Some(loc) => format!("Windows Spotlight;{loc}"),
        None => "Windows Spotlight".into(),
    };

    let mut tags: Vec<(u16, u16, Vec<u8>)> = Vec::new();
    if let Some(credit) = &info.credit {
        tags.push((TAG_COPYRIGHT, TYPE_ASCII, ascii(credit)));
    }
    if let Some(loc) = &info.location {
        tags.push((TAG_XP_TITLE, TYPE_BYTE, utf16(loc)));
    }
    if !comment.is_empty() {
        // An APP1 segment holds at most 64 KB; Spotlight text is far shorter.
        let comment: String = comment.chars().take(8000).collect();
        tags.push((TAG_XP_COMMENT, TYPE_BYTE, utf16(&comment)));
    }
    if let Some(credit) = &info.credit {
        tags.push((TAG_XP_AUTHOR, TYPE_BYTE, utf16(credit)));
    }
    if info.location.is_some() || info.credit.is_some() || info.title.is_some() {
        tags.push((TAG_XP_KEYWORDS, TYPE_BYTE, utf16(&keywords)));
    }
    if let Some(title) = &info.title {
        tags.push((TAG_XP_SUBJECT, TYPE_BYTE, utf16(title)));
    }
    if tags.is_empty() {
        return None;
    }

    // Little-endian TIFF: header, one IFD, then the values that don't fit in 4 bytes.
    let mut tiff = b"II*\0".to_vec();
    tiff.extend_from_slice(&8u32.to_le_bytes());
    let ifd_len = 2 + tags.len() * 12 + 4;
    let mut data_offset = 8 + ifd_len;
    let mut data = Vec::new();
    tiff.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for (tag, kind, value) in &tags {
        tiff.extend_from_slice(&tag.to_le_bytes());
        tiff.extend_from_slice(&kind.to_le_bytes());
        tiff.extend_from_slice(&(value.len() as u32).to_le_bytes());
        if value.len() <= 4 {
            let mut inline = value.clone();
            inline.resize(4, 0);
            tiff.extend_from_slice(&inline);
        } else {
            tiff.extend_from_slice(&(data_offset as u32).to_le_bytes());
            data.extend_from_slice(value);
            if value.len() % 2 == 1 {
                data.push(0); // Keep offsets word-aligned.
            }
            data_offset = 8 + ifd_len + data.len();
        }
    }
    tiff.extend_from_slice(&0u32.to_le_bytes()); // No next IFD.
    tiff.extend_from_slice(&data);

    let len = 2 + EXIF_ID.len() + tiff.len();
    let len = u16::try_from(len).ok()?;
    let mut seg = vec![0xFF, APP1];
    seg.extend_from_slice(&len.to_be_bytes());
    seg.extend_from_slice(EXIF_ID);
    seg.extend_from_slice(&tiff);
    Some(seg)
}

/// Windows' XP* tags: UTF-16LE, NUL-terminated.
fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect()
}

/// EXIF ASCII is 7-bit, so © becomes (c) and other characters become '?'.
fn ascii(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for c in s.chars() {
        match c {
            '©' => out.extend_from_slice(b"(c)"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c as u8),
            _ => out.push(b'?'),
        }
    }
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny but structurally valid JPEG: SOI, JFIF APP0, an old EXIF, SOS, data, EOI.
    fn sample_jpeg() -> Vec<u8> {
        let mut j = SOI.to_vec();
        j.extend_from_slice(&[0xFF, APP0, 0x00, 0x10]);
        j.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        j.extend_from_slice(&[0xFF, APP1, 0x00, 0x0B]);
        j.extend_from_slice(b"Exif\0\0old");
        j.extend_from_slice(&[0xFF, 0xDB, 0x00, 0x03, 0x42]); // Fake DQT.
        j.extend_from_slice(&[0xFF, SOS, 0x00, 0x02, 0x12, 0x34, 0x56, 0xFF, 0xD9]);
        j
    }

    fn info() -> Info {
        Info {
            location: Some("A pier in Corinthia, Greece".into()),
            credit: Some("© ilias beros/ 500px Prime / Getty Images".into()),
            title: Some("Peering over smooth seas".into()),
            description: Some("Corinthia is a regional unit of Greece.".into()),
            link: Some("https://www.bing.com/spotlight?spotlightId=PierSeascapeKorinthia".into()),
        }
    }

    #[test]
    fn replaces_exif_and_keeps_picture_data() {
        let original = sample_jpeg();
        let tagged = with_info(&original, &info());
        assert_ne!(tagged, original);
        assert_eq!(picture_data(&tagged), picture_data(&original));

        let (segs, _) = segments(&tagged).unwrap();
        let markers: Vec<u8> = segs.iter().map(|s| s.marker).collect();
        assert_eq!(markers, [APP0, APP1, 0xDB]);
        let exif = &tagged[segs[1].start..segs[1].end];
        assert!(!exif.windows(3).any(|w| w == b"old"), "old EXIF removed");

        // Tagging again replaces our own block instead of stacking another.
        let twice = with_info(&tagged, &info());
        assert_eq!(twice, tagged);
    }

    #[test]
    fn leaves_non_jpeg_alone() {
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        assert_eq!(with_info(&png, &info()), png);
        assert_eq!(picture_data(&png), &png[..]);
    }

    #[test]
    fn nothing_to_write() {
        assert!(exif_segment(&Info::default()).is_none());
    }

    #[test]
    fn ascii_folds_copyright_sign() {
        assert_eq!(ascii("© A / B"), b"(c) A / B\0");
    }
}
