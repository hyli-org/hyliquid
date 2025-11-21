use orderbook::{
    model::{
        AssetInfo, Balance, ExecuteState, Order, OrderSide, OrderType, OrderbookEvent, PairInfo,
        UserInfo,
    },
    zk::smt::GetKey,
};
use sdk::ContractName;
#[cfg(feature = "instrumentation")]
use tracing::info_span;

#[cfg(feature = "instrumentation")]
use server::setup::init_tracing;

#[tokio::main]
async fn main() {
    let (mut state, taker, taker_order) = build_dense_orderbook(24, 4);

    #[cfg(feature = "instrumentation")]
    let tracer_provider = init_tracing();

    #[cfg(feature = "instrumentation")]
    let span = info_span!(
        "execute_order_run",
        taker = %taker.user,
        order_id = %taker_order.order_id,
        side = ?taker_order.order_side,
        qty = taker_order.quantity
    );
    #[cfg(feature = "instrumentation")]
    let _guard = span.enter();

    let events = state
        .execute_order(&taker, taker_order)
        .expect("execute order");

    state.apply_events(&taker, &events).expect("apply events");
    tracing::info!("Generated {} events", events.len());

    // Flush the batch OTLP exporter; short-lived binaries exit before Tempo receives spans otherwise.
    #[cfg(feature = "instrumentation")]
    tracer_provider.shutdown().ok();
}

fn build_dense_orderbook(
    levels: usize,
    orders_per_level: usize,
) -> (ExecuteState, UserInfo, Order) {
    let mut state = ExecuteState::default();
    let pair = ("BASE".to_string(), "QUOTE".to_string());
    let pair_info = PairInfo {
        base: AssetInfo::new(0, ContractName("BASE".to_string())),
        quote: AssetInfo::new(0, ContractName("QUOTE".to_string())),
    };

    let system_user = UserInfo::new("system".to_string(), b"system".to_vec());
    let pair_events = state.create_pair(&pair, &pair_info).expect("create pair");
    state
        .apply_events(&system_user, &pair_events)
        .expect("apply pair events");

    let taker = UserInfo::new("taker".to_string(), b"taker".to_vec());
    let taker_key = taker.get_key();
    state.users_info.insert(taker.user.clone(), taker.clone());
    state
        .update_balances(&pair.0, vec![(taker_key, Balance(50_000_000))])
        .expect("fund taker base");
    state
        .update_balances(&pair.1, vec![(taker_key, Balance(50_000_000))])
        .expect("fund taker quote");

    for level in 0..levels {
        let price = 1_000 + level as u64;
        for idx in 0..orders_per_level {
            let maker = UserInfo::new(format!("maker-{level}-{idx}"), vec![level as u8, idx as u8]);
            let maker_key = maker.get_key();
            state.users_info.insert(maker.user.clone(), maker.clone());
            state
                .update_balances(&pair.0, vec![(maker_key, Balance(10_000_000))])
                .expect("fund maker base");
            state
                .update_balances(&pair.1, vec![(maker_key, Balance(10_000_000))])
                .expect("fund maker quote");

            let order = Order {
                order_id: format!("bid-{level}-{idx}"),
                order_type: OrderType::Limit,
                order_side: OrderSide::Bid,
                price: Some(price),
                pair: pair.clone(),
                quantity: 1_000,
            };
            let events = vec![OrderbookEvent::OrderCreated {
                order: order.clone(),
            }];
            state
                .apply_events_preserving_zeroed_orders(&maker, &events)
                .expect("insert maker order");
        }
    }

    let taker_order = Order {
        order_id: "taker-order".to_string(),
        order_type: OrderType::Market,
        order_side: OrderSide::Ask,
        price: None,
        pair,
        quantity: (levels * orders_per_level * 1_000) as u64 / 2,
    };

    (state, taker, taker_order)
}
