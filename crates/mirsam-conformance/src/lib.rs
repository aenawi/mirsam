//! One suite, every adapter — and no library code at all.
//!
//! The suite in `tests/conformance.rs` asks whether the format adapters
//! *agree*, so it has to see all of them at once. Its home used to be
//! `mirsam-ooxml`, which worked while every adapter was an OOXML one and
//! stopped working the moment HTML arrived: a suite living inside one adapter
//! would have made that adapter depend on its peers, which is the hexagon
//! leaking through the test tree.
//!
//! So this crate holds the suite and nothing else. It is not published, it
//! exports nothing, and a caller has no reason to depend on it.
