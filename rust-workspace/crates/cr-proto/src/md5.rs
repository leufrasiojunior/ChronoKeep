//! RFC 1321 MD5 incremental implementation for file digest comparison.

use crate::sha1::to_hex;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, // Round 1
    5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, 5,  9, 14, 20, // Round 2
    4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, // Round 3
    6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, // Round 4
];

const K: [u32; 64] = [
    0xD76AA478, 0xE8C7B756, 0x242070DB, 0xC1BDCEEE,
    0xF57C0FAF, 0x4787C62A, 0xA8304613, 0xFD469501,
    0x698098D8, 0x8B44F7AF, 0xFFFF5BB1, 0x895CD7BE,
    0x6B901122, 0xFD987193, 0xA679438E, 0x49B40821,
    0xF61E2562, 0xC040B340, 0x265E5A51, 0xE9B6C7AA,
    0xD62F105D, 0x02441453, 0xD8A1E681, 0xE7D3FBC8,
    0x21E1CDE6, 0xC33707D6, 0xF4D50D87, 0x455A14ED,
    0xA9E3E905, 0xFCEFA3F8, 0x676F02D9, 0x8D2A4C8A,
    0xFFFA3942, 0x8771F681, 0x6D9D6122, 0xFDE5380C,
    0xA4BEEA44, 0x4BDECFA9, 0xF6BB4B60, 0xBEBFBC70,
    0x289B7EC6, 0xEAA127FA, 0xD4EF3085, 0x04881D05,
    0xD9D4D039, 0xE6DB99E5, 0x1FA27CF8, 0xC4AC5665,
    0xF4292244, 0x432AFF97, 0xAB9423A7, 0xFC93A039,
    0x655B59C3, 0x8F0CCC92, 0xFFEFF47D, 0x85845DD1,
    0x6FA87E4F, 0xFE2CE6E0, 0xA3014314, 0x4E0811A1,
    0xF7537E82, 0xBD3AF235, 0x2AD7D2BB, 0xEB86D391,
];

fn process_block(a: &mut u32, b: &mut u32, c: &mut u32, d: &mut u32, chunk: &[u8]) {
    let mut m = [0u32; 16];
    for (i, word) in chunk[..64].chunks_exact(4).enumerate() {
        m[i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
    }

    let mut aa = *a;
    let mut bb = *b;
    let mut cc = *c;
    let mut dd = *d;

    for i in 0..64 {
        let (f, g) = match i {
            0..=15 => ((bb & cc) | ((!bb) & dd), i),
            16..=31 => ((bb & dd) | (cc & (!dd)), (5 * i + 1) % 16),
            32..=47 => (bb ^ cc ^ dd, (3 * i + 5) % 16),
            _ => (cc ^ (bb | (!dd)), (7 * i) % 16),
        };

        let temp = dd;
        dd = cc;
        cc = bb;
        bb = bb.wrapping_add(
            (aa.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g])).rotate_left(S[i]),
        );
        aa = temp;
    }

    *a = a.wrapping_add(aa);
    *b = b.wrapping_add(bb);
    *c = c.wrapping_add(cc);
    *d = d.wrapping_add(dd);
}

#[derive(Clone)]
pub struct Md5Hasher {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    buffer: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Default for Md5Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5Hasher {
    pub fn new() -> Self {
        Self {
            a: 0x67452301,
            b: 0xEFCDAB89,
            c: 0x98BADCFE,
            d: 0x10325476,
            buffer: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self.total_len.wrapping_add(data.len() as u64);

        if self.buf_len > 0 {
            let to_fill = 64 - self.buf_len;
            if data.len() < to_fill {
                self.buffer[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
            self.buffer[self.buf_len..64].copy_from_slice(&data[..to_fill]);
            process_block(&mut self.a, &mut self.b, &mut self.c, &mut self.d, &self.buffer);
            self.buf_len = 0;
            data = &data[to_fill..];
        }

        while data.len() >= 64 {
            process_block(&mut self.a, &mut self.b, &mut self.c, &mut self.d, &data[..64]);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    pub fn finalize(mut self) -> [u8; 16] {
        let bit_len = self.total_len.wrapping_mul(8);

        self.buffer[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            for i in self.buf_len..64 {
                self.buffer[i] = 0;
            }
            process_block(&mut self.a, &mut self.b, &mut self.c, &mut self.d, &self.buffer);
            self.buf_len = 0;
        }

        for i in self.buf_len..56 {
            self.buffer[i] = 0;
        }

        self.buffer[56..64].copy_from_slice(&bit_len.to_le_bytes());
        process_block(&mut self.a, &mut self.b, &mut self.c, &mut self.d, &self.buffer);

        let mut out = [0u8; 16];
        out[0..4].copy_from_slice(&self.a.to_le_bytes());
        out[4..8].copy_from_slice(&self.b.to_le_bytes());
        out[8..12].copy_from_slice(&self.c.to_le_bytes());
        out[12..16].copy_from_slice(&self.d.to_le_bytes());
        out
    }
}

pub fn md5_digest(data: &[u8]) -> [u8; 16] {
    let mut hasher = Md5Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

pub fn md5_hex(data: &[u8]) -> String {
    to_hex(&md5_digest(data))
}

pub fn md5_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Md5Hasher::new();
    let mut buf = [0u8; 65536]; // 64 KiB buffer constante
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_empty() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn test_md5_fox() {
        assert_eq!(
            md5_hex(b"The quick brown fox jumps over the lazy dog"),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
    }

    #[test]
    fn test_md5_incremental() {
        let msg = b"The quick brown fox jumps over the lazy dog";
        let mut hasher = Md5Hasher::new();
        hasher.update(&msg[..10]);
        hasher.update(&msg[10..25]);
        hasher.update(&msg[25..]);
        assert_eq!(to_hex(&hasher.finalize()), "9e107d9d372bb6826bd81d3542a419d6");
    }
}
