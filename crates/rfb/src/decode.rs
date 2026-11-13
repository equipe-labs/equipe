//! The server's messages, and the framebuffer they draw into.
//!
//! The pixel format is always [`PixelFormat::BGRX`](crate::PixelFormat::BGRX)
//! (the client sets it before asking for anything), so the framebuffer is kept
//! in the window's BGRA byte order and a rectangle is a copy, not a conversion.

use std::io::{self, Read};

use flate2::{Decompress, FlushDecompress, Status};

use crate::wire::encoding;

/// A rectangle of the framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// The smallest rectangle holding both.
    pub fn union(self, other: Rect) -> Rect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let (x0, y0) = (self.x.min(other.x), self.y.min(other.y));
        let x1 = (u32::from(self.x) + u32::from(self.w)).max(u32::from(other.x) + u32::from(other.w));
        let y1 = (u32::from(self.y) + u32::from(self.h)).max(u32::from(other.y) + u32::from(other.h));
        Rect { x: x0, y: y0, w: (x1 - u32::from(x0)) as u16, h: (y1 - u32::from(y0)) as u16 }
    }
}

/// The remote screen as last drawn: BGRA, rows top to bottom, no padding.
#[derive(Debug, Clone)]
pub struct Framebuffer {
    pub width: u16,
    pub height: u16,
    pub data: Vec<u8>,
}

impl Framebuffer {
    pub fn new(width: u16, height: u16) -> Self {
        let mut data = vec![0u8; usize::from(width) * usize::from(height) * 4];
        // Opaque black until drawn.
        for px in data.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Framebuffer { width, height, data }
    }

    /// The part of `x, y, w, h` inside the framebuffer.
    fn clip(&self, x: u16, y: u16, w: u16, h: u16) -> (usize, usize, usize, usize) {
        let (x, y) = (usize::from(x), usize::from(y));
        let (fw, fh) = (usize::from(self.width), usize::from(self.height));
        let w = usize::from(w).min(fw.saturating_sub(x));
        let h = usize::from(h).min(fh.saturating_sub(y));
        (x, y, w, h)
    }

    /// Paint one colour (BGRA) over a rectangle.
    pub fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, bgra: [u8; 4]) {
        let (x, y, w, h) = self.clip(x, y, w, h);
        let stride = usize::from(self.width) * 4;
        for row in y..y + h {
            let start = row * stride + x * 4;
            for px in self.data[start..start + w * 4].as_chunks_mut::<4>().0 {
                px.copy_from_slice(&bgra);
            }
        }
    }

    /// Copy BGRA pixels (`w` across, rows packed) into a rectangle.
    pub fn put(&mut self, x: u16, y: u16, w: u16, h: u16, bgra: &[u8]) {
        let src_stride = usize::from(w) * 4;
        let (cx, cy, cw, ch) = self.clip(x, y, w, h);
        let stride = usize::from(self.width) * 4;
        for row in 0..ch {
            let src = &bgra[row * src_stride..row * src_stride + cw * 4];
            let start = (cy + row) * stride + cx * 4;
            self.data[start..start + cw * 4].copy_from_slice(src);
        }
    }

    /// CopyRect: a rectangle of the framebuffer moved to `x, y`.
    pub fn copy(&mut self, src_x: u16, src_y: u16, x: u16, y: u16, w: u16, h: u16) {
        let (_, _, sw, sh) = self.clip(src_x, src_y, w, h);
        let (_, _, dw, dh) = self.clip(x, y, w, h);
        let (w, h) = (sw.min(dw), sh.min(dh));
        if w == 0 || h == 0 {
            return;
        }
        let stride = usize::from(self.width) * 4;
        let (sx, sy, dx, dy) = (usize::from(src_x), usize::from(src_y), usize::from(x), usize::from(y));
        // Rows in the order that never reads a row already written.
        let rows: Box<dyn Iterator<Item = usize>> = if dy > sy { Box::new((0..h).rev()) } else { Box::new(0..h) };
        for row in rows {
            let from = (sy + row) * stride + sx * 4;
            let to = (dy + row) * stride + dx * 4;
            self.data.copy_within(from..from + w * 4, to);
        }
    }
}

