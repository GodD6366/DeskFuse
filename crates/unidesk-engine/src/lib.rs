//! Shared application engine. No UI framework or UI event loop is required.
pub mod app_update;
pub mod clipboard_bundle;
pub mod diagnostics;
#[cfg(not(test))]
mod diagnostics_file;
mod github_update;
pub mod input_coordinator;
pub mod model;
mod platform;
pub mod tokio_bridge;

pub mod state;

pub mod history_search;

pub mod workflow;
pub mod workflow_catalog;
pub mod workflow_sharing;

pub mod clipboard_library;

pub mod library_files;

#[cfg(target_os = "macos")]
pub mod input_service;

#[cfg(target_os = "macos")]
pub mod lan_web;
