pub(super) fn crc32c(bytes: &[u8]) -> u32 {
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
