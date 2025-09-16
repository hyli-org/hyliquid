#!/bin/sh

# Sorry for that


VITE_SERVER_BASE_URL=${VITE_SERVER_BASE_URL:-"http://wallet-server:8081"}
VITE_NODE_BASE_URL=${VITE_NODE_BASE_URL:-"http://hyli:4321"}
VITE_WALLET_SERVER_BASE_URL=${VITE_WALLET_SERVER_BASE_URL:-"http://wallet-server:8081"}
VITE_WALLET_WS_URL=${VITE_WALLET_WS_URL:-"ws://wallet-server:8081/ws"}
VITE_INDEXER_BASE_URL=${VITE_INDEXER_BASE_URL:-"http://hyli:4321"}
VITE_TX_EXPLORER_URL=${VITE_TX_EXPLORER_URL:-"http://hyli:4321"}
VITE_FAUCET_URL=${VITE_FAUCET_URL:-"http://wallet-server:8081"}

escape_sed() {
    echo "$1" | sed 's/[[\.*^$()+?{|]/\\&/g'
}

find /usr/share/nginx/html -name "*.js" -type f | while read -r file; do
    echo "Processing $file..."
    
    sed -i "s|http://localhost:8081|$VITE_WALLET_SERVER_BASE_URL|g" "$file"
    sed -i "s|https://localhost:8081|$VITE_WALLET_SERVER_BASE_URL|g" "$file"
    sed -i "s|http://localhost:4321|$VITE_NODE_BASE_URL|g" "$file"
    sed -i "s|https://localhost:4321|$VITE_NODE_BASE_URL|g" "$file"
    sed -i "s|ws://localhost:8081/ws|$VITE_WALLET_WS_URL|g" "$file"
    sed -i "s|wss://localhost:8081/ws|$VITE_WALLET_WS_URL|g" "$file"
    
    sed -i "s|https://wallet\.testnet\.hyli\.org|$VITE_WALLET_SERVER_BASE_URL|g" "$file"
    sed -i "s|https://node\.testnet\.hyli\.org|$VITE_NODE_BASE_URL|g" "$file"
    sed -i "s|https://indexer\.testnet\.hyli\.org|$VITE_INDEXER_BASE_URL|g" "$file"
    sed -i "s|https://explorer\.hyli\.org|$VITE_TX_EXPLORER_URL|g" "$file"
    sed -i "s|https://faucet\.testnet\.hyli\.org|$VITE_FAUCET_URL|g" "$file"
    
    sed -i "s|wallet-server:8081|$(echo $VITE_WALLET_SERVER_BASE_URL | sed 's|https\?://||')|g" "$file"
    sed -i "s|hyli:4321|$(echo $VITE_NODE_BASE_URL | sed 's|https\?://||')|g" "$file"
done

exec nginx -g "daemon off;"