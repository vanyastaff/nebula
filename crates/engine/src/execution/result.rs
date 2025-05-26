use std::thread::JoinHandle;
use tokio::sync::{mpsc, watch};
use crate::execution::status::{ExecutionProgress, ExecutionStatus};

#[derive(Debug)]
pub enum ExecutionResult<T, E> {
    /// Результат для ручного режима
    Manual(Result<T, E>),

    /// Handle для фонового режима
    Background {
        task_id: String,
        handle: JoinHandle<Result<T, E>>,
        progress_rx: Option<mpsc::Receiver<ExecutionProgress>>,
    },

    /// Stream для потокового режима
    Streaming(Box<dyn Stream<Item = Result<PollingResult<T>, E>> + Send + Unpin>),

    /// Контроллер для управляемого режима
    Controlled {
        task_id: String,
        handle: JoinHandle<Result<T, E>>,
        control_tx: mpsc::Sender<ControlCommand>,
        status_rx: watch::Receiver<ExecutionStatus>,
    },
}