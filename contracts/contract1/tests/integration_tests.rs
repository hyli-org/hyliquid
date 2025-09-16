use client_sdk::tests::assert_handle;
use contract1::{Contract1, Contract1Action};
use sdk::{BlobIndex, BlobTransaction};

#[test]
fn test_contract_lifecycle() {
    let mut contract1 = Contract1::default();

    let tx = BlobTransaction::new(
        sdk::Identity("identityTest".into()),
        vec![Contract1Action::Increment.as_blob("contract1".into())],
    );

    assert_handle(&mut contract1, &tx, BlobIndex(0));

    assert_eq!(contract1.n, 1);

    assert_handle(&mut contract1, &tx, BlobIndex(0));

    assert_eq!(contract1.n, 2);
}
