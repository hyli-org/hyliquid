// Runtime configuration - this file is generated at container startup in production
// For local development, these defaults are used
window.__CONFIG__ = {
  API_BASE_URL: "http://localhost:3000",
  BACKEND_API_URL: "http://localhost:9002",
  WEBSOCKET_URL: "ws://localhost:3000/ws",
  ETH_COLLATERAL_TOKEN_ADDRESS: "0x5FbDB2315678afecb367f032d93F642f64180aa3",
  HYLI_VAULT_ADDRESS: "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65",
  COLLATERAL_NETWORKS: JSON.stringify([
    {
      id: "ethereum-mainnet",
      name: "Ethereum Mainnet",
      chainId: "0x1",
      tokenAddress: "0x0000000000000000000000000000000000000000",
      vaultAddress: "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65"
    },
    {
      id: "arbitrum-one",
      name: "Arbitrum One",
      chainId: "0xa4b1",
      tokenAddress: "0x0000000000000000000000000000000000000000",
      vaultAddress: "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65"
    },
    {
      id: "local-anvil",
      name: "Local (Anvil)",
      chainId: "0x7a69",
      tokenAddress: "0x5FbDB2315678afecb367f032d93F642f64180aa3",
      vaultAddress: "0x15d34aaf54267db7d7c367839aaf71a00a2c6a65",
      rpcUrl: "http://localhost:8545"
    }
  ])
};
