use std::ops::Range;

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    // ponytail: keep the dependency-free bitwise checksum; use a table only if profiling proves it matters.
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0x82f6_3b78
            };
        }
    }
    !crc
}

pub(crate) fn crc32c_with_zeroed_range(bytes: &[u8], zeroed: Range<usize>) -> u32 {
    let mut crc = u32::MAX;
    for (index, byte) in bytes.iter().enumerate() {
        crc ^= u32::from(if zeroed.contains(&index) { 0 } else { *byte });
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0x82f6_3b78
            };
        }
    }
    !crc
}
