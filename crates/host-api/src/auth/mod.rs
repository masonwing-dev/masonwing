//! Browser authentication/session/authorization integration for Masonwing.
//!
//! Prime host integration should expose this module, attach `SessionManagerLayer`
//! outside `router()`, and merge `router(AuthState)` into the application router.

mod error;
mod gate;
mod router;
mod session;

pub use error::AuthError;
pub use gate::{AuthGate, AuthorizationInput, CurrentPermit, VerifiedActor};
pub use router::{AuthState, router};
pub use session::{
    AuthenticatedSession, CSRF_HEADER, CsrfGuard, CurrentAuthority, CurrentMembership,
    CurrentPolicySource, PrincipalRef, SessionAuthority, SessionCookieProfile, SessionView,
};
