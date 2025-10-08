use sp1_helper::{build_program_with_args, BuildArgs};

#[cfg(feature = "nobuild")]
fn main() {}

#[cfg(not(feature = "nobuild"))]
fn main() {
    build_program_with_args(
        "./orderbook",
        BuildArgs {
            docker: !cfg!(feature = "nonreproducible"),
            features: vec!["sp1".to_string()],
            output_directory: Some("../elf".to_string()),
            ..Default::default()
        },
    )
}
