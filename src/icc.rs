//! PDF/A OutputIntent 용 sRGB ICC v2 프로파일을 코드로 만든다.
//! 외부 .icc 파일을 배포하지 않아도 되고 라이선스 문제도 없다.
//! (matrix/TRC 방식: D50 적응 원색 + sRGB 전달 곡선 1024 점 테이블)

fn s15f16(v: f64) -> [u8; 4] {
    ((v * 65536.0).round() as i32).to_be_bytes()
}

fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
    let mut t = b"XYZ \0\0\0\0".to_vec();
    t.extend_from_slice(&s15f16(x));
    t.extend_from_slice(&s15f16(y));
    t.extend_from_slice(&s15f16(z));
    t
}

fn curv_tag() -> Vec<u8> {
    const N: usize = 1024;
    let mut t = b"curv\0\0\0\0".to_vec();
    t.extend_from_slice(&(N as u32).to_be_bytes());
    for i in 0..N {
        let v = i as f64 / (N - 1) as f64;
        let lin = if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        t.extend_from_slice(&((lin * 65535.0).round() as u16).to_be_bytes());
    }
    t
}

fn desc_tag(text: &str) -> Vec<u8> {
    let mut t = b"desc\0\0\0\0".to_vec();
    t.extend_from_slice(&(text.len() as u32 + 1).to_be_bytes());
    t.extend_from_slice(text.as_bytes());
    t.push(0);
    t.extend_from_slice(&[0; 4]); // unicode language code
    t.extend_from_slice(&[0; 4]); // unicode count
    t.extend_from_slice(&[0; 2]); // scriptcode code
    t.push(0); // scriptcode count
    t.extend_from_slice(&[0; 67]);
    t
}

fn text_tag(text: &str) -> Vec<u8> {
    let mut t = b"text\0\0\0\0".to_vec();
    t.extend_from_slice(text.as_bytes());
    t.push(0);
    t
}

pub fn srgb_profile() -> Vec<u8> {
    let trc = curv_tag();
    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"desc", desc_tag("sRGB IEC61966-2.1")),
        (b"cprt", text_tag("No copyright, use freely")),
        (b"wtpt", xyz_tag(0.9642, 1.0, 0.8249)),
        (b"rXYZ", xyz_tag(0.4361, 0.2225, 0.0139)),
        (b"gXYZ", xyz_tag(0.3851, 0.7169, 0.0971)),
        (b"bXYZ", xyz_tag(0.1431, 0.0606, 0.7141)),
        (b"rTRC", trc.clone()),
        (b"gTRC", trc.clone()),
        (b"bTRC", trc),
    ];

    let table_len = 4 + tags.len() * 12;
    let mut offset = 128 + table_len;
    let mut table = (tags.len() as u32).to_be_bytes().to_vec();
    let mut data = Vec::new();
    for (sig, body) in &tags {
        table.extend_from_slice(*sig);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
        offset += body.len();
        while data.len() % 4 != 0 {
            data.push(0);
            offset += 1;
        }
    }

    let size = 128 + table.len() + data.len();
    let mut h = Vec::with_capacity(size);
    h.extend_from_slice(&(size as u32).to_be_bytes());
    h.extend_from_slice(&[0; 4]); // CMM
    h.extend_from_slice(&[0x02, 0x10, 0, 0]); // version 2.1
    h.extend_from_slice(b"mntr");
    h.extend_from_slice(b"RGB ");
    h.extend_from_slice(b"XYZ ");
    for v in [2026u16, 1, 1, 0, 0, 0] {
        h.extend_from_slice(&v.to_be_bytes());
    }
    h.extend_from_slice(b"acsp");
    h.extend_from_slice(&[0; 4]); // platform
    h.extend_from_slice(&[0; 4]); // flags
    h.extend_from_slice(&[0; 4]); // manufacturer
    h.extend_from_slice(&[0; 4]); // model
    h.extend_from_slice(&[0; 8]); // attributes
    h.extend_from_slice(&[0; 4]); // rendering intent: perceptual
    h.extend_from_slice(&s15f16(0.9642));
    h.extend_from_slice(&s15f16(1.0));
    h.extend_from_slice(&s15f16(0.8249));
    h.extend_from_slice(&[0; 4]); // creator
    h.resize(128, 0);
    h.extend_from_slice(&table);
    h.extend_from_slice(&data);
    debug_assert_eq!(h.len(), size);
    h
}
