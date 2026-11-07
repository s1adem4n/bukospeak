use bukospeak_proto::control::Rejection;

use crate::channel::Lost;
use crate::session::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    Rejected(Rejection),
    NotFound,
    NoAnswer,
    Unreachable,
    Audio,
    ConnectionLost,
    Replaced,
}

impl From<&Error> for Failure {
    fn from(error: &Error) -> Self {
        match error {
            Error::Rejected(rejection) => Self::Rejected(*rejection),
            Error::Replaced => Self::Replaced,
            Error::Resolve(_) => Self::NotFound,
            Error::TimedOut => Self::NoAnswer,
            Error::Connect(_)
            | Error::Connection(_)
            | Error::Control(_)
            | Error::Read(_)
            | Error::Protocol(_)
            | Error::Io(_)
            | Error::Damaged(_) => Self::Unreachable,
        }
    }
}

impl From<&Lost> for Failure {
    fn from(lost: &Lost) -> Self {
        match lost {
            Lost::Unrecovered { .. } => Self::ConnectionLost,
            Lost::Replaced => Self::Replaced,
        }
    }
}
