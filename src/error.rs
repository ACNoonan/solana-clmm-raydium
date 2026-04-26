// Replacement for `crate::error::ErrorCode` from raydium-clmm program.
// Variants are the subset actually referenced from libraries/.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    TickUpperOverflow,
    SqrtPriceX64,
    LiquiditySubValueErr,
    LiquidityAddValueErr,
    MaxTokenOverflow,
    SqrtPriceLimitOverflow,
    InvalidTickIndex,
}

pub type Result<T> = core::result::Result<T, ErrorCode>;

#[macro_export]
macro_rules! require {
    ($cond:expr, $err:expr $(,)?) => {
        if !($cond) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gt {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a > $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! require_gte {
    ($a:expr, $b:expr, $err:expr $(,)?) => {
        if !($a >= $b) {
            return Err($err);
        }
    };
}

#[macro_export]
macro_rules! err {
    ($err:expr $(,)?) => {
        Err($err)
    };
}
