#[allow(unused_imports)]
use sp1_build::{build_program_with_args, vkey, BuildArgs};

#[cfg(feature = "nobuild")]
fn main() {}

#[cfg(not(feature = "nobuild"))]
fn main() {
    use std::{fs::File, io::Read};

    use sp1_sdk::{Prover, ProverClient};

    build_program_with_args(
        "./orderbook",
        BuildArgs {
            docker: !cfg!(feature = "nonreproducible"),
            features: vec!["sp1".to_string()],
            output_directory: Some("../elf".to_string()),
            ..Default::default()
        },
    );

    let mut file = File::open("../elf/orderbook").unwrap();
    let mut elf = Vec::new();

    file.read_to_end(&mut elf).unwrap();

    let local_client = ProverClient::builder().cpu().build();
    let (_, vk) = local_client.setup(&elf);

    let vk = serde_json::to_vec(&vk).expect("Failed to serialize SP1 Proving Key");

    std::fs::write("../elf/orderbook_vk", vk).expect("Failed to write verification key");
}
