//! PE Section & MSVC x64 RTTI Scanner for `steamclient64.dll`.
//! Dynamically locates:
//! 1. `CClientUnifiedServiceTransport` vtable via RTTI (`.?AVCClientUnifiedServiceTransport@@`)
//! 2. Protobuf `SerializeToArray` and `ParseFromArray` inside `steamclient64.dll`

#[derive(Debug, Clone, Copy)]
pub struct SectionSpan {
    pub base: usize,
    pub size: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct SteamClientSymbols {
    pub module_base: usize,
    pub transport_vtable: usize,
    pub serialize_to_array: usize,
    pub parse_from_array: usize,
}

pub unsafe fn resolve_steamclient_symbols(module_base: usize) -> Option<SteamClientSymbols> {
    if module_base == 0 {
        return None;
    }

    crate::log_debug(&format!(
        "[Scanner] Escaneando secoes PE em steamclient64.dll (base=0x{:X})...",
        module_base
    ));

    let (text, rdata, data) = parse_pe_sections(module_base)?;
    crate::log_debug(&format!(
        "[Scanner] Secoes PE mapeadas: .text=0x{:X} ({}KB), .rdata=0x{:X} ({}KB), .data=0x{:X} ({}KB)",
        text.base,
        text.size / 1024,
        rdata.base,
        rdata.size / 1024,
        data.base,
        data.size / 1024
    ));

    let transport_vtable = resolve_vtable_by_rtti(
        module_base,
        text,
        rdata,
        data,
        b".?AVCClientUnifiedServiceTransport@@\0",
    )?;
    crate::log_debug(&format!(
        "[Scanner] RTTI VTable localizada em 0x{:X} (RVA 0x{:X})",
        transport_vtable,
        transport_vtable - module_base
    ));

    let serialize_to_array = resolve_serialize_to_array(text)?;
    crate::log_debug(&format!(
        "[Scanner] SerializeToArray localizado em 0x{:X} (RVA 0x{:X})",
        serialize_to_array,
        serialize_to_array - module_base
    ));

    let parse_from_array = resolve_parse_from_array(text, serialize_to_array)?;
    crate::log_debug(&format!(
        "[Scanner] ParseFromArray localizado em 0x{:X} (RVA 0x{:X})",
        parse_from_array,
        parse_from_array - module_base
    ));

    Some(SteamClientSymbols {
        module_base,
        transport_vtable,
        serialize_to_array,
        parse_from_array,
    })
}

unsafe fn parse_pe_sections(
    module_base: usize,
) -> Option<(SectionSpan, SectionSpan, SectionSpan)> {
    let base_ptr = module_base as *const u8;
    let e_magic = u16::from_le_bytes([*base_ptr, *base_ptr.add(1)]);
    if e_magic != 0x5A4D {
        return None; // Not "MZ"
    }

    let e_lfanew = i32::from_le_bytes([
        *base_ptr.add(0x3C),
        *base_ptr.add(0x3D),
        *base_ptr.add(0x3E),
        *base_ptr.add(0x3F),
    ]) as usize;

    let nt_ptr = base_ptr.add(e_lfanew);
    let pe_sig = u32::from_le_bytes([
        *nt_ptr,
        *nt_ptr.add(1),
        *nt_ptr.add(2),
        *nt_ptr.add(3),
    ]);
    if pe_sig != 0x0000_4550 {
        return None; // Not "PE\0\0"
    }

    let num_sections = u16::from_le_bytes([*nt_ptr.add(6), *nt_ptr.add(7)]) as usize;
    let size_of_opt_hdr = u16::from_le_bytes([*nt_ptr.add(20), *nt_ptr.add(21)]) as usize;
    let first_section = nt_ptr.add(24 + size_of_opt_hdr);

    let mut text = SectionSpan { base: 0, size: 0 };
    let mut rdata = SectionSpan { base: 0, size: 0 };
    let mut data = SectionSpan { base: 0, size: 0 };

    for i in 0..num_sections {
        let sec = first_section.add(i * 40);
        let name = std::slice::from_raw_parts(sec, 8);
        let virtual_size = u32::from_le_bytes([
            *sec.add(8),
            *sec.add(9),
            *sec.add(10),
            *sec.add(11),
        ]) as usize;
        let virtual_addr = u32::from_le_bytes([
            *sec.add(12),
            *sec.add(13),
            *sec.add(14),
            *sec.add(15),
        ]) as usize;

        if name.starts_with(b".text\0") {
            text = SectionSpan {
                base: module_base + virtual_addr,
                size: virtual_size,
            };
        } else if name.starts_with(b".rdata") {
            rdata = SectionSpan {
                base: module_base + virtual_addr,
                size: virtual_size,
            };
        } else if name.starts_with(b".data\0") {
            data = SectionSpan {
                base: module_base + virtual_addr,
                size: virtual_size,
            };
        }
    }

    if text.base == 0 || rdata.base == 0 {
        return None;
    }
    Some((text, rdata, data))
}

unsafe fn resolve_vtable_by_rtti(
    module_base: usize,
    _text: SectionSpan,
    rdata: SectionSpan,
    data: SectionSpan,
    rtti_name: &[u8],
) -> Option<usize> {
    let name_addr = find_bytes_in_span(data, rtti_name)
        .or_else(|| find_bytes_in_span(rdata, rtti_name))?;

    // In 64-bit MSVC RTTI, TypeDescriptor begins 0x10 bytes before the mangled name string
    let td_addr = name_addr.checked_sub(0x10)?;
    let td_rva = (td_addr - module_base) as u32;

    // Scan .rdata for CompleteObjectLocator (COL):
    // +0x00: signature == 1 (x64)
    // +0x0C: pTypeDescriptor == td_rva
    // +0x14: pSelf == col_rva
    let rdata_slice = std::slice::from_raw_parts(rdata.base as *const u8, rdata.size);
    let mut col_addr = None;
    if rdata.size >= 0x18 {
        let mut off = 0usize;
        while off + 0x18 <= rdata.size {
            let sig = u32::from_le_bytes([
                rdata_slice[off],
                rdata_slice[off + 1],
                rdata_slice[off + 2],
                rdata_slice[off + 3],
            ]);
            let cand_td = u32::from_le_bytes([
                rdata_slice[off + 12],
                rdata_slice[off + 13],
                rdata_slice[off + 14],
                rdata_slice[off + 15],
            ]);
            if sig == 1 && cand_td == td_rva {
                let p_self = u32::from_le_bytes([
                    rdata_slice[off + 20],
                    rdata_slice[off + 21],
                    rdata_slice[off + 22],
                    rdata_slice[off + 23],
                ]);
                let cand_abs = rdata.base + off;
                if p_self == (cand_abs - module_base) as u32 {
                    col_addr = Some(cand_abs);
                    break;
                }
            }
            off += 4;
        }
    }

    let col = col_addr?;
    let col_bytes = (col as u64).to_le_bytes();

    // The vtable immediately follows the 8-byte pointer to COL in .rdata
    let mut off = 0usize;
    while off + 16 <= rdata.size {
        if &rdata_slice[off..off + 8] == &col_bytes {
            return Some(rdata.base + off + 8);
        }
        off += 8;
    }

    None
}

unsafe fn find_bytes_in_span(span: SectionSpan, needle: &[u8]) -> Option<usize> {
    if span.base == 0 || span.size < needle.len() {
        return None;
    }
    let hay = std::slice::from_raw_parts(span.base as *const u8, span.size);
    hay.windows(needle.len())
        .position(|w| w == needle)
        .map(|pos| span.base + pos)
}

unsafe fn find_pattern_in_span(span: SectionSpan, pat: &[Option<u8>]) -> Option<usize> {
    if span.base == 0 || span.size < pat.len() {
        return None;
    }
    let hay = std::slice::from_raw_parts(span.base as *const u8, span.size);
    'outer: for i in 0..=(span.size - pat.len()) {
        for (j, &b) in pat.iter().enumerate() {
            if let Some(expected) = b {
                if hay[i + j] != expected {
                    continue 'outer;
                }
            }
        }
        return Some(span.base + i);
    }
    None
}