/// What a server message was, once read (and drawn).
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A framebuffer update: what changed, and whether the size did.
    Update { damage: Rect, resized: bool },
    Bell,
    /// The server's clipboard (Latin-1).
    CutText(Vec<u8>),
    /// A fence; answer requests with [`crate::wire::fence_reply`].
    Fence { flags: u32, payload: Vec<u8> },
    /// The server speaks continuous updates (or stopped sending them).
    EndOfContinuousUpdates,
    /// A message with nothing for the window (a colour map: never used, the
    /// format is true colour).
    Other,
}

/// Server message types.
mod server {
    pub const UPDATE: u8 = 0;
    pub const COLOUR_MAP: u8 = 1;
    pub const BELL: u8 = 2;
    pub const CUT_TEXT: u8 = 3;
    pub const END_OF_CONTINUOUS_UPDATES: u8 = 150;
    pub const FENCE: u8 = 248;
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn read_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

fn read_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0u8; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}

fn read_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn read_vec(r: &mut impl Read, len: usize) -> io::Result<Vec<u8>> {
    // Grown as it arrives, so a lying length can't ask for gigabytes up front.
    let mut out = Vec::with_capacity(len.min(1 << 20));
    r.take(len as u64).read_to_end(&mut out)?;
    if out.len() != len {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the server stopped mid-rectangle"));
    }
    Ok(out)
}

/// Tight's compact length: 1–3 bytes, 7 bits each, low bits first.
fn compact_len(r: &mut impl Read) -> io::Result<usize> {
    let mut len = 0usize;
    for i in 0..3 {
        let b = read_u8(r)?;
        len |= usize::from(b & if i == 2 { 0xff } else { 0x7f }) << (7 * i);
        if i == 2 || b & 0x80 == 0 {
            break;
        }
    }
    Ok(len)
}

/// More than this in one rectangle's data is no server's.
const MAX_CHUNK: usize = 64 << 20;

/// Inflate one rectangle's worth from a zlib stream that goes on across
/// rectangles (each ends at a sync flush). `expect`: the size it must come to.
fn inflate(z: &mut Decompress, input: &[u8], expect: Option<usize>) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expect.unwrap_or(input.len() * 4).clamp(64, MAX_CHUNK) + 64);
    let mut used = 0usize;
    loop {
        if out.capacity() - out.len() < 1024 {
            if out.len() >= MAX_CHUNK {
                return Err(bad("a compressed rectangle that large is no server's"));
            }
            out.reserve(out.capacity().max(4096));
        }
        let (in0, out0) = (z.total_in(), z.total_out());
        let status = z.decompress_vec(&input[used..], &mut out, FlushDecompress::Sync).map_err(|e| bad(&format!("zlib: {e}")))?;
        used += (z.total_in() - in0) as usize;
        let made = z.total_out() - out0;
        let room_left = out.capacity() > out.len();
        if status == Status::StreamEnd || (used >= input.len() && room_left) {
            break;
        }
        if made == 0 && z.total_in() == in0 && room_left {
            break;
        }
    }
    if let Some(expect) = expect
        && out.len() != expect
    {
        return Err(bad("a compressed rectangle came to the wrong size"));
    }
    Ok(out)
}

/// `R, G, B` triples to BGRA.
fn rgb_to_bgra(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for p in rgb.as_chunks::<3>().0 {
        out.extend_from_slice(&[p[2], p[1], p[0], 255]);
    }
    out
}

/// Reads server messages and keeps the framebuffer they draw.
pub struct Decoder {
    fb: Framebuffer,
    tight: [Decompress; 4],
    zrle: Decompress,
}

impl Decoder {
    pub fn new(width: u16, height: u16) -> Self {
        Decoder {
            fb: Framebuffer::new(width, height),
            tight: std::array::from_fn(|_| Decompress::new(true)),
            zrle: Decompress::new(true),
        }
    }

    pub fn framebuffer(&self) -> &Framebuffer {
        &self.fb
    }

    /// The encodings this decoder draws, best first, for `SetEncodings`.
    /// `quality`: Tight's JPEG quality (0–9); `compress`: zlib effort (0–9).
    pub fn encodings(quality: u8, compress: u8) -> Vec<i32> {
        vec![
            encoding::COPY_RECT,
            encoding::TIGHT,
            encoding::ZRLE,
            encoding::RAW,
            encoding::quality(quality),
            encoding::compress(compress),
            encoding::DESKTOP_SIZE,
            encoding::EXTENDED_DESKTOP_SIZE,
            encoding::LAST_RECT,
            encoding::FENCE,
            encoding::CONTINUOUS_UPDATES,
        ]
    }

