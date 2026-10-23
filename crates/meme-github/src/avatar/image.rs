//! Bounded raster framing checks; image data is never decoded or re-encoded here.

pub(super) fn valid(mime: &str, body: &[u8]) -> bool {
    let result = match mime {
        "image/png" => png(body),
        "image/jpeg" => jpeg(body),
        "image/gif" => gif(body),
        "image/webp" => webp(body),
        _ => None,
    };
    result.is_some()
}

fn take<'a>(rest: &mut &'a [u8], size: usize) -> Option<&'a [u8]> {
    let (value, tail) = rest.split_at_checked(size)?;
    *rest = tail;
    Some(value)
}

fn byte(rest: &mut &[u8]) -> Option<u8> {
    Some(take(rest, 1)?[0])
}

fn be16(value: &[u8]) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes(value.try_into().ok()?)))
}

fn le16(value: &[u8]) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes(value.try_into().ok()?)))
}

fn be32(value: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(value.try_into().ok()?))
}

fn le32(value: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(value.try_into().ok()?))
}

fn le24(value: &[u8]) -> Option<u32> {
    let value: &[u8; 3] = value.try_into().ok()?;
    Some(u32::from_le_bytes([value[0], value[1], value[2], 0]))
}

fn dimensions(width: u32, height: u32) -> bool {
    (1..=1024).contains(&width) && (1..=1024).contains(&height)
}

fn png(body: &[u8]) -> Option<()> {
    let mut rest = body
        .strip_prefix(b"\x89PNG\r\n\x1a\n")?
        .strip_suffix(b"\0\0\0\0IEND\xaeB`\x82")?;
    let (kind, header) = png_chunk(&mut rest)?;
    png_header(kind, header)?;
    let mut pixels = false;
    while !rest.is_empty() {
        let (kind, data) = png_chunk(&mut rest)?;
        pixels |= png_data(kind, data)?;
    }
    pixels.then_some(())
}

fn png_chunk<'a>(rest: &mut &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let size = be32(take(rest, 4)?)? as usize;
    let kind = take(rest, 4)?;
    let data = take(rest, size)?;
    let checksum = be32(take(rest, 4)?)?;
    (crc32(kind, data) == checksum).then_some((kind, data))
}

fn crc32(kind: &[u8], data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn png_header(kind: &[u8], data: &[u8]) -> Option<()> {
    if kind != b"IHDR" {
        return None;
    }
    png_fields(data.try_into().ok()?)
}

fn png_fields(data: &[u8; 13]) -> Option<()> {
    let size = dimensions(be32(&data[..4])?, be32(&data[4..8])?);
    let modes = data[10..12] == [0, 0] && data[12] <= 1;
    (size && modes && png_depth(data[8], data[9])).then_some(())
}

fn png_depth(depth: u8, color: u8) -> bool {
    let allowed: &[u8] = match color {
        0 => &[1, 2, 4, 8, 16],
        2 | 4 | 6 => &[8, 16],
        3 => &[1, 2, 4, 8],
        _ => &[],
    };
    allowed.contains(&depth)
}

fn png_data(kind: &[u8], data: &[u8]) -> Option<bool> {
    match kind {
        b"IDAT" => Some(!data.is_empty()),
        b"PLTE" => {
            ((3..=768).contains(&data.len()) && data.len().is_multiple_of(3)).then_some(false)
        }
        _ => (kind[0].is_ascii_lowercase() && kind.iter().all(u8::is_ascii_alphabetic))
            .then_some(false),
    }
}

fn jpeg(body: &[u8]) -> Option<()> {
    let mut rest = body.strip_prefix(b"\xff\xd8")?;
    let (mut frame, mut scan, mut seen_scan) = (false, false, false);
    loop {
        let marker = jpeg_marker(&mut rest, scan)?;
        if marker == 0xd9 {
            return jpeg_complete(rest, frame, seen_scan);
        }
        (frame, scan) = jpeg_state(&mut rest, marker, frame)?;
        seen_scan |= scan;
    }
}

fn jpeg_complete(rest: &[u8], frame: bool, scan: bool) -> Option<()> {
    (rest.is_empty() && frame && scan).then_some(())
}

fn jpeg_state(rest: &mut &[u8], marker: u8, frame: bool) -> Option<(bool, bool)> {
    let data = jpeg_segment(rest, marker)?;
    let frame = frame | jpeg_frame(marker, data)?;
    let scan = jpeg_scan(marker, data, frame)?;
    Some((frame, scan))
}

fn jpeg_marker(rest: &mut &[u8], entropy: bool) -> Option<u8> {
    if entropy {
        return jpeg_entropy_marker(rest);
    }
    (byte(rest)? == 0xff).then_some(())?;
    let mut marker = byte(rest)?;
    while marker == 0xff {
        marker = byte(rest)?;
    }
    Some(marker)
}

fn jpeg_entropy_marker(rest: &mut &[u8]) -> Option<u8> {
    loop {
        let count = rest.iter().position(|byte| *byte == 0xff)?;
        take(rest, count)?;
        let marker = jpeg_marker(rest, false)?;
        if marker == 0 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        return Some(marker);
    }
}

fn jpeg_segment<'a>(rest: &mut &'a [u8], marker: u8) -> Option<&'a [u8]> {
    let allowed = (0xc0..=0xfe).contains(&marker) && !(0xd0..=0xd9).contains(&marker);
    allowed.then_some(())?;
    let size = be16(take(rest, 2)?)? as usize;
    take(rest, size.checked_sub(2)?)
}

