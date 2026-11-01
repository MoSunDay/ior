//! Integration tests for the engine-core `.ior` reader, using hand-built
//! v1/v2 byte buffers (engine-core owns no dict-tools dependency; the two
//! crates implement the same format independently and these bytes encode
//! the spec directly).

use engine_core::dict::Dictionary;

/// LEB128 u32 varint (spec: 7 bits per byte, high bit = continuation).
fn push_varint(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

/// Hand-build a v2 buffer from `(key, [(hanzi, freq)])` groups.
fn v2_bytes(groups: &[(&str, &[(&str, u32)])]) -> Vec<u8> {
    let total: usize = groups.iter().map(|(_, vals)| vals.len()).sum();
    let mut out = Vec::new();
    out.extend_from_slice(b"IORD");
    out.push(2u8);
    out.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
    out.extend_from_slice(&u32::try_from(groups.len()).unwrap().to_le_bytes());
    for &(key, vals) in groups {
        push_varint(&mut out, u32::try_from(key.len()).unwrap());
        out.extend_from_slice(key.as_bytes());
        push_varint(&mut out, u32::try_from(vals.len()).unwrap());
    }
    for &(_, vals) in groups {
        for &(hanzi, freq) in vals {
            push_varint(&mut out, u32::try_from(hanzi.len()).unwrap());
            out.extend_from_slice(hanzi.as_bytes());
            push_varint(&mut out, freq);
        }
    }
    out
}

/// Hand-build a legacy v1 buffer from flat `(pinyin, hanzi, freq)` triples.
fn v1_bytes(entries: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"IORD");
    out.push(1u8);
    out.extend_from_slice(&u32::try_from(entries.len()).unwrap().to_le_bytes());
    for &(pinyin, hanzi, freq) in entries {
        for field in [pinyin, hanzi] {
            out.extend_from_slice(&u32::try_from(field.len()).unwrap().to_le_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out.extend_from_slice(&freq.to_le_bytes());
    }
    out
}

#[test]
fn v2_header_and_multi_byte_varints_parse() {
    // 180-byte hanzi forces a two-byte length varint (> 127).
    let long_word: String = "词".repeat(60);
    assert_eq!(long_word.len(), 180);
    let bytes = v2_bytes(&[
        ("a", &[("短", 1)]),
        ("chang'de'ci'shu", &[(long_word.as_str(), 90_000)]),
    ]);
    assert_eq!(&bytes[..4], b"IORD");
    assert_eq!(bytes[4], 2);
    assert_eq!(&bytes[5..9], &2u32.to_le_bytes()); // entry_count
    assert_eq!(&bytes[9..13], &2u32.to_le_bytes()); // key_count
                                                    // multi-byte freq varint decodes
    let d = Dictionary::from_ior_bytes(&bytes).unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!(
        d.lookup("chang'de'ci'shu").unwrap(),
        [(long_word.clone(), 90_000)]
    );
}

#[test]
fn v1_buffer_still_loads() {
    let bytes = v1_bytes(&[
        ("ni'hao", "你好", 100),
        ("ni'hao", "你好", 50), // duplicate pair sums
        ("xi'an", "西安", 40),
    ]);
    let d = Dictionary::from_ior_bytes(&bytes).unwrap();
    assert_eq!(d.lookup("ni'hao").unwrap(), [("你好".to_string(), 150)]);
    assert_eq!(d.lookup("xi'an").unwrap()[0].1, 40);
}

#[test]
fn corrupt_buffers_fail_with_specific_errors() {
    let good = v2_bytes(&[("ni", &[("你", 9)])]);

    let mut bad_magic = good.clone();
    bad_magic[0] = b'Q';
    assert!(Dictionary::from_ior_bytes(&bad_magic)
        .unwrap_err()
        .contains("magic"));

    let mut bad_version = good.clone();
    bad_version[4] = 3;
    assert!(Dictionary::from_ior_bytes(&bad_version)
        .unwrap_err()
        .contains("unsupported version"));

    for cut in 0..good.len() {
        assert!(
            Dictionary::from_ior_bytes(&good[..cut]).is_err(),
            "cut at {cut} must fail"
        );
    }

    let mut trailing = good.clone();
    trailing.push(0);
    assert!(Dictionary::from_ior_bytes(&trailing)
        .unwrap_err()
        .contains("trailing"));

    // a length field that runs past the buffer end
    let mut overrun = good.clone();
    let value_len_pos = 13 + 3 + 1; // header + key record + hanzi len byte
    overrun[value_len_pos] = 0x7f; // claims 127-byte hanzi
    assert!(Dictionary::from_ior_bytes(&overrun)
        .unwrap_err()
        .contains("truncated"));

    // 6-byte varint (continuation bit set on the 5th byte)
    let mut long_varint = good.clone();
    let values_len_pos = 16; // header(13) + key len(1) + "ni"(2)
    long_varint[values_len_pos..values_len_pos + 5]
        .copy_from_slice(&[0x81, 0x80, 0x80, 0x80, 0x80]);
    assert!(Dictionary::from_ior_bytes(&long_varint)
        .unwrap_err()
        .contains("u32 range"));
}

#[test]
fn layers_overlay_base_domain_user_with_freq_summing() {
    let base = v2_bytes(&[("kai'fa", &[("开发", 500)]), ("fu'wu", &[("服务", 80)])]);
    let domain = v2_bytes(&[("kai'fa", &[("开发", 20), ("开伐", 3)])]);
    let user = v2_bytes(&[("kai'fa", &[("开发", 7)])]);
    let d = Dictionary::from_ior_layers(&[&base, &domain, &user]).unwrap();
    let kai = d.lookup("kai'fa").unwrap();
    assert_eq!(kai[0], ("开发".to_string(), 527)); // 500 + 20 + 7
    assert_eq!(kai[1], ("开伐".to_string(), 3));
    assert_eq!(d.lookup("fu'wu").unwrap()[0].1, 80);
    assert_eq!(d.len(), 2);

    // empty layer list: empty dictionary, not an error
    assert!(Dictionary::from_ior_layers(&[]).unwrap().is_empty());
    // a corrupt layer aborts the whole load
    let corrupt = vec![base.as_slice(), b"IORD\x02corrupt".as_slice()];
    assert!(Dictionary::from_ior_layers(&corrupt).is_err());
}

#[test]
fn canonical_keys_match_the_tsv_path_without_fuzzy() {
    // P1: `.ior` keys stay canonical full pinyin (fuzzy expansion moved to
    // the query side), so bytes and TSV text agree key for key and "zhang"
    // is reachable under its own spelling only.
    let bytes = v2_bytes(&[("zhang", &[("张", 9)]), ("san", &[("三", 4)])]);
    let from_ior = Dictionary::from_ior_bytes(&bytes).unwrap();
    let from_tsv = Dictionary::from_tsv("zhang\t张\t9\nsan\t三\t4\n").unwrap();
    assert_eq!(from_ior.len(), from_tsv.len());
    assert_eq!(from_ior.len(), 2, "zhang and san stay distinct keys");
    assert_eq!(
        from_ior.lookup("zhang").unwrap(),
        from_tsv.lookup("zhang").unwrap()
    );
    assert_eq!(from_ior.lookup("zhang").unwrap()[0].0, "张");
    assert!(from_ior.lookup("zan").is_none(), "no fuzzy collapse");
}