    /// Read one server message, drawing what it carries.
    pub fn read_message(&mut self, r: &mut impl Read) -> io::Result<Event> {
        match read_u8(r)? {
            server::UPDATE => self.update(r),
            server::COLOUR_MAP => {
                let mut head = [0u8; 5];
                r.read_exact(&mut head)?;
                let n = usize::from(u16::from_be_bytes([head[3], head[4]]));
                read_vec(r, n * 6)?;
                Ok(Event::Other)
            }
            server::BELL => Ok(Event::Bell),
            server::CUT_TEXT => {
                let mut pad = [0u8; 3];
                r.read_exact(&mut pad)?;
                let len = read_u32(r)?;
                // The extended clipboard's form (a negative length) is never asked for.
                if len & 0x8000_0000 != 0 || len as usize > MAX_CHUNK {
                    return Err(bad("a clipboard message this client didn't ask for"));
                }
                Ok(Event::CutText(read_vec(r, len as usize)?))
            }
            server::END_OF_CONTINUOUS_UPDATES => Ok(Event::EndOfContinuousUpdates),
            server::FENCE => {
                let mut pad = [0u8; 3];
                r.read_exact(&mut pad)?;
                let flags = read_u32(r)?;
                let len = read_u8(r)?;
                if len > 64 {
                    return Err(bad("a fence longer than 64 bytes"));
                }
                Ok(Event::Fence { flags, payload: read_vec(r, usize::from(len))? })
            }
            other => Err(bad(&format!("server message {other}, which this client never asked for"))),
        }
    }

    fn update(&mut self, r: &mut impl Read) -> io::Result<Event> {
        read_u8(r)?;
        let count = read_u16(r)?;
        let (mut damage, mut resized) = (Rect::default(), false);
        let mut i = 0u32;
        // 0xffff: as many as come before a LastRect.
        while count == 0xffff || i < u32::from(count) {
            i += 1;
            let rect = Rect { x: read_u16(r)?, y: read_u16(r)?, w: read_u16(r)?, h: read_u16(r)? };
            let enc = read_u32(r)? as i32;
            let Rect { x, y, w, h } = rect;
            match enc {
                encoding::RAW => {
                    let mut px = read_vec(r, usize::from(w) * usize::from(h) * 4)?;
                    for p in px.as_chunks_mut::<4>().0 {
                        p[3] = 255;
                    }
                    self.fb.put(x, y, w, h, &px);
                }
                encoding::COPY_RECT => {
                    let (sx, sy) = (read_u16(r)?, read_u16(r)?);
                    self.fb.copy(sx, sy, x, y, w, h);
                }
                encoding::TIGHT => self.tight(r, rect)?,
                encoding::ZRLE => self.zrle(r, rect)?,
                encoding::LAST_RECT => break,
                encoding::DESKTOP_SIZE => {
                    self.resize(w, h);
                    resized = true;
                    continue;
                }
                encoding::EXTENDED_DESKTOP_SIZE => {
                    let screens = read_u8(r)?;
                    read_vec(r, 3 + usize::from(screens) * 16)?;
                    // x: why it changed; y: 0 unless a resize this client asked for failed.
                    if (w, h) != (self.fb.width, self.fb.height) {
                        self.resize(w, h);
                        resized = true;
                    }
                    continue;
                }
                other => return Err(bad(&format!("encoding {other}, which this client never asked for"))),
            }
            damage = damage.union(rect);
        }
        if resized {
            damage = Rect { x: 0, y: 0, w: self.fb.width, h: self.fb.height };
        }
        Ok(Event::Update { damage, resized })
    }

    fn resize(&mut self, w: u16, h: u16) {
        let old = std::mem::replace(&mut self.fb, Framebuffer::new(w, h));
        // What was there stays where it still fits until it is drawn again.
        let (cw, ch) = (old.width.min(w), old.height.min(h));
        let stride = usize::from(old.width) * 4;
        let mut rows = Vec::with_capacity(usize::from(cw) * usize::from(ch) * 4);
        for row in 0..usize::from(ch) {
            rows.extend_from_slice(&old.data[row * stride..row * stride + usize::from(cw) * 4]);
        }
        self.fb.put(0, 0, cw, ch, &rows);
    }

    // ── Tight ──

