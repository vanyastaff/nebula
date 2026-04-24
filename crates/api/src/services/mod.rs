//! Services
//!
//! Business logic layer (вызов портов, оркестрация).

pub mod credential;
pub mod oauth;
pub mod webhook;

// TODO: Implement service layer for business logic
// Services будут вызывать методы портов (WorkflowRepo, ExecutionRepo, etc.)
// и содержать бизнес-правила, которые не должны быть в handlers.
