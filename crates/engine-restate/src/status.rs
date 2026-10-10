//! How refusals cross the ingress: a handler's terminal error carries the
//! refusal's kind as an HTTP status code, and the client reads it back.

use rafi_drawing::ErrorKind;

/// The HTTP status code the `Drawing` object answers a refusal of this kind
/// with.
pub fn status_code(kind: ErrorKind) -> u16 {
    match kind {
        ErrorKind::NotFound => 404,
        ErrorKind::BadRequest => 400,
        ErrorKind::Conflict => 409,
        ErrorKind::Invariant => 500,
    }
}

/// The refusal kind a status code stands for; `None` for anything the object
/// never refuses with (the engine is unavailable).
pub fn refusal_kind(status: u16) -> Option<ErrorKind> {
    match status {
        404 => Some(ErrorKind::NotFound),
        400 => Some(ErrorKind::BadRequest),
        409 => Some(ErrorKind::Conflict),
        500 => Some(ErrorKind::Invariant),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips() {
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::BadRequest,
            ErrorKind::Conflict,
            ErrorKind::Invariant,
        ] {
            assert_eq!(refusal_kind(status_code(kind)), Some(kind));
        }
        assert_eq!(refusal_kind(503), None, "unavailable is not a refusal");
    }
}
