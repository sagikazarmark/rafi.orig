//! Rafi core: the raffle's lifecycle, how a raffle uses a drawing, join and
//! claim codes, and email normalization. The drawing itself lives in
//! `rafi-drawing`, which knows nothing of raffles.

pub mod codes;
pub mod drawing;
pub mod email;
pub mod raffle;
