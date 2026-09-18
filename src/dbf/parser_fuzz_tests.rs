use super::DbfTable;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[test]
fn parser_does_not_panic_on_deterministic_binary_inputs() {
    let mut state = 0x9e37_79b9_u32;

    for length in 0..=512 {
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            bytes.push((state >> 24) as u8);
        }

        let result = catch_unwind(AssertUnwindSafe(|| DbfTable::from_bytes(&bytes)));
        assert!(
            result.is_ok(),
            "DBF parser panicked for {length} random bytes"
        );
    }
}
