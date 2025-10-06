use borsh::{BorshDeserialize, BorshSerialize};
use monotree::{Errors, Hash, Monotree, Proof, Result};
use std::collections::{hash_map::Entry, HashMap};

/// Represents a deduplicated proof node reused across multiple paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize)]
pub struct ProofNode {
    right: bool,
    cut: Vec<u8>,
}

impl ProofNode {
    fn as_tuple(&self) -> (bool, Vec<u8>) {
        (self.right, self.cut.clone())
    }
}

/// Describes how a key is represented inside the aggregated proof.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct MultiProofEntry {
    pub key: Hash,
    path: Option<Vec<u32>>,
}

impl MultiProofEntry {
    fn new(key: Hash, path: Option<Vec<u32>>) -> Self {
        MultiProofEntry { key, path }
    }
}

/// Result of looking up a key inside a [`MonotreeMultiProof`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofStatus {
    /// The key was part of the aggregated request and an inclusion proof is available.
    Present(Proof),
    /// The key was part of the aggregated request but no inclusion proof exists (non-inclusion).
    Absent,
}

/// Aggregated Merkle proof for multiple leaves of a [`monotree::Monotree`].
///
/// Sketch of how the compression works:
/// ```text
/// Inserted leaves (after sorting by key):   [k0, k1, k2]
/// Individual proofs (siblings top-to-bottom):
///   k0 -> [a, b, c]
///   k1 -> [a, d]
///   k2 -> [e, f, c]
///
/// build() walks the proofs in key order and writes siblings into a shared
/// `nodes` array while the per-leaf `path` just stores indices into that array:
///   nodes = [a, b, c, d, e, f]
///   path(k0) = [0, 1, 2]
///   path(k1) = [0, 3]
///   path(k2) = [4, 5, 2]
///
/// At verification time we replay the paths, resolving indices back into
/// concrete `(direction, value)` tuples. Any leaf that shares a sibling with
/// another leaf points to the same node entry, so the proof is shorter than
/// the naive concatenation of individual paths.
/// ```
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct MonotreeMultiProof {
    nodes: Vec<ProofNode>,
    entries: Vec<MultiProofEntry>,
}

impl MonotreeMultiProof {
    /// Builds a multi-proof by collecting individual proofs and deduplicating shared nodes.
    pub fn build<D, H, K>(
        tree: &mut Monotree<D, H>,
        root: Option<&Hash>,
        keys: K,
    ) -> Result<Self>
    where
        D: monotree::Database,
        H: monotree::Hasher,
        K: IntoIterator<Item = Hash>,
    {
        let mut nodes: Vec<ProofNode> = Vec::new();
        let mut node_indexes: HashMap<ProofNode, usize> = HashMap::new();
        let keys_iter = keys.into_iter();
        let (_, upper_bound) = keys_iter.size_hint();
        let mut entries: Vec<MultiProofEntry> = Vec::with_capacity(upper_bound.unwrap_or(0));

        // Collect each per-leaf proof in key order, reusing siblings that have already been emitted.
        for key in keys_iter {
            match tree.get_merkle_proof(root, &key)? {
                None => entries.push(MultiProofEntry::new(key, None)),
                Some(proof) => {
                    let mut path: Vec<u32> = Vec::with_capacity(proof.len());
                    for (right, cut) in proof {
                        let node = ProofNode { right, cut };
                        let idx = match node_indexes.entry(node.clone()) {
                            Entry::Occupied(entry) => *entry.get(),
                            Entry::Vacant(entry) => {
                                let position = nodes.len();
                                nodes.push(node);
                                entry.insert(position);
                                position
                            }
                        };
                        let index = u32::try_from(idx)
                            .map_err(|_| Errors::new("multi proof node index overflow"))?;
                        path.push(index);
                    }
                    entries.push(MultiProofEntry::new(key, Some(path)));
                }
            }
        }

        Ok(MonotreeMultiProof { nodes, entries })
    }

    /// Returns every key that was part of this aggregated proof alongside its status.
    pub fn entries(&self) -> impl Iterator<Item = (&Hash, ProofStatus)> {
        self.entries.iter().map(|entry| match &entry.path {
            Some(path) => (
                &entry.key,
                ProofStatus::Present(
                    path.iter()
                        .map(|&idx| self.resolve_node(idx))
                        .collect::<Result<Vec<_>>>()
                        .expect("invalid multiproof path"),
                ),
            ),
            None => (&entry.key, ProofStatus::Absent),
        })
    }

    /// Looks up a specific key and returns its proof status.
    pub fn proof_status(&self, key: &Hash) -> Option<ProofStatus> {
        self.entries
            .iter()
            .find(|entry| &entry.key == key)
            .map(|entry| match &entry.path {
                Some(path) => ProofStatus::Present(
                    path.iter()
                        .map(|&idx| self.resolve_node(idx))
                        .collect::<Result<Vec<_>>>()
                        .expect("invalid multiproof path"),
                ),
                None => ProofStatus::Absent,
            })
    }

