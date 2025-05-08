mod context;
mod runtime;

use quickjs_rusty::{ExecutionError, ValueError};
pub use runtime::*;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Execution(#[from] ExecutionError),
    #[error(transparent)]
    Value(#[from] ValueError),
    #[error(transparent)]
    Serde(#[from] quickjs_rusty::serde::Error),
    #[error(transparent)]
    Context(#[from] quickjs_rusty::ContextError),

    #[cfg(feature = "transpiling")]
    #[error(transparent)]
    Parse(#[from] deno_ast::ParseDiagnostic),
    #[cfg(feature = "transpiling")]
    #[error(transparent)]
    Transpile(#[from] deno_ast::TranspileError),

    #[error("unexpected")]
    Unexpected(String),
}
