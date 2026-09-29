//! Speaker diarization (phase 3 of docs/plans/projects-and-rag.md): who spoke when
//! in a recorded meeting, with speakers matched to project members by voice.

pub mod cluster;
pub mod commands;
pub mod engine;
pub mod naming;
pub mod service;
pub mod store;
