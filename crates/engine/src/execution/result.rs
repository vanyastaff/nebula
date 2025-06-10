use std::thread::JoinHandle;
use tokio::sync::{mpsc, watch};
use crate::execution::status::{ExecutionProgress, ExecutionStatus};

#[derive(Debug)]
pub enum ExecutionResult<T, E> {
    
}
