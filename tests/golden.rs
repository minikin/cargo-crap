//! Golden outputs of the cargo-crap binary: every case's stdout, stderr and
//! exit status, recorded from main at `4b57930` (package version 0.9.0), so
//! a later change that moves code between crates proves it changed no byte.
//!
//! Unix only: Windows prints `.\src\lib.rs` and its own OS error text, and a
//! normaliser that hid those could hide a real change.

#![cfg(unix)]

#[path = "golden/config.rs"]
mod config;

#[path = "golden/encode.rs"]
mod encode;
