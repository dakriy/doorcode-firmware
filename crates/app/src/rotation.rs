//! Handing over to new broker credentials. The new pair is kept beside the
//! old one until it has worked; while the broker refuses it the door
//! serves with the old pair and tries the new one again later.

use protocol::Credentials;

pub(crate) struct Rotation {
    pub(crate) next: Credentials,
    /// Whether the connection asked for or held uses the new pair.
    pub(crate) trying: bool,
    /// Times in a row the broker refused the new pair.
    pub(crate) refusals: u32,
    /// Whether the broker refused the other pair just before this one, so
    /// a refusal of this one too means both are revoked.
    other_refused: bool,
    /// While connected with the old pair, when to try the new one again.
    pub(crate) retry_at: Option<u64>,
}

impl Rotation {
    /// A hand-over to `next` that has not been tried yet.
    pub(crate) fn to(next: Credentials) -> Rotation {
        Rotation {
            next,
            trying: false,
            refusals: 0,
            other_refused: false,
            retry_at: None,
        }
    }

    /// Trying the new pair now.
    pub(crate) fn tried(self) -> Rotation {
        Rotation {
            trying: true,
            retry_at: None,
            ..self
        }
    }

    /// Trying the other pair after the broker refused this one; `None`
    /// when it refused the other pair just before, so both are revoked.
    pub(crate) fn switched(self) -> Option<Rotation> {
        if self.other_refused {
            return None;
        }
        let refusals = if self.trying {
            self.refusals + 1
        } else {
            self.refusals
        };
        Some(Rotation {
            trying: !self.trying,
            refusals,
            other_refused: true,
            retry_at: None,
            ..self
        })
    }

    /// Connected with the old pair; the new one is tried again at `at`.
    pub(crate) fn retried_at(self, at: u64) -> Rotation {
        Rotation {
            other_refused: false,
            retry_at: Some(at),
            ..self
        }
    }
}
