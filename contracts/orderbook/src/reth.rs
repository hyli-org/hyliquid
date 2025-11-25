use k256::ecdsa::SigningKey;
use sdk::{ContractName, ProgramId};
use sha3::{Digest, Keccak256};

pub fn derive_program_pubkey(contract_name: &ContractName) -> ProgramId {
    let mut seed = keccak(contract_name.0.as_bytes());
    loop {
        let field_bytes = k256::FieldBytes::from(seed);
        if let Ok(key) = SigningKey::from_bytes(&field_bytes) {
            let encoded = key
                .verifying_key()
                .to_encoded_point(false)
                .as_bytes()
                .to_vec();
            return ProgramId(encoded);
        }
        seed = keccak(&seed);
    }
}

pub fn program_address_from_program_id(program_id: &ProgramId) -> [u8; 20] {
    let hash = keccak(&program_id.0);
    let mut address = [0u8; 20];
    address.copy_from_slice(&hash[12..]);
    address
}

pub fn extract_erc20_recipient(tx_bytes: &[u8]) -> Result<[u8; 20], String> {
    if tx_bytes.first() != Some(&0x02) {
        return Err("Deposit blob must be a typed EIP-1559 transaction".into());
    }
    let payload = &tx_bytes[1..];
    let (item, _) = rlp::decode_item(payload)
        .map_err(|err| format!("Invalid typed transaction payload: {err}"))?;
    let list_payload = match item {
        rlp::RlpItem::List(body) => body,
        rlp::RlpItem::Bytes(_) => {
            return Err("Typed transaction payload is not an RLP list".into());
        }
    };
    let data_item = rlp::nth_list_item(list_payload, 7)
        .map_err(|_| "Typed transaction missing data field".to_string())?;
    let data_bytes = match data_item {
        rlp::RlpItem::Bytes(bytes) => bytes,
        rlp::RlpItem::List(_) => return Err("Transaction data field must be bytes".into()),
    };
    if data_bytes.len() < 4 + 32 + 32 {
        return Err("ERC20 calldata too short".into());
    }
    if data_bytes[..4] != ERC20_TRANSFER_SELECTOR {
        return Err("First blob is not an ERC20 transfer".into());
    }
    let mut address = [0u8; 20];
    address.copy_from_slice(&data_bytes[4 + 12..4 + 32]);
    Ok(address)
}

const ERC20_TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];

