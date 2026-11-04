//! Unwraps Steam Cloud's single-entry `"z"` ZIP wrapper (RFC 1951 DEFLATE or STORE).
//! If the payload is not a single-entry `"z"` ZIP (e.g. raw save or a game's own multi-file ZIP),
//! returns `None` so the original bytes are preserved untouched.

pub fn try_unwrap_steam_zip(data: &[u8]) -> Option<Vec<u8>> {
    // Minimum ZIP local file header (30 bytes) + Central Directory (46 bytes) + EOCD (22 bytes)
    if data.len() < 30 || &data[0..4] != b"PK\x03\x04" {
        return None;
    }

    let compression_method = u16::from_le_bytes([data[8], data[9]]);
    let comp_size_hdr = u32::from_le_bytes([data[18], data[19], data[20], data[21]]) as usize;
    let uncomp_size_hdr = u32::from_le_bytes([data[22], data[23], data[24], data[25]]) as usize;
    let name_len = u16::from_le_bytes([data[26], data[27]]) as usize;
    let extra_len = u16::from_le_bytes([data[28], data[29]]) as usize;

    if 30 + name_len + extra_len > data.len() {
        return None;
    }

    // Steam Cloud compression always names the single inner entry "z"
    let entry_name = &data[30..30 + name_len];
    if entry_name != b"z" {
        return None;
    }

    // Verify EOCD at the end of the archive confirms exactly 1 entry
    let (comp_size, uncomp_size) = resolve_zip_sizes(data, comp_size_hdr, uncomp_size_hdr)?;
    if uncomp_size > 512 * 1024 * 1024 {
        return None;
    }

    let payload_start = 30 + name_len + extra_len;
    if payload_start + comp_size > data.len() {
        return None;
    }
    let payload = &data[payload_start..payload_start + comp_size];

    match compression_method {
        0 => Some(payload.to_vec()),
        8 => inflate_raw(payload, uncomp_size),
        _ => None,
    }
}

fn resolve_zip_sizes(data: &[u8], comp_hdr: usize, uncomp_hdr: usize) -> Option<(usize, usize)> {
    // Search backward for EOCD signature PK\x05\x06
    let min_pos = data.len().saturating_sub(65557);
    let mut eocd_pos = None;
    for i in (min_pos..=data.len().saturating_sub(22)).rev() {
        if &data[i..i + 4] == b"PK\x05\x06" {
            eocd_pos = Some(i);
            break;
        }
    }
    let eocd = eocd_pos?;
    let total_entries = u16::from_le_bytes([data[eocd + 10], data[eocd + 11]]);
    if total_entries != 1 {
        return None;
    }
    let cd_offset = u32::from_le_bytes([data[eocd + 16], data[eocd + 17], data[eocd + 18], data[eocd + 19]]) as usize;
    if cd_offset + 46 <= data.len() && &data[cd_offset..cd_offset + 4] == b"PK\x01\x02" {
        let cd_comp = u32::from_le_bytes([
            data[cd_offset + 20],
            data[cd_offset + 21],
            data[cd_offset + 22],
            data[cd_offset + 23],
        ]) as usize;
        let cd_uncomp = u32::from_le_bytes([
            data[cd_offset + 24],
            data[cd_offset + 25],
            data[cd_offset + 26],
            data[cd_offset + 27],
        ]) as usize;
        return Some((cd_comp, cd_uncomp));
    }
    Some((comp_hdr, uncomp_hdr))
}

struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_buf: u64,
    bit_cnt: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            byte_pos: 0,
            bit_buf: 0,
            bit_cnt: 0,
        }
    }

    fn ensure(&mut self, n: u32) {
        while self.bit_cnt < n && self.byte_pos < self.data.len() {
            self.bit_buf |= (self.data[self.byte_pos] as u64) << self.bit_cnt;
            self.byte_pos += 1;
            self.bit_cnt += 8;
        }
    }

    fn peek(&mut self, n: u32) -> u32 {
        self.ensure(n);
        (self.bit_buf & ((1u64 << n) - 1)) as u32
    }

    fn drop_bits(&mut self, n: u32) {
        self.bit_buf >>= n;
        self.bit_cnt = self.bit_cnt.saturating_sub(n);
    }

    fn read_bits(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        self.ensure(n);
        if self.bit_cnt < n {
            return None;
        }
        let val = (self.bit_buf & ((1u64 << n) - 1)) as u32;
        self.drop_bits(n);
        Some(val)
    }

    fn align_byte(&mut self) {
        let rem = self.bit_cnt & 7;
        self.drop_bits(rem);
    }

    fn read_u16_aligned(&mut self) -> Option<u16> {
        self.align_byte();
        let lo = self.read_bits(8)? as u16;
        let hi = self.read_bits(8)? as u16;
        Some(lo | (hi << 8))
    }
}

