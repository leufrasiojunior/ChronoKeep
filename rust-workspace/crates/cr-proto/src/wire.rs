//! Clean-room Protobuf wire format reader and writer (Proto2/Proto3 compatible).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireValue<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField<'a> {
    pub number: u32,
    pub value: WireValue<'a>,
}

impl<'a> ProtoField<'a> {
    pub fn as_u64(&self) -> Option<u64> {
        match self.value {
            WireValue::Varint(v) | WireValue::Fixed64(v) => Some(v),
            WireValue::Fixed32(v) => Some(v as u64),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        self.as_u64().map(|v| v as u32)
    }

    pub fn as_bool(&self) -> Option<bool> {
        self.as_u64().map(|v| v != 0)
    }

    pub fn as_bytes(&self) -> Option<&'a [u8]> {
        match self.value {
            WireValue::Bytes(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'a str> {
        self.as_bytes().and_then(|b| std::str::from_utf8(b).ok())
    }
}

pub fn decode_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while *pos < buf.len() {
        let byte = buf[*pos];
        *pos += 1;
        result |= ((byte & 0x7F) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Some(result);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

pub fn parse_message<'a>(buf: &'a [u8]) -> Vec<ProtoField<'a>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < buf.len() {
        let Some(tag) = decode_varint(buf, &mut pos) else {
            break;
        };
        let number = (tag >> 3) as u32;
        let wire_type = (tag & 0x07) as u8;
        if number == 0 {
            break;
        }
        let value = match wire_type {
            0 => {
                let Some(v) = decode_varint(buf, &mut pos) else {
                    break;
                };
                WireValue::Varint(v)
            }
            1 => {
                if pos + 8 > buf.len() {
                    break;
                }
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&buf[pos..pos + 8]);
                pos += 8;
                WireValue::Fixed64(u64::from_le_bytes(arr))
            }
            2 => {
                let Some(len) = decode_varint(buf, &mut pos) else {
                    break;
                };
                let len = len as usize;
                let slice = if pos + len > buf.len() {
                    let s = &buf[pos..];
                    pos = buf.len();
                    s
                } else {
                    let s = &buf[pos..pos + len];
                    pos += len;
                    s
                };
                WireValue::Bytes(slice)
            }
            5 => {
                if pos + 4 > buf.len() {
                    break;
                }
                let mut arr = [0u8; 4];
                arr.copy_from_slice(&buf[pos..pos + 4]);
                pos += 4;
                WireValue::Fixed32(u32::from_le_bytes(arr))
            }
            _ => break,
        };
        out.push(ProtoField { number, value });
    }
    out
}

pub fn find_field<'a, 'b>(fields: &'b [ProtoField<'a>], number: u32) -> Option<&'b ProtoField<'a>> {
    fields.iter().find(|f| f.number == number)
}

#[derive(Debug, Default, Clone)]
pub struct ProtoWriter {
    buf: Vec<u8>,
}

impl ProtoWriter {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    fn push_varint_raw(&mut self, mut val: u64) {
        loop {
            let mut byte = (val & 0x7F) as u8;
            val >>= 7;
            if val != 0 {
                byte |= 0x80;
            }
            self.buf.push(byte);
            if val == 0 {
                break;
            }
        }
    }

    fn push_tag(&mut self, field_number: u32, wire_type: u8) {
        let tag = ((field_number as u64) << 3) | (wire_type as u64);
        self.push_varint_raw(tag);
    }

    pub fn write_varint(&mut self, field_number: u32, val: u64) -> &mut Self {
        self.push_tag(field_number, 0);
        self.push_varint_raw(val);
        self
    }

    pub fn write_fixed64(&mut self, field_number: u32, val: u64) -> &mut Self {
        self.push_tag(field_number, 1);
        self.buf.extend_from_slice(&val.to_le_bytes());
        self
    }

    pub fn write_fixed32(&mut self, field_number: u32, val: u32) -> &mut Self {
        self.push_tag(field_number, 5);
        self.buf.extend_from_slice(&val.to_le_bytes());
        self
    }

    pub fn write_bytes(&mut self, field_number: u32, bytes: &[u8]) -> &mut Self {
        self.push_tag(field_number, 2);
        self.push_varint_raw(bytes.len() as u64);
        self.buf.extend_from_slice(bytes);
        self
    }

    pub fn write_str(&mut self, field_number: u32, s: &str) -> &mut Self {
        self.write_bytes(field_number, s.as_bytes())
    }

    pub fn write_submessage(&mut self, field_number: u32, sub: &ProtoWriter) -> &mut Self {
        self.write_bytes(field_number, sub.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_truncated_length_delimited_field() {
        let mut writer = ProtoWriter::new();
        writer.write_varint(1, 2868840);
        writer.write_str(2, "profile1/saves/history/1793546464.run");
        let full = writer.into_bytes();

        // Simulate truncation by taking fewer bytes than the full message
        let truncated = &full[..full.len() - 5];
        let fields = parse_message(truncated);
        let f2 = find_field(&fields, 2);
        assert!(f2.is_some(), "Field 2 must be preserved despite truncation");
        let str_val = f2.unwrap().as_str();
        assert!(str_val.is_some(), "Must parse as valid string prefix");
        assert!(
            str_val.unwrap().starts_with("profile1/saves/history/"),
            "String must preserve prefix"
        );
    }
}


