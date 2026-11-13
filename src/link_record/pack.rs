//! The snappy compression of a record (T-299).
//!
//! Rules: a payload under [`MIN_PACK`] bytes is never compressed. A
//! compressor that does not make the payload smaller sends it as it is,
//! with no flag. A compressed payload that says it holds more than
//! [`MAX_UNPACKED`] bytes, or more than [`MAX_RATIO`] times its own size,
//! is refused before it is decompressed.

/// A payload under this size is never compressed.
pub const MIN_PACK: usize = 256;

/// The most bytes that a compressed payload may hold: 8 MiB.
pub const MAX_UNPACKED: usize = 8 * 1024 * 1024;

/// The most that a compressed payload may grow: 64 to 1.
pub const MAX_RATIO: usize = 64;

/// Why a compressed payload was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The size or the ratio is over its cap: `decompress-limit`.
    Limit,
    /// The payload is not snappy.
    Bad,
}

/// The compressed form of `data`, or `None` when it is under [`MIN_PACK`]
/// bytes or when the compressed form is not smaller.
pub fn pack(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < MIN_PACK {
        return None;
    }
    let packed = snap::raw::Encoder::new().compress_vec(data).ok()?;
    (packed.len() < data.len()).then_some(packed)
}

/// The bytes of a compressed payload. The size that the payload says is
/// checked against the caps first.
pub fn unpack(packed: &[u8]) -> Result<Vec<u8>, Refused> {
    let size = snap::raw::decompress_len(packed).map_err(|_| Refused::Bad)?;
    if size > MAX_UNPACKED || size > packed.len().saturating_mul(MAX_RATIO) {
        return Err(Refused::Limit);
    }
    snap::raw::Decoder::new()
        .decompress_vec(packed)
        .map_err(|_| Refused::Bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_payload_is_not_packed() {
        assert!(pack(&[0u8; MIN_PACK - 1]).is_none());
        assert!(pack(&[0u8; MIN_PACK]).is_some());
    }

    #[test]
    fn a_payload_that_does_not_shrink_is_not_packed() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let noise: Vec<u8> = (0..4096)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        assert!(pack(&noise).is_none());
    }

    #[test]
    fn the_data_comes_back_equal() {
        let data = b"podnet ".repeat(1000);
        let packed = pack(&data).unwrap();
        assert!(packed.len() < data.len() / 4);
        assert_eq!(unpack(&packed).unwrap(), data);
    }

    #[test]
    fn a_size_over_the_cap_is_refused_before_it_is_decompressed() {
        // The preamble says 9 MiB; the rest is short.
        let mut bad = Vec::new();
        let mut n = 9 * 1024 * 1024usize;
        while n >= 0x80 {
            bad.push((n & 0x7f) as u8 | 0x80);
            n >>= 7;
        }
        bad.push(n as u8);
        bad.extend_from_slice(&[0u8; 10]);
        assert_eq!(unpack(&bad), Err(Refused::Limit));
    }

    #[test]
    fn a_ratio_over_the_cap_is_refused() {
        // 100000 bytes said, 1000 bytes sent: 100 to 1.
        let mut bad = vec![0xa0, 0x8d, 0x06];
        bad.resize(1000, 0);
        assert_eq!(snap::raw::decompress_len(&bad).unwrap(), 100_000);
        assert_eq!(unpack(&bad), Err(Refused::Limit));
        // 20 to 1 is under the cap and fails only as a bad payload.
        let mut ok = vec![0xa0, 0x8d, 0x06];
        ok.resize(5000, 0);
        assert_eq!(unpack(&ok), Err(Refused::Bad));
    }

    #[test]
    fn text_that_is_not_snappy_is_refused() {
        assert_eq!(unpack(b"\xff\xff\xff\xff\xff\xff"), Err(Refused::Bad));
    }
}
