mod executor;
mod thread;
mod vm;

use thiserror::Error;

use crate::{
    OutOfMemory,
    meta_ops::{MetaCallError, MetaOperatorError},
    table::TableError,
};

impl From<TableError> for VMError {
    fn from(err: TableError) -> Self {
        VMError::OperatorError(err.into())
    }
}

pub(crate) use self::thread::backtrace;
pub use self::{
    executor::{
        BadExecutorMode, CurrentThread, Execution, Executor, ExecutorInner, ExecutorMode,
        HostOpError, UpperLuaFrame,
    },
    thread::{BadThreadMode, OpenUpValue, Thread, ThreadInner, ThreadMode},
};

#[derive(Debug, Clone, Error)]
pub enum VMError {
    #[error("{}", if *.0 {
        "operation expects variable stack"
    } else {
        "unexpected variable stack during operation"
    })]
    ExpectedVariableStack(bool),
    #[error("Bad types for SetList op, expected table, integer, found {0}, {1}")]
    BadSetList(&'static str, &'static str),
    #[error("bad call: {0}")]
    BadCall(#[from] MetaCallError),
    #[error("operator error: {0}")]
    OperatorError(#[from] MetaOperatorError),
    #[error("{0}")]
    OutOfMemory(#[from] OutOfMemory),
    #[error("_ENV upvalue is only allowed on top-level closure")]
    BadEnvUpValue,
    #[error("Invalid types in for loop; expected numbers, found {0}, {1}, and {2}")]
    BadForLoop(&'static str, &'static str, &'static str),
    #[error("Invalid types in for loop; expected numbers, found {0} and {1}")]
    BadForLoopPrep(&'static str, &'static str),
}
