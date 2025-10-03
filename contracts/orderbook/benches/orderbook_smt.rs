use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use rand::{rngs::StdRng, RngCore, SeedableRng};
use sparse_merkle_tree::{
    blake2b::Blake2bHasher, default_store::DefaultStore, SparseMerkleTree, H256,
};

type SMT = SparseMerkleTree<Blake2bHasher, H256, DefaultStore<H256>>;

fn random_h256(rng: &mut StdRng) -> H256 {
    let mut bytes = [0u8; 32];
    rng.fill_bytes(&mut bytes);
    H256::from(bytes)
}

fn random_leaves(count: usize, seed: u64) -> Vec<(H256, H256)> {
    let mut rng = StdRng::seed_from_u64(seed);
    (0..count)
        .map(|_| (random_h256(&mut rng), random_h256(&mut rng)))
        .collect()
}

fn populate_tree(count: usize, seed: u64) -> (SMT, Vec<(H256, H256)>) {
    let mut tree: SMT = SparseMerkleTree::new(H256::zero(), DefaultStore::default());
    let mut leaves = random_leaves(count, seed);
    for (key, value) in &leaves {
        tree.update(*key, *value).expect("smt update");
    }
    leaves.sort_by_key(|(k, _)| *k);
    (tree, leaves)
}

fn bench_smt_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_update");
    for &count in &[16usize, 64, 256] {
        group.bench_with_input(BenchmarkId::new("update_all", count), &count, |b, &n| {
            b.iter_batched(
                || random_leaves(n, 0xA5A5A5 + n as u64),
                |mut leaves| {
                    leaves.sort_by_key(|(k, _)| *k);
                    let mut tree: SMT =
                        SparseMerkleTree::new(H256::zero(), DefaultStore::default());
                    tree.update_all(leaves).expect("smt update_all");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_smt_merkle_proof(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_merkle_proof");
    for &count in &[16usize, 64, 256] {
        group.bench_with_input(BenchmarkId::new("proof", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let (tree, leaves) = populate_tree(n, 0xBEEFu64 + n as u64);
                    let keys: Vec<H256> = leaves.iter().map(|(k, _)| *k).collect();
                    (tree, keys)
                },
                |(tree, keys)| {
                    tree.merkle_proof(keys).expect("smt proof generation");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_smt_verify(c: &mut Criterion) {
    let mut group = c.benchmark_group("smt_verify");
    for &count in &[16usize, 64, 256] {
        group.bench_with_input(BenchmarkId::new("verify", count), &count, |b, &n| {
            b.iter_batched(
                || {
                    let (tree, leaves) = populate_tree(n, 0xFACEu64 + n as u64);
                    let keys: Vec<H256> = leaves.iter().map(|(k, _)| *k).collect();
                    let proof = tree
                        .merkle_proof(keys.clone())
                        .expect("smt proof generation");
                    let root = *tree.root();
                    (proof, root, leaves)
                },
                |(proof, root, leaves)| {
                    let mut sorted_leaves = leaves.clone();
                    sorted_leaves.sort_by_key(|(k, _)| *k);
                    proof
                        .verify::<Blake2bHasher>(&root, sorted_leaves)
                        .expect("smt proof verify");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    smt_benches,
    bench_smt_update,
    bench_smt_merkle_proof,
    bench_smt_verify,
);
criterion_main!(smt_benches);