    /// Number of deduplicated nodes stored in this multi-proof.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Total number of entries recorded in the multi-proof.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn resolve_node(&self, idx: u32) -> Result<(bool, Vec<u8>)> {
        let index = idx as usize;
        self.nodes
            .get(index)
            .map(|node| node.as_tuple())
            .ok_or_else(|| Errors::new("multi proof entry references missing node"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monotree::{hasher::Blake3, utils::random_hash, verify_proof, Hasher};

    fn sample_pairs(count: usize) -> (Vec<Hash>, Vec<Hash>) {
        let keys = (0..count).map(|_| random_hash()).collect::<Vec<_>>();
        let leaves = (0..count).map(|_| random_hash()).collect::<Vec<_>>();
        (keys, leaves)
    }

    #[test]
    fn multi_proof_builds_and_verifies_inclusions() {
        let (keys, leaves) = sample_pairs(3);
        let mut tree = Monotree::default();

        let mut root = None;
        for (key, leaf) in keys.iter().zip(leaves.iter()) {
            root = tree.insert(root.as_ref(), key, leaf).unwrap();
        }

        let proof =
            MonotreeMultiProof::build(&mut tree, root.as_ref(), keys.iter().cloned()).unwrap();

        // Expect some deduplication to happen for a non-trivial tree.
        assert!(proof.node_count() <= 3 * keys.len());

        let hasher = Blake3::new();
        for (key, leaf) in keys.iter().zip(leaves.iter()) {
            match proof.proof_status(key).unwrap() {
                ProofStatus::Present(path) => {
                    assert!(verify_proof(&hasher, root.as_ref(), leaf, Some(&path)));
                }
                ProofStatus::Absent => panic!("expected inclusion proof"),
            }
        }
    }

    #[test]
    fn multi_proof_marks_missing_keys() {
        let (keys, leaves) = sample_pairs(2);
        let missing = random_hash();

        let mut tree = Monotree::default();
        let mut root = None;
        for (key, leaf) in keys.iter().zip(leaves.iter()) {
            root = tree.insert(root.as_ref(), key, leaf).unwrap();
        }

        let mut all_keys = keys.clone();
        all_keys.push(missing);

        let proof =
            MonotreeMultiProof::build(&mut tree, root.as_ref(), all_keys.iter().cloned()).unwrap();

        // Missing key should be flagged as absent.
        assert_eq!(proof.proof_status(&missing), Some(ProofStatus::Absent));

        // Included keys must still verify correctly.
        let hasher = Blake3::new();
        for (key, leaf) in keys.iter().zip(leaves.iter()) {
            if let ProofStatus::Present(path) = proof.proof_status(key).unwrap() {
                assert!(verify_proof(&hasher, root.as_ref(), leaf, Some(&path)));
            } else {
                panic!("expected inclusion proof for inserted key");
            }
        }
    }

    #[test]
    fn multi_proof_deduplicates_siblings() {
        use std::collections::HashSet;

        let pairs = [
            (random_hash(), random_hash()),
            (random_hash(), random_hash()),
            (random_hash(), random_hash()),
            (random_hash(), random_hash()),
        ];

        let mut tree = Monotree::default();
        let mut root = None;
        for (key, leaf) in pairs.iter() {
            root = tree.insert(root.as_ref(), key, leaf).unwrap();
        }

        let keys: Vec<Hash> = pairs.iter().map(|(k, _)| *k).collect();
        let multiproof =
            MonotreeMultiProof::build(&mut tree, root.as_ref(), keys.iter().cloned()).unwrap();

        // Build the naive set of siblings by querying each proof individually.
        let mut tree_for_naive = Monotree::default();
        let mut root_naive = None;
        for (key, leaf) in pairs.iter() {
            root_naive = tree_for_naive
                .insert(root_naive.as_ref(), key, leaf)
                .unwrap();
        }

        let mut unique_nodes = HashSet::new();
        for (key, _) in pairs.iter() {
            let proof = tree_for_naive
                .get_merkle_proof(root_naive.as_ref(), key)
                .unwrap()
                .unwrap();
            for (right, cut) in proof {
                unique_nodes.insert(ProofNode { right, cut });
            }
        }

        assert_eq!(multiproof.node_count(), unique_nodes.len());

        // Ensure each aggregated path matches the naive path for that key.
        for (key, _) in pairs.iter() {
            let naive = tree_for_naive
                .get_merkle_proof(root_naive.as_ref(), key)
                .unwrap()
                .unwrap();
            if let ProofStatus::Present(path) = multiproof.proof_status(key).unwrap() {
                assert_eq!(path, naive);
            } else {
                panic!("expected inclusion proof for inserted key");
            }
        }
    }
}