fn jpeg_frame(marker: u8, data: &[u8]) -> Option<bool> {
    const FRAMES: &[u8] = &[
        0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
    ];
    if !FRAMES.contains(&marker) {
        return Some(false);
    }
    jpeg_frame_data(data)
}

fn jpeg_frame_data(data: &[u8]) -> Option<bool> {
    let header = data.get(..6)?;
    let size = dimensions(be16(&header[3..5])?, be16(&header[1..3])?);
    let channels = header[5];
    let format = [8, 12, 16].contains(&header[0])
        && [1, 3, 4].contains(&channels)
        && data.len() == 6 + 3 * usize::from(channels);
    (size && format).then_some(true)
}

fn jpeg_scan(marker: u8, data: &[u8], frame: bool) -> Option<bool> {
    if marker != 0xda {
        return Some(false);
    }
    let channels = *data.first()?;
    (frame && (1..=4).contains(&channels) && data.len() == 4 + 2 * usize::from(channels))
        .then_some(true)
}

fn gif(body: &[u8]) -> Option<()> {
    let mut rest = body;
    gif_header(&mut rest)?;
    let mut pixels = false;
    while rest.first() != Some(&0x3b) {
        pixels |= gif_block(&mut rest)?;
    }
    take(&mut rest, 1)?;
    (pixels && rest.is_empty()).then_some(())
}

fn gif_header(rest: &mut &[u8]) -> Option<()> {
    let header = take(rest, 13)?;
    let signature = [b"GIF87a", b"GIF89a"].contains(&header[..6].try_into().ok()?);
    let size = dimensions(le16(&header[6..8])?, le16(&header[8..10])?);
    (signature && size).then_some(())?;
    gif_palette(rest, header[10])
}

fn gif_palette(rest: &mut &[u8], flags: u8) -> Option<()> {
    if flags & 0x80 != 0 {
        take(rest, 3 * (1 << ((flags & 7) + 1)))?;
    }
    Some(())
}

fn gif_block(rest: &mut &[u8]) -> Option<bool> {
    match byte(rest)? {
        0x2c => gif_image(rest).map(|()| true),
        0x21 => {
            byte(rest)?;
            gif_sub_blocks(rest).map(|_| false)
        }
        _ => None,
    }
}