unsafe fn resolve_serialize_to_array(text: SectionSpan) -> Option<usize> {
    // mov [rsp+18h], rbx; push rbp; push rsi; push rdi; sub rsp, ??; mov rax, [rip+??]; xor rax, rsp
    let pat: [Option<u8>; 25] = [
        Some(0x48),
        Some(0x89),
        Some(0x5C),
        Some(0x24),
        Some(0x18),
        Some(0x55),
        Some(0x56),
        Some(0x57),
        Some(0x48),
        Some(0x81),
        Some(0xEC),
        None,
        None,
        None,
        None,
        Some(0x48),
        Some(0x8B),
        Some(0x05),
        None,
        None,
        None,
        None,
        Some(0x48),
        Some(0x33),
        Some(0xC4),
    ];
    find_pattern_in_span(text, &pat)
}

unsafe fn resolve_parse_from_array(text: SectionSpan, serialize_addr: usize) -> Option<usize> {
    // mov [rsp+10h], rbx; mov [rsp+18h], rsi; push rbp; push rdi; push r14; lea rbp, [rsp+??]
    let pat: [Option<u8>; 19] = [
        Some(0x48),
        Some(0x89),
        Some(0x5C),
        Some(0x24),
        Some(0x10),
        Some(0x48),
        Some(0x89),
        Some(0x74),
        Some(0x24),
        Some(0x18),
        Some(0x55),
        Some(0x57),
        Some(0x41),
        Some(0x56),
        Some(0x48),
        Some(0x8D),
        Some(0x6C),
        Some(0x24),
        None,
    ];

    if serialize_addr > text.base + 0x1000 {
        let local_span = SectionSpan {
            base: serialize_addr - 0x1000,
            size: 0x1000,
        };
        if let Some(hit) = find_pattern_in_span(local_span, &pat) {
            return Some(hit);
        }
    }
    find_pattern_in_span(text, &pat)
}

