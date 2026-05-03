//! Env-gated binary dump helpers for layer-diff debugging.
//!
//! All functions are no-ops unless LEWTOFF_DEBUG_DUMP=1 is set at runtime.
//! Each "first short block" dump fires exactly once per process, guarded by
//! a OnceLock<()>.

use std::sync::OnceLock;

static DUMP_ENABLED: OnceLock<bool> = OnceLock::new();
static SHORT_BLOCK_DUMPED: OnceLock<()> = OnceLock::new();
static MAPPING0_DUMPED: OnceLock<()> = OnceLock::new();

pub(crate) fn dump_enabled() -> bool {
    *DUMP_ENABLED.get_or_init(|| std::env::var("LEWTOFF_DEBUG_DUMP").is_ok())
}

pub(crate) fn try_claim_first_short_block() -> bool {
    SHORT_BLOCK_DUMPED.set(()).is_ok()
}

pub(crate) fn try_claim_mapping0_dump() -> bool {
    MAPPING0_DUMPED.set(()).is_ok()
}

pub(crate) fn write_f32_bin(path: &str, data: &[f32]) {
    use std::io::Write;
    let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_le_bytes()).collect();
    let mut f = std::fs::File::create(path).expect("debug dump: create file");
    f.write_all(&bytes).expect("debug dump: write f32 bin");
}

pub(crate) fn write_i32_bin(path: &str, data: &[i32]) {
    use std::io::Write;
    let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_le_bytes()).collect();
    let mut f = std::fs::File::create(path).expect("debug dump: create file");
    f.write_all(&bytes).expect("debug dump: write i32 bin");
}

pub(crate) fn write_txt(path: &str, content: &str) {
    use std::io::Write;
    let mut f = std::fs::File::create(path).expect("debug dump: create txt");
    f.write_all(content.as_bytes())
        .expect("debug dump: write txt");
}
