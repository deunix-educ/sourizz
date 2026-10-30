//! Sourizz — cursor highlighter for Linux (X11 and Wayland).

// Style lints we deliberately do not follow: nested `if let` reads better than let-chains in the
// event handlers, the drawing helpers take plain geometry parameters, and the RGBA<->BGRA loops
// are clearer with `chunks_exact`.
#![allow(clippy::collapsible_if, clippy::too_many_arguments, clippy::chunks_exact_to_as_chunks)]

pub mod annot;
pub mod audio;
pub mod canvas;
pub mod config;
pub mod control;
pub mod core;
pub mod demo;
pub mod engine;
pub mod evdev_in;
pub mod fx;
pub mod geom;
pub mod i18n;
pub mod input;
pub mod ipc;
pub mod keysym;
pub mod keyviz;
pub mod kwin;
pub mod text;
pub mod tray;
pub mod wayland;
pub mod wlsrc;
pub mod x11;
pub mod xkb;
