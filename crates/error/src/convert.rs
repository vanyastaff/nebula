//! Error conversion utilities.
//!
//! This module is reserved for future HTTP status code and gRPC status
//! conversions. These bridges will be added behind feature flags
//! (`http`, `grpc`) so the core crate stays dependency-free.
//!
//! Planned conversions:
//!
//! - `ErrorCategory` -> HTTP status code
//! - `ErrorCategory` -> gRPC status code
//! - `NebulaError<E>` -> `http::Response` (with feature `http`)
//! - `NebulaError<E>` -> `tonic::Status` (with feature `grpc`)
