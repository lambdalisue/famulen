//! What a pack holds once unpacked, found out without holding any of it, so
//! that a small pack of something huge never reaches Git.
//!
//! The shape is Git's own: <https://git-scm.com/docs/pack-format>.

use std::io::{self, Read};

use anyhow::Context as _;
use flate2::bufread::ZlibDecoder;

const OFS_DELTA: u8 = 6;
const REF_DELTA: u8 = 7;

/// Make sure that no object of a pack has more than `each` bytes, nor all of
/// them more than `all`. `id` is the length of an object's id in bytes.
pub(super) fn bounded(pack: &[u8], id: usize, each: u64, all: u64) -> anyhow::Result<()> {
    let mut rest = pack;
    let mut head = [0; 12];
    rest.read_exact(&mut head).context("cut short")?;
    anyhow::ensure!(&head[..4] == b"PACK", "not a pack");
    let count = u32::from_be_bytes([head[8], head[9], head[10], head[11]]);
    let mut total = 0u64;
    for _ in 0..count {
        let (kind, size) = header(&mut rest)?;
        match kind {
            OFS_DELTA => while byte(&mut rest)? & 0x80 != 0 {},
            REF_DELTA => rest = rest.get(id..).context("cut short")?,
            _ => {}
        }
        anyhow::ensure!(size <= each, "a file is too large");
        let mut data = ZlibDecoder::new(rest);
        let mut held = size;
        if matches!(kind, OFS_DELTA | REF_DELTA) {
            // A delta says how large its base is and what it makes of it.
            let mut body = io::BufReader::new(data.by_ref().take(size));
            number(&mut body)?;
            held = held.max(number(&mut body)?);
            anyhow::ensure!(held <= each, "a file is too large");
            io::copy(&mut body, &mut io::sink())?;
        } else {
            io::copy(&mut data.by_ref().take(size), &mut io::sink())?;
        }
        // To its end, where the checksum is, so that the next object is found.
        anyhow::ensure!(
            data.read(&mut [0])? == 0,
            "an object is larger than it says"
        );
        rest = data.into_inner();
        total = total.saturating_add(held);
        anyhow::ensure!(total <= all, "too much at once");
    }
    Ok(())
}

/// An object's type and size.
fn header(from: &mut impl Read) -> anyhow::Result<(u8, u64)> {
    let first = byte(from)?;
    let mut size = u64::from(first & 0x0f);
    if first & 0x80 != 0 {
        size |= number(from)?.checked_mul(16).context("a size too large")?;
    }
    Ok(((first >> 4) & 7, size))
}

fn number(from: &mut impl Read) -> anyhow::Result<u64> {
    let mut number = 0u64;
    for shift in (0..56).step_by(7) {
        let byte = byte(from)?;
        number |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(number);
        }
    }
    anyhow::bail!("a size too large")
}

fn byte(from: &mut impl Read) -> anyhow::Result<u8> {
    let mut byte = [0];
    from.read_exact(&mut byte).context("cut short")?;
    Ok(byte[0])
}
