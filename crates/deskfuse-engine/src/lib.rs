//! Shared application engine. No UI framework or UI event loop is required.
pub mod app_update;
pub mod clipboard_bundle;
pub mod diagnostics;
#[cfg_attr(test, allow(dead_code))]
mod diagnostics_file;
pub mod diagnostics_redact;
#[cfg(not(windows))]
mod github_update;
#[cfg(windows)]
#[path = "github_update_windows.rs"]
mod github_update;
pub mod host;
pub mod input_coordinator;
pub mod model;
mod platform;
pub mod tokio_bridge;

pub mod state;

pub mod history_search;

pub mod mouse_settings;
pub mod workflow;
pub mod workflow_catalog;
pub mod workflow_image;
pub mod workflow_sharing;

mod clipboard_echo;
mod clipboard_image;
pub mod clipboard_library;
pub mod library_sync;

pub mod library_files;

#[cfg(target_os = "macos")]
pub mod input_service;

#[cfg(target_os = "macos")]
pub mod lan_web;

pub mod microphone;
pub mod remote_application;

#[cfg(target_os = "macos")]
mod remote_application_input;