    fn tight(&mut self, r: &mut impl Read, Rect { x, y, w, h }: Rect) -> io::Result<()> {
        let comp = read_u8(r)?;
        for (i, z) in self.tight.iter_mut().enumerate() {
            if comp & (1 << i) != 0 {
                *z = Decompress::new(true);
            }
        }
        let kind = comp >> 4;
        let pixels = usize::from(w) * usize::from(h);
        match kind {
            // Fill: one colour.
            0x08 => {
                let mut p = [0u8; 3];
                r.read_exact(&mut p)?;
                self.fb.fill(x, y, w, h, [p[2], p[1], p[0], 255]);
            }
            0x09 => {
                let len = compact_len(r)?;
                let jpeg = read_vec(r, len)?;
                let image = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).map_err(|e| bad(&format!("JPEG: {e}")))?.into_rgb8();
                let (iw, ih) = (image.width().min(u32::from(w)) as u16, image.height().min(u32::from(h)) as u16);
                let bgra = rgb_to_bgra(image.as_raw());
                if (u32::from(iw), u32::from(ih)) == (image.width(), image.height()) {
                    self.fb.put(x, y, iw, ih, &bgra);
                } else {
                    // Larger than its rectangle: only the rectangle is drawn.
                    let stride = image.width() as usize * 4;
                    let mut rows = Vec::with_capacity(usize::from(iw) * usize::from(ih) * 4);
                    for row in 0..usize::from(ih) {
                        rows.extend_from_slice(&bgra[row * stride..row * stride + usize::from(iw) * 4]);
                    }
                    self.fb.put(x, y, iw, ih, &rows);
                }
            }
            k if k & 0x08 == 0 => {
                let stream = usize::from(k & 0x03);
                let filter = if k & 0x04 != 0 { read_u8(r)? } else { 0 };
                match filter {
                    // Copy: TPIXELs (R, G, B).
                    0 => {
                        let rgb = self.tight_data(r, stream, pixels * 3)?;
                        self.fb.put(x, y, w, h, &rgb_to_bgra(&rgb));
                    }
                    // Palette: indices into up to 256 colours; 2 colours take a bit each.
                    1 => {
                        let n = usize::from(read_u8(r)?) + 1;
                        let palette: Vec<[u8; 4]> = read_vec(r, n * 3)?.as_chunks::<3>().0.iter().map(|p| [p[2], p[1], p[0], 255]).collect();
                        let row_bytes = if n == 2 { usize::from(w).div_ceil(8) } else { usize::from(w) };
                        let data = self.tight_data(r, stream, row_bytes * usize::from(h))?;
                        let mut bgra = Vec::with_capacity(pixels * 4);
                        for row in 0..usize::from(h) {
                            let line = &data[row * row_bytes..(row + 1) * row_bytes];
                            for col in 0..usize::from(w) {
                                let index = if n == 2 { usize::from((line[col / 8] >> (7 - col % 8)) & 1) } else { usize::from(line[col]) };
                                bgra.extend_from_slice(palette.get(index).unwrap_or(&[0, 0, 0, 255]));
                            }
                        }
                        self.fb.put(x, y, w, h, &bgra);
                    }
                    // Gradient: each component predicted from left, up and up-left.
                    2 => {
                        let data = self.tight_data(r, stream, pixels * 3)?;
                        self.fb.put(x, y, w, h, &rgb_to_bgra(&ungradient(&data, usize::from(w), usize::from(h))));
                    }
                    other => return Err(bad(&format!("Tight filter {other}"))),
                }
            }
            other => return Err(bad(&format!("Tight compression {other:#x}, which this client never asked for"))),
        }
        Ok(())
    }

    /// A basic rectangle's data: as is under 12 bytes, else inflated from its stream.
    fn tight_data(&mut self, r: &mut impl Read, stream: usize, size: usize) -> io::Result<Vec<u8>> {
        if size < 12 {
            return read_vec(r, size);
        }
        let len = compact_len(r)?;
        let packed = read_vec(r, len)?;
        inflate(&mut self.tight[stream], &packed, Some(size))
    }

    // ── ZRLE ──

    fn zrle(&mut self, r: &mut impl Read, Rect { x, y, w, h }: Rect) -> io::Result<()> {
        let len = read_u32(r)? as usize;
        if len > MAX_CHUNK {
            return Err(bad("a ZRLE rectangle that large is no server's"));
        }
        let packed = read_vec(r, len)?;
        let data = inflate(&mut self.zrle, &packed, None)?;
        let mut at = Tiles { data: &data, pos: 0 };
        let mut tile = vec![0u8; 64 * 64 * 4];
        for ty in (0..h).step_by(64) {
            for tx in (0..w).step_by(64) {
                let (tw, th) = ((w - tx).min(64), (h - ty).min(64));
                let n = usize::from(tw) * usize::from(th);
                let pixels = &mut tile[..n * 4];
                at.tile(pixels, usize::from(tw))?;
                self.fb.put(x + tx, y + ty, tw, th, pixels);
            }
        }
        Ok(())
    }
}