struct HuffmanTable {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl HuffmanTable {
    fn build(lengths: &[u8]) -> Option<Self> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len as usize >= 16 {
                return None;
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut offsets = [0u16; 16];
        let mut total = 0u16;
        for bits in 1..16 {
            offsets[bits] = total;
            total += counts[bits];
        }
        let mut symbols = vec![0u16; total as usize];
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let idx = offsets[len as usize] as usize;
                symbols[idx] = sym as u16;
                offsets[len as usize] += 1;
            }
        }
        Some(Self { counts, symbols })
    }

    fn decode(&self, br: &mut BitReader) -> Option<u16> {
        let mut code = 0i32;
        let mut first = 0i32;
        let mut index = 0i32;
        for len in 1..=15 {
            code |= br.read_bits(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

fn inflate_raw(input: &[u8], expected_size: usize) -> Option<Vec<u8>> {
    const LEN_BASE: [u16; 29] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258,
    ];
    const LEN_EXTRA: [u8; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DIST_BASE: [u16; 30] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
    ];
    const DIST_EXTRA: [u8; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    const CL_ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];

    let mut br = BitReader::new(input);
    let mut out = Vec::with_capacity(expected_size);

    loop {
        let bfinal = br.read_bits(1)?;
        let btype = br.read_bits(2)?;

        match btype {
            0 => {
                let len = br.read_u16_aligned()?;
                let nlen = br.read_u16_aligned()?;
                if len != !nlen {
                    return None;
                }
                for _ in 0..len {
                    out.push(br.read_bits(8)? as u8);
                }
            }
            1 | 2 => {
                let (lit_table, dist_table) = if btype == 1 {
                    let mut lit_lengths = [0u8; 288];
                    lit_lengths[0..144].fill(8);
                    lit_lengths[144..256].fill(9);
                    lit_lengths[256..280].fill(7);
                    lit_lengths[280..288].fill(8);
                    let dist_lengths = [5u8; 32];
                    (
                        HuffmanTable::build(&lit_lengths)?,
                        HuffmanTable::build(&dist_lengths)?,
                    )
                } else {
                    let hlit = br.read_bits(5)? as usize + 257;
                    let hdist = br.read_bits(5)? as usize + 1;
                    let hclen = br.read_bits(4)? as usize + 4;
                    let mut cl_lengths = [0u8; 19];
                    for i in 0..hclen {
                        cl_lengths[CL_ORDER[i]] = br.read_bits(3)? as u8;
                    }
                    let cl_table = HuffmanTable::build(&cl_lengths)?;
                    let mut all_lengths = Vec::with_capacity(hlit + hdist);
                    while all_lengths.len() < hlit + hdist {
                        let sym = cl_table.decode(&mut br)?;
                        match sym {
                            0..=15 => all_lengths.push(sym as u8),
                            16 => {
                                let prev = *all_lengths.last()?;
                                let repeat = br.read_bits(2)? as usize + 3;
                                for _ in 0..repeat {
                                    all_lengths.push(prev);
                                }
                            }
                            17 => {
                                let repeat = br.read_bits(3)? as usize + 3;
                                for _ in 0..repeat {
                                    all_lengths.push(0);
                                }
                            }
                            18 => {
                                let repeat = br.read_bits(7)? as usize + 11;
                                for _ in 0..repeat {
                                    all_lengths.push(0);
                                }
                            }
                            _ => return None,
                        }
                    }
                    if all_lengths.len() != hlit + hdist {
                        return None;
                    }
                    (
                        HuffmanTable::build(&all_lengths[..hlit])?,
                        HuffmanTable::build(&all_lengths[hlit..])?,
                    )
                };

                loop {
                    let sym = lit_table.decode(&mut br)?;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let len_idx = (sym - 257) as usize;
                        if len_idx >= LEN_BASE.len() {
                            return None;
                        }
                        let length = LEN_BASE[len_idx] as usize
                            + br.read_bits(LEN_EXTRA[len_idx] as u32)? as usize;
                        let dist_sym = dist_table.decode(&mut br)? as usize;
                        if dist_sym >= DIST_BASE.len() {
                            return None;
                        }
                        let dist = DIST_BASE[dist_sym] as usize
                            + br.read_bits(DIST_EXTRA[dist_sym] as u32)? as usize;
                        if dist == 0 || dist > out.len() {
                            return None;
                        }
                        for _ in 0..length {
                            let b = out[out.len() - dist];
                            out.push(b);
                        }
                    }
                }
            }
            _ => return None,
        }

        if bfinal != 0 {
            break;
        }
    }

    let _ = br.peek(0);
    Some(out)
}

