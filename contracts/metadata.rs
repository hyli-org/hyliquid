mod metadata {
    pub const ORDERBOOK_ELF: &[u8] = include_bytes!("../elf/orderbook");
    pub const ORDERBOOK_VK: &[u8] = include_bytes!("../elf/orderbook_vk");
}

pub use metadata::*;
