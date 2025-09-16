# Hyli App Scaffold

This repository provides a scaffold to build applications on the Hyli network using Risc0 contracts.

For step-by-step instructions, [follow our quickstart](https://docs.hyli.org/quickstart/run.md).

## Architecture

The application follows a client-server model:

- The frontend sends operation requests to the server.
- The server handles transaction creation, proving, and submission.
- All interactions are executed through the Hyli network.

Currently, only Risc0 contracts are supported.

## Getting Started

### Pre-requisites

- Install [Hylix](https://github.com/hyli-org/hyli/blob/main/crates/hylix/README.md) (Binary: `hy`)
- Clone this repository
- [Install RISC-Zero](https://dev.risczero.com/api/zkvm/install)
- [Install Docker](https://docs.docker.com/compose/install/)

### 1. Start the Hyli devnet

You can run the docker node and the wallet using

```bash
hy devnet start --bake
```

This will launch a development-mode node and the wallet server and ui.

### 2. Start the server

From the root of this repository:

```bash
# Export devnet env vars first, so that server can connect to your local devnet
source <(hy devnet env)>
cargo run -p server
```

This starts the backend service, which handles contract interactions and proofs.

### 3. Start the frontend

To navigate to the frontend directory and start the development server:

```bash
cd front
bun install
bun run dev
```

This runs the web interface for interacting with the Hyli network.

## Development

### Building Contracts

Contract ELF files are rebuilt automatically when changes are made.

For reproducible builds using Docker:

```bash
cargo build -p contracts --features build --features all
```

This ensures builds are consistent across environments.

For more details, refer to the [Hyli documentation](https://docs.hyli.org).
