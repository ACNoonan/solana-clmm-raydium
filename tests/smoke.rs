use solana_clmm_raydium::{
    get_sqrt_price_at_tick, get_tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MAX_TICK,
    MIN_SQRT_PRICE_X64, MIN_TICK,
};

#[test]
fn boundary_constants_round_trip() {
    assert_eq!(
        get_sqrt_price_at_tick(MIN_TICK).unwrap(),
        MIN_SQRT_PRICE_X64
    );
    assert_eq!(
        get_sqrt_price_at_tick(MAX_TICK).unwrap(),
        MAX_SQRT_PRICE_X64
    );
}

#[test]
fn tick_zero_is_q64() {
    let q64: u128 = (u64::MAX as u128) + 1;
    assert_eq!(get_sqrt_price_at_tick(0).unwrap(), q64);
}

#[test]
fn tick_round_trip_sample() {
    for &tick in &[
        -443_636, -100_000, -10_000, -100, -1, 0, 1, 100, 10_000, 100_000, 443_635,
    ] {
        let sp = get_sqrt_price_at_tick(tick).unwrap();
        let recovered = get_tick_at_sqrt_price(sp).unwrap();
        assert_eq!(
            recovered, tick,
            "round-trip failed at tick={}, sqrt_price={}",
            tick, sp
        );
    }
}