/// Undo Tight's gradient filter on `R, G, B` triples.
fn ungradient(data: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 3];
    for row in 0..h {
        for col in 0..w {
            for c in 0..3 {
                let at = |r: usize, cl: usize| i32::from(out[(r * w + cl) * 3 + c]);
                let left = if col > 0 { at(row, col - 1) } else { 0 };
                let up = if row > 0 { at(row - 1, col) } else { 0 };
                let up_left = if row > 0 && col > 0 { at(row - 1, col - 1) } else { 0 };
                let predicted = (left + up - up_left).clamp(0, 255);
                let i = (row * w + col) * 3 + c;
                out[i] = (i32::from(data[i]) + predicted) as u8;
            }
        }
    }
    out
}

/// ZRLE's inflated data, read tile by tile.
struct Tiles<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Tiles<'_> {
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let slice = self.data.get(self.pos..self.pos + n).ok_or_else(|| bad("a ZRLE tile ran past its data"))?;
        self.pos += n;
        Ok(slice)
    }

    fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// A CPIXEL: the pixel's three low bytes, which in this format are B, G, R.
    fn cpixel(&mut self) -> io::Result<[u8; 4]> {
        let p = self.take(3)?;
        Ok([p[0], p[1], p[2], 255])
    }

    fn run(&mut self) -> io::Result<usize> {
        let mut len = 1usize;
        loop {
            let b = self.byte()?;
            len += usize::from(b);
            if b != 255 {
                return Ok(len);
            }
        }
    }

    /// One tile into `out` (BGRA, `tw` across).
    fn tile(&mut self, out: &mut [u8], tw: usize) -> io::Result<()> {
        let n = out.len() / 4;
        let sub = self.byte()?;
        match sub {
            0 => {
                for px in out.as_chunks_mut::<4>().0 {
                    px.copy_from_slice(&self.cpixel()?);
                }
            }
            1 => {
                let c = self.cpixel()?;
                for px in out.as_chunks_mut::<4>().0 {
                    px.copy_from_slice(&c);
                }
            }
            2..=16 => {
                let size = usize::from(sub);
                let palette: Vec<[u8; 4]> = (0..size).map(|_| self.cpixel()).collect::<io::Result<_>>()?;
                let bits = match size {
                    2 => 1,
                    3..=4 => 2,
                    _ => 4,
                };
                let th = n / tw;
                let row_bytes = (tw * bits).div_ceil(8);
                let packed = self.take(row_bytes * th)?;
                for row in 0..th {
                    for col in 0..tw {
                        let bit = col * bits;
                        let byte = packed[row * row_bytes + bit / 8];
                        let index = usize::from((byte >> (8 - bits - bit % 8)) & ((1 << bits) - 1) as u8);
                        let i = (row * tw + col) * 4;
                        out[i..i + 4].copy_from_slice(palette.get(index).unwrap_or(&[0, 0, 0, 255]));
                    }
                }
            }
            128 => {
                let mut i = 0;
                while i < n {
                    let c = self.cpixel()?;
                    let len = self.run()?.min(n - i);
                    for px in out[i * 4..(i + len) * 4].as_chunks_mut::<4>().0 {
                        px.copy_from_slice(&c);
                    }
                    i += len;
                }
            }
            130..=255 => {
                let size = usize::from(sub - 128);
                let palette: Vec<[u8; 4]> = (0..size).map(|_| self.cpixel()).collect::<io::Result<_>>()?;
                let mut i = 0;
                while i < n {
                    let b = self.byte()?;
                    let c = *palette.get(usize::from(b & 0x7f)).unwrap_or(&[0, 0, 0, 255]);
                    let len = if b & 0x80 != 0 { self.run()?.min(n - i) } else { 1 };
                    for px in out[i * 4..(i + len) * 4].as_chunks_mut::<4>().0 {
                        px.copy_from_slice(&c);
                    }
                    i += len;
                }
            }
            other => return Err(bad(&format!("ZRLE subencoding {other}"))),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};
    use std::io::Cursor;

    fn header(count: u16) -> Vec<u8> {
        let mut m = vec![0, 0];
        m.extend(count.to_be_bytes());
        m
    }

    fn rect(m: &mut Vec<u8>, x: u16, y: u16, w: u16, h: u16, enc: i32) {
        for v in [x, y, w, h] {
            m.extend(v.to_be_bytes());
        }
        m.extend(enc.to_be_bytes());
    }

    fn px(fb: &Framebuffer, x: usize, y: usize) -> [u8; 4] {
        let i = (y * usize::from(fb.width) + x) * 4;
        fb.data[i..i + 4].try_into().unwrap()
    }

    /// One rectangle's data from a zlib stream that goes on (a sync flush at its end).
    fn deflate(z: &mut Compress, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(data.len() + 64);
        z.compress_vec(data, &mut out, FlushCompress::Sync).unwrap();
        out
    }

    fn compact(len: usize) -> Vec<u8> {
        let mut out = vec![(len & 0x7f) as u8];
        if len > 0x7f {
            out[0] |= 0x80;
            out.push(((len >> 7) & 0x7f) as u8);
            if len > 0x3fff {
                out[1] |= 0x80;
                out.push((len >> 14) as u8);
            }
        }
        out
    }

    const RED: [u8; 4] = [0, 0, 255, 255];
    const BLUE: [u8; 4] = [255, 0, 0, 255];

    #[test]
    fn raw_and_copyrect_draw_where_they_say() {
        let mut d = Decoder::new(4, 4);
        let mut m = header(2);
        rect(&mut m, 1, 1, 2, 1, encoding::RAW);
        m.extend([0, 0, 255, 0, 255, 0, 0, 0]); // red, blue (the unused byte comes as 0)
        rect(&mut m, 0, 3, 2, 1, encoding::COPY_RECT);
        m.extend([0, 0, 0, 1]); // from (0, 1)
        let ev = d.read_message(&mut Cursor::new(m)).unwrap();
        assert_eq!(ev, Event::Update { damage: Rect { x: 0, y: 1, w: 3, h: 3 }, resized: false });
        let fb = d.framebuffer();
        assert_eq!(px(fb, 1, 1), RED);
        assert_eq!(px(fb, 2, 1), BLUE);
        assert_eq!(px(fb, 1, 3), RED, "copied from (1,1)");
        assert_eq!(px(fb, 0, 3), [0, 0, 0, 255], "copied from (0,1): black");
    }

    #[test]
    fn copyrect_onto_itself_reads_before_it_writes() {
        let mut fb = Framebuffer::new(1, 4);
        for (y, c) in [[1u8, 0, 0, 255], [2, 0, 0, 255], [3, 0, 0, 255], [4, 0, 0, 255]].iter().enumerate() {
            fb.put(0, y as u16, 1, 1, c);
        }
        fb.copy(0, 0, 0, 1, 1, 3); // down by one: a scroll
        assert_eq!([px(&fb, 0, 1)[0], px(&fb, 0, 2)[0], px(&fb, 0, 3)[0]], [1, 2, 3]);
        fb.copy(0, 1, 0, 0, 1, 3); // and back up
        assert_eq!([px(&fb, 0, 0)[0], px(&fb, 0, 1)[0], px(&fb, 0, 2)[0]], [1, 2, 3]);
    }

    #[test]
    fn tight_fill_palette_copy_and_gradient_draw_through_their_streams() {
        let mut d = Decoder::new(16, 4);
        let mut zs: Vec<Compress> = (0..4).map(|_| Compress::new(Compression::default(), true)).collect();
        let mut m = header(5);
        // Fill the whole of it blue.
        rect(&mut m, 0, 0, 16, 4, encoding::TIGHT);
        m.push(0x80);
        m.extend([0, 0, 255]);
        // Two colours, a bit a pixel, stream 1: alternate red and blue on row 0.
        rect(&mut m, 0, 0, 16, 1, encoding::TIGHT);
        m.extend([0x50 /* stream 1, filter follows */, 1, 1]);
        m.extend([255, 0, 0, 0, 0, 255]);
        let bits = [0b0101_0101u8, 0b0101_0101]; // 2 bytes < 12: sent as is
        m.extend(bits);
        // Copy, stream 2: row 1 all red, compressed.
        rect(&mut m, 0, 1, 16, 1, encoding::TIGHT);
        m.push(0x20);
        let red_row: Vec<u8> = std::iter::repeat_n([255u8, 0, 0], 16).flatten().collect();
        let packed = deflate(&mut zs[2], &red_row);
        m.extend(compact(packed.len()));
        m.extend(&packed);
        // Again on stream 2 (the stream goes on): row 2 green.
        rect(&mut m, 0, 2, 16, 1, encoding::TIGHT);
        m.push(0x20);
        let green_row: Vec<u8> = std::iter::repeat_n([0u8, 255, 0], 16).flatten().collect();
        let packed = deflate(&mut zs[2], &green_row);
        m.extend(compact(packed.len()));
        m.extend(&packed);
        // Gradient, stream 3: a flat grey row is its first pixel, then zeros.
        rect(&mut m, 0, 3, 16, 1, encoding::TIGHT);
        m.extend([0x70, 2]);
        let mut filtered = vec![0u8; 16 * 3];
        filtered[..3].copy_from_slice(&[100, 100, 100]);
        let packed = deflate(&mut zs[3], &filtered);
        m.extend(compact(packed.len()));
        m.extend(&packed);
        d.read_message(&mut Cursor::new(m)).unwrap();
        let fb = d.framebuffer();
        assert_eq!(px(fb, 0, 0), RED);
        assert_eq!(px(fb, 1, 0), BLUE);
        assert_eq!(px(fb, 15, 0), BLUE);
        assert_eq!(px(fb, 7, 1), RED);
        assert_eq!(px(fb, 7, 2), [0, 255, 0, 255]);
        assert_eq!(px(fb, 9, 3), [100, 100, 100, 255]);
    }

    #[test]
    fn tight_jpeg_draws_the_picture() {
        let mut jpeg = Vec::new();
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([250, 10, 10]));
        image::DynamicImage::ImageRgb8(img).write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        let mut d = Decoder::new(10, 10);
        let mut m = header(1);
        rect(&mut m, 2, 2, 8, 8, encoding::TIGHT);
        m.push(0x90);
        m.extend(compact(jpeg.len()));
        m.extend(&jpeg);
        d.read_message(&mut Cursor::new(m)).unwrap();
        let p = px(d.framebuffer(), 5, 5);
        assert!(p[2] > 200 && p[0] < 50, "{p:?}");
        assert_eq!(px(d.framebuffer(), 1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn a_reset_bit_starts_its_stream_again() {
        let mut d = Decoder::new(16, 1);
        let row: Vec<u8> = std::iter::repeat_n([1u8, 2, 3], 16).flatten().collect();
        for _ in 0..2 {
            // A fresh compressor each time: only right if the decoder resets stream 0 too.
            let mut z = Compress::new(Compression::default(), true);
            let packed = deflate(&mut z, &row);
            let mut m = header(1);
            rect(&mut m, 0, 0, 16, 1, encoding::TIGHT);
            m.push(0x01); // reset stream 0, basic, copy
            m.extend(compact(packed.len()));
            m.extend(&packed);
            d.read_message(&mut Cursor::new(m)).unwrap();
        }
        assert_eq!(px(d.framebuffer(), 3, 0), [3, 2, 1, 255]);
    }

    #[test]
    fn zrle_tiles_in_every_subencoding() {
        // 66 × 2: two tiles across (64 and 2).
        let mut d = Decoder::new(66, 2);
        let mut z = Compress::new(Compression::default(), true);
        let mut tiles = Vec::new();
        // Tile 1 (64×2), plain RLE: 100 red then 28 blue.
        tiles.push(128);
        tiles.extend([0, 0, 255]);
        tiles.push(99);
        tiles.extend([255, 0, 0]);
        tiles.push(27);
        // Tile 2 (2×2), packed palette of 2: red, blue / blue, red.
        tiles.push(2);
        tiles.extend([0, 0, 255, 255, 0, 0]);
        tiles.extend([0b0100_0000, 0b1000_0000]);
        let mut m = header(3);
        rect(&mut m, 0, 0, 66, 2, encoding::ZRLE);
        let packed = deflate(&mut z, &tiles);
        m.extend((packed.len() as u32).to_be_bytes());
        m.extend(&packed);
        // Then a 2×1 solid green and a palette RLE — on the same stream.
        let mut more = vec![1, 0, 255, 0];
        more.extend([130, 0, 0, 255, 255, 0, 0, 0x81, 1]); // palette RLE: index 1 (blue), a run of 2
        rect(&mut m, 0, 0, 2, 1, encoding::ZRLE);
        let packed = deflate(&mut z, &more[..4]);
        m.extend((packed.len() as u32).to_be_bytes());
        m.extend(&packed);
        rect(&mut m, 2, 0, 2, 1, encoding::ZRLE);
        let packed = deflate(&mut z, &more[4..13]);
        m.extend((packed.len() as u32).to_be_bytes());
        m.extend(&packed);
        d.read_message(&mut Cursor::new(m)).unwrap();
        let fb = d.framebuffer();
        assert_eq!(px(fb, 0, 0), [0, 255, 0, 255], "the solid tile drawn last");
        assert_eq!(px(fb, 2, 0), BLUE, "palette RLE: index 1 is blue");
        assert_eq!(px(fb, 3, 0), BLUE);
        assert_eq!(px(fb, 63, 0), RED);
        assert_eq!(px(fb, 35, 1), RED, "pixel 99 is red");
        assert_eq!(px(fb, 36, 1), BLUE, "pixel 100 is blue");
        assert_eq!(px(fb, 64, 0), RED);
        assert_eq!(px(fb, 65, 0), BLUE);
        assert_eq!(px(fb, 64, 1), BLUE);
        assert_eq!(px(fb, 65, 1), RED);
    }

    #[test]
    fn a_new_desktop_size_and_lastrect_end_the_update() {
        let mut d = Decoder::new(4, 4);
        let mut m = header(0xffff);
        rect(&mut m, 0, 0, 8, 2, encoding::EXTENDED_DESKTOP_SIZE);
        m.extend([1, 0, 0, 0]);
        m.extend([0u8; 16]);
        rect(&mut m, 0, 0, 1, 1, encoding::TIGHT);
        m.extend([0x80, 255, 0, 0]);
        rect(&mut m, 0, 0, 0, 0, encoding::LAST_RECT);
        let ev = d.read_message(&mut Cursor::new(m)).unwrap();
        assert_eq!(ev, Event::Update { damage: Rect { x: 0, y: 0, w: 8, h: 2 }, resized: true });
        assert_eq!((d.framebuffer().width, d.framebuffer().height), (8, 2));
        assert_eq!(px(d.framebuffer(), 0, 0), RED);
    }

    #[test]
    fn fences_bells_and_the_end_of_continuous_updates_are_told() {
        let mut d = Decoder::new(1, 1);
        let mut m = vec![248, 0, 0, 0];
        m.extend((crate::wire::fence::REQUEST | 1).to_be_bytes());
        m.extend([2, 9, 9]);
        m.extend([2, 150]);
        m.extend([3, 0, 0, 0, 0, 0, 0, 2, b'h', b'i']);
        let mut c = Cursor::new(m);
        assert_eq!(d.read_message(&mut c).unwrap(), Event::Fence { flags: crate::wire::fence::REQUEST | 1, payload: vec![9, 9] });
        assert_eq!(d.read_message(&mut c).unwrap(), Event::Bell);
        assert_eq!(d.read_message(&mut c).unwrap(), Event::EndOfContinuousUpdates);
        assert_eq!(d.read_message(&mut c).unwrap(), Event::CutText(b"hi".to_vec()));
    }

    #[test]
    fn rectangles_past_the_edge_are_drawn_clipped_and_lies_are_refused() {
        let mut d = Decoder::new(2, 2);
        let mut m = header(1);
        rect(&mut m, 1, 1, 4, 4, encoding::TIGHT);
        m.extend([0x80, 255, 0, 0]);
        d.read_message(&mut Cursor::new(m)).unwrap();
        assert_eq!(px(d.framebuffer(), 1, 1), RED);
        assert_eq!(px(d.framebuffer(), 0, 0), [0, 0, 0, 255]);
        let mut m = header(1);
        rect(&mut m, 0, 0, 1, 1, 42);
        assert!(d.read_message(&mut Cursor::new(m)).is_err());
        assert!(d.read_message(&mut Cursor::new(vec![77])).is_err());
        let mut m = header(1);
        rect(&mut m, 0, 0, 100, 100, encoding::RAW);
        m.extend([0u8; 10]);
        assert!(d.read_message(&mut Cursor::new(m)).is_err(), "cut short");
    }

    #[test]
    fn the_compact_length_takes_one_to_three_bytes() {
        for len in [0usize, 5, 127, 128, 300, 16383, 16384, 4_000_000] {
            assert_eq!(compact_len(&mut Cursor::new(compact(len))).unwrap(), len, "{len}");
        }
    }
}
