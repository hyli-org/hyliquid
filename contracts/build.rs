use sp1_helper::{build_program_with_args, BuildArgs};

fn main() {
    build_program_with_args(
        "./orderbook",
        BuildArgs {
            features: vec!["sp1".to_string()],
            output_directory: Some("../elf".to_string()),
            ..Default::default()
        },
    )
}
