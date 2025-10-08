use borsh::{BorshDeserialize, BorshSerialize};
use monotree::Proof;

use monotree::{hasher::Sha2, Hash as MonotreeHash, Hasher};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BorshableMonotreeProof(pub Proof);

impl BorshSerialize for BorshableMonotreeProof {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        let len: u32 = self.0.len().try_into().map_err(|_| {
            borsh::io::Error::new(
                borsh::io::ErrorKind::InvalidInput,
                "Proof too large to serialize",
            )
        })?;
        len.serialize(writer)?;
        for (is_right, bytes) in &self.0 {
            is_right.serialize(writer)?;
            bytes.serialize(writer)?;
        }
        Ok(())
    }
}

pub fn compute_root_from_proof_with_hasher<H: Hasher>(
    hasher: &H,
    leaf: &MonotreeHash,
    proof: &Proof,
) -> MonotreeHash {
    let mut hash = *leaf;
    for (is_right, path) in proof.iter().rev() {
        if *is_right {
            let l = path.len();
            let combined = [&path[..l - 1], &hash[..], &path[l - 1..]].concat();
            hash = hasher.digest(&combined);
        } else {
            let combined = [&hash[..], &path[..]].concat();
            hash = hasher.digest(&combined);
        }
    }
    hash
}

pub fn compute_root_from_proof(leaf: &MonotreeHash, proof: &Proof) -> MonotreeHash {
    compute_root_from_proof_with_hasher(&Sha2::new(), leaf, proof)
}

impl BorshDeserialize for BorshableMonotreeProof {
    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        let len = u32::deserialize_reader(reader)?;
        let mut proof = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let is_right = bool::deserialize_reader(reader)?;
            let bytes = Vec::<u8>::deserialize_reader(reader)?;
            proof.push((is_right, bytes));
        }
        Ok(Self(proof))
    }
}
