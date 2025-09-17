mod metadata {
    pub const ORDERBOOK_ELF: &[u8] = include_bytes!("../elf/orderbook");
}

pub use metadata::*;
