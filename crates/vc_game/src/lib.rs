//! The vc-rs runtime: a resident Vice City, physically lit.
//!
//! The binary is a thin shell around these modules: [`world`] plans and bakes the
//! map, [`sky`] reads the timecyc, [`camera`] drives the view and [`hud`] reports.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod camera;
pub mod hud;
pub mod scene;
pub mod sky;
pub mod world;
