use std::env;

use hex::FromHex;
use sdk::StateCommitment;
use server::init::DebugStateCommitment;

fn main() {
    let mut args = env::args().skip(1);
    let hex_input = args
        .next()
        .expect("usage: parse_state <hex-encoded state commitment>");

    let hex_input = hex_input.strip_prefix("0x").unwrap_or(&hex_input);

    let bytes = Vec::from_hex(hex_input).expect("invalid hex string");

    let commitment = StateCommitment(bytes);
    let debug = DebugStateCommitment::from(commitment);

    println!("{debug:#?}");
}