fn gif_image(rest: &mut &[u8]) -> Option<()> {
    let size = gif_descriptor(rest)?;
    let lzw = (2..=8).contains(&byte(rest)?);
    let bytes = gif_sub_blocks(rest)?;
    (size && lzw && bytes > 0).then_some(())
}

fn gif_descriptor(rest: &mut &[u8]) -> Option<bool> {
    let header = take(rest, 9)?;
    let size = dimensions(le16(&header[4..6])?, le16(&header[6..8])?);
    gif_palette(rest, header[8])?;
    Some(size)
}

fn gif_sub_blocks(rest: &mut &[u8]) -> Option<usize> {
    let mut total = 0;
    loop {
        let size = usize::from(byte(rest)?);
        if size == 0 {
            return Some(total);
        }
        total += take(rest, size)?.len();
    }
}

fn webp(body: &[u8]) -> Option<()> {
    let header = body.get(..12)?;
    let signature = &header[..4] == b"RIFF" && &header[8..] == b"WEBP";
    let size = u64::from(le32(&header[4..8])?) + 8 == body.len() as u64;
    (signature && size).then_some(())?;
    webp_chunks(&body[12..], true)?.then_some(())
}

fn webp_chunks(mut rest: &[u8], animation: bool) -> Option<bool> {
    let mut pixels = false;
    while !rest.is_empty() {
        let (kind, data) = webp_chunk(&mut rest)?;
        pixels |= webp_data(kind, data, animation)?;
    }
    Some(pixels)
}

fn webp_chunk<'a>(rest: &mut &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let kind = take(rest, 4)?;
    let size = le32(take(rest, 4)?)? as usize;
    let data = take(rest, size)?;
    if !size.is_multiple_of(2) {
        (byte(rest)? == 0).then_some(())?;
    }
    Some((kind, data))
}

fn webp_data(kind: &[u8], data: &[u8], animation: bool) -> Option<bool> {
    match kind {
        b"VP8 " => webp_lossy(data).map(|()| true),
        b"VP8L" => webp_lossless(data).map(|()| true),
        b"ANMF" => {
            animation.then_some(())?;
            webp_frame(data)
        }
        _ => webp_metadata(kind, data).map(|()| false),
    }
}

fn webp_lossy(data: &[u8]) -> Option<()> {
    let header = data.get(..11)?;
    let signature = header[0] & 1 == 0 && &header[3..6] == b"\x9d\x01\x2a";
    let size = dimensions(
        le16(&header[6..8])? & 0x3fff,
        le16(&header[8..10])? & 0x3fff,
    );
    (signature && size).then_some(())
}

fn webp_lossless(data: &[u8]) -> Option<()> {
    let header = data.get(..6)?;
    let bits = le32(&header[1..5])?;
    let size = dimensions(1 + (bits & 0x3fff), 1 + ((bits >> 14) & 0x3fff));
    (header[0] == 0x2f && bits >> 29 == 0 && size).then_some(())
}

fn webp_metadata(kind: &[u8], data: &[u8]) -> Option<()> {
    if kind == b"VP8X" {
        return webp_canvas(data);
    }
    let allowed: &[&[u8]] = &[b"ALPH", b"ANIM", b"ICCP", b"EXIF", b"XMP "];
    (allowed.contains(&kind) && !data.is_empty()).then_some(())
}

fn webp_canvas(data: &[u8]) -> Option<()> {
    let data: &[u8; 10] = data.try_into().ok()?;
    let size = dimensions(1 + le24(&data[4..7])?, 1 + le24(&data[7..10])?);
    (data[0] & 0xc1 == 0 && data[1..4] == [0, 0, 0] && size).then_some(())
}

fn webp_frame(data: &[u8]) -> Option<bool> {
    let header = data.get(..16)?;
    let size = dimensions(1 + le24(&header[6..9])?, 1 + le24(&header[9..12])?);
    (size && header[15] <= 3).then_some(())?;
    webp_chunks(&data[16..], false)
}
