//! Every query the server runs. This directory is not a crate of its own:
//! neo-db-sqlite and neo-db-postgres both compile it, each against its own
//! database, so the SQL is written once and checked by `query!` on both
//! (book/src/data-model.md, "存储层的代码结构"). A backend crate provides
//! `Database`, `Pool` and `Tx` at its root.

pub mod analysis;
pub mod detail;
pub mod edit;
pub mod import;
pub mod legacy_state;
pub mod library;
pub mod media;
pub mod metadata;
pub mod progress;
pub mod reviews;
pub mod roots;
pub mod scan;
pub mod server_settings;
pub mod settings;
pub mod tags;
pub mod tasks;
pub mod users;
pub mod works;
