//! Panel dimensions are native, unrotated millimetres, independent of desktop scaling.

pub(crate) fn registry_path(device_path: &str) -> Option<String> {
    let mut parts = device_path.strip_prefix("\\\\?\\")?.split('#');
    if !parts.next()?.eq_ignore_ascii_case("display") {
        return None;
    }
    let model = parts.next()?;
    let instance = parts.next()?;
    let class = parts.next()?;
    if parts.next().is_some()
        || !class.starts_with('{')
        || !class.ends_with('}')
        || [model, instance].iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'&' || b == b'_')
        })
    {
        return None;
    }
    Some(format!(
        "SYSTEM\\CurrentControlSet\\Enum\\DISPLAY\\{model}\\{instance}\\Device Parameters"
    ))
}

pub(crate) fn panel_size_mm(edid: &[u8]) -> Option<(f64, f64)> {
    let block = edid.get(..128)?;
    if block[..8] != [0, 255, 255, 255, 255, 255, 255, 0]
        || block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) != 0
        || block[18] != 1
    {
        return None;
    }
    // Detailed timing dimensions have millimetre precision; basic dimensions
    // are centimetres. Zero-clock descriptors are text/range data, not timings.
    let (timings, _) = block[54..126].as_chunks::<18>();
    for timing in timings {
        if timing[0] == 0 && timing[1] == 0 {
            continue;
        }
        let width = u16::from(timing[12]) | (u16::from(timing[14] & 0xf0) << 4);
        let height = u16::from(timing[13]) | (u16::from(timing[14] & 0x0f) << 8);
        if width > 1 && height > 1 {
            return Some((f64::from(width), f64::from(height)));
        }
    }
    // EDID 1.4 uses a single nonzero dimension to encode aspect ratio.
    (block[21] > 0 && block[22] > 0)
        .then(|| (f64::from(block[21]) * 10.0, f64::from(block[22]) * 10.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checksum(edid: &mut [u8; 128]) {
        edid[127] = 0;
        edid[127] = 0u8.wrapping_sub(edid.iter().fold(0u8, |s, b| s.wrapping_add(*b)));
    }

    #[test]
    fn detailed_size_and_basic_fallback() {
        let mut edid = [0; 128];
        edid[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        edid[18] = 1;
        edid[21] = 60;
        edid[22] = 34;
        checksum(&mut edid);
        assert_eq!(panel_size_mm(&edid), Some((600.0, 340.0)));
        edid[54] = 1;
        edid[66] = 85;
        edid[67] = 79;
        edid[68] = 0x21;
        checksum(&mut edid);
        assert_eq!(panel_size_mm(&edid), Some((597.0, 335.0)));
        edid[127] ^= 1;
        assert_eq!(panel_size_mm(&edid), None);
        assert_eq!(panel_size_mm(&edid[..64]), None);
    }

    #[test]
    fn missing_dimensions_and_aspect_ratio_are_unknown() {
        let mut edid = [0; 128];
        edid[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
        edid[18] = 1;
        edid[21] = 160;
        checksum(&mut edid);
        assert_eq!(panel_size_mm(&edid), None);
    }

    #[test]
    fn registry_lookup_is_scoped_to_selected_monitor() {
        assert_eq!(
            registry_path(r"\\?\display#del1234#5&abc&0&uid1#{guid}"),
            Some(
                r"SYSTEM\CurrentControlSet\Enum\DISPLAY\del1234\5&abc&0&uid1\Device Parameters"
                    .into()
            )
        );
        assert!(registry_path(r"\\?\display#..\other#instance#{guid}").is_none());
        assert!(registry_path(r"\\?\other#model#instance#{guid}").is_none());
    }
}
