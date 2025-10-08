#!/bin/bash


# From argument or default to 0
delai=${1:-0}
identity=${2:-tx_sender}

echo "Starting random order generation loop..."
echo "Press Ctrl+C to stop the loop"
read -p "Press enter to start"

function tx {
    echo "Next command: $@"

    cargo run --bin tx_sender -F nonreproducible -- "$@"
}

# Function to generate random UUID
generate_uuid() {
    cat /proc/sys/kernel/random/uuid
}

# Function to generate random number within bounds
random_between() {
    local min=$1
    local max=$2
    echo $((RANDOM % (max - min + 1) + min))
}

mid_price=27500000000

# Loop with random parameters
while true; do
    # Generate random parameters
    order_id=$(generate_uuid)
    side=$([ $((RANDOM % 2)) -eq 0 ] && echo "bid" || echo "ask")
    # order_type=$([ $((RANDOM % 2)) -eq 0 ] && echo "limit" || echo "market")
    order_type="limit"

    # make the price of 1 USDT at each step
    mid_price=$((mid_price + 100000))

    quantity=$(random_between 10000 50000)  # Bid quantity between 10k and 50k

    price=$(random_between $((mid_price - 5000000000)) $((mid_price + 5000000000)))  # Random price between 20k and 30k (in smallest units)
    
    echo "Generating random order: side=$side, type=$order_type, quantity=$quantity, price=$price"
    
    # Execute the transaction with random parameters
    tx --identity $identity create-order \
        --order-id $order_id \
        --order-side $side \
        --order-type $order_type \
        --contract-name1 BTC \
        --contract-name2 USDT \
        --quantity $quantity \
        --price $price
    
    # Optional: Add a small delay between transactions
    sleep $delai
done
