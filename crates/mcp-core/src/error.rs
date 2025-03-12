use std::{borrow::Cow, fmt::Display};

use crate::schema::{ErrorCode, ErrorData};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum Error {
    #[error("Internal error: {0}")]
    Internal(#[from] InternalError),
    #[error("MethodNotFound")]
    MethodNotFound,
}

impl Error {
    pub fn internal<E: std::error::Error>(e: E) -> Self {
        Self::Internal(InternalError::from_std(e))
    }
    pub fn not_found() -> Self {
        Self::MethodNotFound
    }
}

impl From<ErrorData> for Error {
    fn from(val: ErrorData) -> Self {
        match val.code {
            ErrorCode::INTERNAL_ERROR => Error::Internal(InternalError {
                message: val.message.into(),
                data: val.data,
            }),
            ErrorCode::METHOD_NOT_FOUND => Error::MethodNotFound,
            _ => {
                todo!()
            }
        }
    }
}
impl From<Error> for ErrorData {
    fn from(val: Error) -> Self {
        match val {
            Error::Internal(e) => e.into(),
            Error::MethodNotFound => ErrorData {
                code: ErrorCode::METHOD_NOT_FOUND,
                message: String::default(),
                data: None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct InternalError {
    pub message: Cow<'static, str>,
    pub data: Option<Value>,
}

impl std::error::Error for InternalError {}
impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl InternalError {
    pub fn from_std<E: std::error::Error>(error: E) -> Self {
        InternalError {
            message: error.to_string().into(),
            data: None,
        }
    }
    pub fn with_data<T: Serialize>(self, data: &T) -> Self {
        Self {
            data: Some(serde_json::to_value(data).expect("a valid json")),
            ..self
        }
    }
}

impl From<InternalError> for ErrorData {
    fn from(val: InternalError) -> Self {
        ErrorData {
            message: val.message.to_string(),
            data: val.data,
            code: ErrorCode::INTERNAL_ERROR,
        }
    }
}
