#!/bin/bash

function tx {
    echo "Next command: $@"
    read -p "Press enter to continue"

    cargo run --bin tx_sender -F nonreproducible -- "$@"
}

identity=hyli@wallet

tx --identity $identity create-pair --pair-token1 BTC --pair-token2 USDT
tx --identity $identity add-session-key
tx --identity $identity deposit --token USDT --amount 1000000000
tx --identity $identity deposit --token BTC --amount 125000000
tx --identity $identity create-order --order-id 01998501-dbeb-72ce-9fe1-a09cc9473fe2 --order-side bid --order-type limit --pair-token1 BTC --pair-token2 USDT --quantity 100000 --price 27500000000
tx --identity $identity create-order --order-id 0199850f-f129-72d0-8fa8-ec7e732f41e9 --order-side ask --order-type limit --pair-token1 BTC --pair-token2 USDT --quantity 25000000 --price 27600000000
echo "First matching trade"
tx --identity $identity create-order --order-id 0199851b-7a3a-737d-9148-929cace1fa70 --order-side bid --order-type limit --pair-token1 BTC --pair-token2 USDT --quantity 100000 --price 27600000000
echo "Second matching trade"
tx --identity $identity create-order --order-id 0199852e-b569-70fc-8c89-b88bb2040dd5 --order-side bid --order-type limit --pair-token1 BTC --pair-token2 USDT --quantity 100000 --price 27600000000
