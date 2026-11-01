//! LEB128 varint encoding used by the `.ior` v2 binary format.
//!
//! Values are u32 written 7 bits per byte, least-significant group first;
//! every byte except the last sets the continuation bit (0x80). Decoding
//! is strict: at most five bytes, the fifth carrying four payload bits.

/// Append `value` to `out` as a LEB128 varint (1..=5 bytes).
pub fn write_u32(out: &mut Vec<u8>, value: u32) {
    let mut remaining = value;
    loop {
        let mut byte = (remaining & 0x7f) as u8;
        remaining >>= 7;
        if remaining != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if remaining == 0 {
            return;
        }
    }
}

/// Read one varint at `pos`; returns `(value, next_pos)`.
pub fn read_u32(bytes: &[u8], pos: usize) -> Result<(u32, usize), String> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut cursor = pos;
    loop {
        let Some(&byte) = bytes.get(cursor) else {
            return Err(format!(
                "truncated varint starting at offset {pos}: buffer is {} byte(s)",
                bytes.len()
            ));
        };
        cursor += 1;
        let payload = u32::from(byte & 0x7f);
        if shift >= 28 {
            // Fifth (final) group: only four payload bits fit into a u32.
            if byte > 0x0f {
                return Err(format!("varint at offset {pos} exceeds u32 range"));
            }
            return Ok((result | (payload << shift), cursor));
        }
        result |= payload << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Ok((result, cursor));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_boundary_values() {
        for value in [
            0u32,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            2_097_152,
            u32::MAX - 1,
            u32::MAX,
        ] {
            let mut buf = Vec::new();
            write_u32(&mut buf, value);
            let (decoded, next) = read_u32(&buf, 0).expect("decode");
            assert_eq!(decoded, value, "value {value}");
            assert_eq!(next, buf.len());
        }
    }

    #[test]
    fn encoded_sizes_match_leb128_spec() {
        let mut buf = Vec::new();
        write_u32(&mut buf, 0);
        assert_eq!(buf, vec![0x00]);
        buf.clear();
        write_u32(&mut buf, 300);
        assert_eq!(buf, vec![0xac, 0x02]);
        buf.clear();
        write_u32(&mut buf, u32::MAX);
        assert_eq!(buf.len(), 5);
    }
    #[test]
    fn rejects_overlong_and_truncated_varints() {
        // 6-byte varint: the fifth byte still carries the continuation bit.
        let overlong = [0xff, 0xff, 0xff, 0xff, 0x8f, 0x00];
        assert!(read_u32(&overlong, 0).unwrap_err().contains("exceeds u32"));
        // Payload bits that do not fit into a u32.
        let overflow = [0xff, 0xff, 0xff, 0xff, 0x7f];
        assert!(read_u32(&overflow, 0).unwrap_err().contains("exceeds u32"));
        assert!(read_u32(&[0x80], 0).unwrap_err().contains("truncated"));
    }
}
