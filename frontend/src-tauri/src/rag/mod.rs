//! Retrieval-augmented search over meetings (phase 2 of docs/plans/projects-and-rag.md).
//!
//! ingestion/chunking (`chunker`) → embeddings (`embeddings`) → storage + FTS
//! (`store`) → hybrid retrieval (`retriever`), orchestrated by `indexer`.

pub mod answer;
pub mod chunker;
pub mod commands;
pub mod embeddings;
pub mod entities;
pub mod history;
pub mod indexer;
pub mod llm;
pub mod retriever;
pub mod store;

pub use indexer::schedule_meeting_index;