fn keccak(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

mod rlp {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RlpItem<'a> {
        Bytes(&'a [u8]),
        List(&'a [u8]),
    }

    #[derive(Debug)]
    pub enum RlpError {
        InputTooShort,
        InvalidLength,
    }

    impl core::fmt::Display for RlpError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            match self {
                RlpError::InputTooShort => write!(f, "rlp input too short"),
                RlpError::InvalidLength => write!(f, "rlp length invalid"),
            }
        }
    }

    pub fn decode_item(input: &[u8]) -> Result<(RlpItem<'_>, usize), RlpError> {
        if input.is_empty() {
            return Err(RlpError::InputTooShort);
        }
        let prefix = input[0];
        match prefix {
            0x00..=0x7f => Ok((RlpItem::Bytes(&input[..1]), 1)),
            0x80..=0xb7 => {
                let len = (prefix - 0x80) as usize;
                let end = 1 + len;
                if input.len() < end {
                    return Err(RlpError::InputTooShort);
                }
                Ok((RlpItem::Bytes(&input[1..end]), end))
            }
            0xb8..=0xbf => {
                let len_of_len = (prefix - 0xb7) as usize;
                if input.len() < 1 + len_of_len {
                    return Err(RlpError::InputTooShort);
                }
                let len = parse_length(&input[1..1 + len_of_len])?;
                let end = 1 + len_of_len + len;
                if input.len() < end {
                    return Err(RlpError::InputTooShort);
                }
                Ok((RlpItem::Bytes(&input[1 + len_of_len..end]), end))
            }
            0xc0..=0xf7 => {
                let len = (prefix - 0xc0) as usize;
                let end = 1 + len;
                if input.len() < end {
                    return Err(RlpError::InputTooShort);
                }
                Ok((RlpItem::List(&input[1..end]), end))
            }
            0xf8..=0xff => {
                let len_of_len = (prefix - 0xf7) as usize;
                if input.len() < 1 + len_of_len {
                    return Err(RlpError::InputTooShort);
                }
                let len = parse_length(&input[1..1 + len_of_len])?;
                let end = 1 + len_of_len + len;
                if input.len() < end {
                    return Err(RlpError::InputTooShort);
                }
                Ok((RlpItem::List(&input[1 + len_of_len..end]), end))
            }
        }
    }

    pub fn nth_list_item<'a>(
        list_payload: &'a [u8],
        target_index: usize,
    ) -> Result<RlpItem<'a>, RlpError> {
        let mut cursor = list_payload;
        let mut idx = 0usize;
        while !cursor.is_empty() {
            let (item, consumed) = decode_item(cursor)?;
            if idx == target_index {
                return Ok(item);
            }
            cursor = &cursor[consumed..];
            idx += 1;
        }
        Err(RlpError::InvalidLength)
    }

    fn parse_length(bytes: &[u8]) -> Result<usize, RlpError> {
        if bytes.is_empty() {
            return Err(RlpError::InvalidLength);
        }
        let mut len: usize = 0;
        for &b in bytes {
            len = (len << 8) | (b as usize);
        }
        Ok(len)
    }

    #[cfg(test)]
    pub fn encode_bytes(data: &[u8]) -> Vec<u8> {
        match data.len() {
            0 => vec![0x80],
            1 if data[0] <= 0x7f => vec![data[0]],
            len @ 1..=55 => {
                let mut out = Vec::with_capacity(1 + len);
                out.push(0x80 + len as u8);
                out.extend_from_slice(data);
                out
            }
            len => {
                let len_bytes = length_bytes(len);
                let mut out = Vec::with_capacity(1 + len_bytes.len() + len);
                out.push(0xb7 + len_bytes.len() as u8);
                out.extend_from_slice(&len_bytes);
                out.extend_from_slice(data);
                out
            }
        }
    }

    #[cfg(test)]
    pub fn encode_u64(value: u64) -> Vec<u8> {
        if value == 0 {
            return vec![0x80];
        }
        let mut buf = Vec::new();
        let mut v = value;
        while v > 0 {
            buf.push((v & 0xff) as u8);
            v >>= 8;
        }
        buf.reverse();
        encode_bytes(&buf)
    }

    #[cfg(test)]
    pub fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
        let payload_len: usize = items.iter().map(|item| item.len()).sum();
        let mut payload = Vec::with_capacity(payload_len);
        for item in items {
            payload.extend_from_slice(item);
        }
        match payload_len {
            len @ 0..=55 => {
                let mut out = Vec::with_capacity(1 + len);
                out.push(0xc0 + len as u8);
                out.extend_from_slice(&payload);
                out
            }
            len => {
                let len_bytes = length_bytes(len);
                let mut out = Vec::with_capacity(1 + len_bytes.len() + len);
                out.push(0xf7 + len_bytes.len() as u8);
                out.extend_from_slice(&len_bytes);
                out.extend_from_slice(&payload);
                out
            }
        }
    }

    #[cfg(test)]
    fn length_bytes(len: usize) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut value = len;
        while value > 0 {
            buf.push((value & 0xff) as u8);
            value >>= 8;
        }
        buf.reverse();
        buf
    }
}

#[cfg(test)]
pub fn build_typed_eip1559_tx(data: Vec<u8>, to: [u8; 20]) -> Vec<u8> {
    use rlp::{encode_bytes, encode_list, encode_u64};
    let mut to_bytes = [0u8; 20];
    to_bytes.copy_from_slice(&to);
    let tx_fields = vec![
        encode_u64(1),           // chain id
        encode_u64(0),           // nonce
        encode_u64(1),           // max priority fee
        encode_u64(1),           // max fee
        encode_u64(21000),       // gas limit
        encode_bytes(&to_bytes), // to
        encode_u64(0),           // value
        encode_bytes(&data),     // data
        encode_list(&[]),        // access list
        encode_u64(0),           // y parity
        encode_bytes(&[0]),      // r
        encode_bytes(&[0]),      // s
    ];
    let mut out = vec![0x02];
    out.extend_from_slice(&encode_list(&tx_fields));
    out
}

#[cfg(test)]
pub fn encode_erc20_transfer_payload(recipient: [u8; 20], amount: u128) -> Vec<u8> {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&ERC20_TRANSFER_SELECTOR);
    let mut recipient_padded = [0u8; 32];
    recipient_padded[12..].copy_from_slice(&recipient);
    data.extend_from_slice(&recipient_padded);
    let mut amount_padded = [0u8; 32];
    amount_padded[16..].copy_from_slice(&amount.to_be_bytes());
    data.extend_from_slice(&amount_padded);
    data
}
