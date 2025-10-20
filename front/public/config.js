// Runtime configuration - this file is generated at container startup in production
// For local development, these defaults are used
window.__CONFIG__ = {
  API_BASE_URL: "http://localhost:3000",
  BACKEND_API_URL: "http://localhost:9002",
  WEBSOCKET_URL: "ws://localhost:3000/ws",
  COLLATERAL_NETWORKS: JSON.stringify([
    {
        id: "ethereum-sepolia",
        name: "Ethereum Sepolia",
        chainId: "0xaa36a7",
        tokenAddress: "0x6d6Fc2b5B6F71B84838C70ED1719C9D498FdB083",
        vaultAddress: "0x2ffCC85Db88Dbb4047d4d1528CE7739CFB961302",
        rpcUrl: "https://0xrpc.io/sep",
        blockExplorerUrl: "https://sepolia.etherscan.io",
    },
    {
        id: "ethereum-mainnet",
        name: "Ethereum Mainnet",
        chainId: "0x1",
        tokenAddress: "TBD",
        vaultAddress: "0x2ffCC85Db88Dbb4047d4d1528CE7739CFB961302",
        rpcUrl: "https://tbd",
        blockExplorerUrl: "https://etherscan.io",
    },
  ])
};
