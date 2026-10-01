//! Errors from building and checking Thing Descriptions.

use crate::form::{Operation, OperationScope};

/// A Thing Description breaks a TD 1.1 rule that the type system doesn't enforce.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum TdError {
    /// The `@context` isn't a valid TD 1.1 context.
    #[error("invalid @context: {0}")]
    InvalidContext(String),
    /// `securityDefinitions` is empty.
    #[error("the TD has no security definitions")]
    NoSecurityDefinitions,
    /// Security definitions were added but `security` wasn't set.
    #[error("security definitions were added, but `security` wasn't set")]
    MissingSecurity,
    /// A security name is used but not defined.
    #[error("security scheme `{0}` is used but not defined in securityDefinitions")]
    UndefinedSecurity(String),
    /// Two affordances of the same kind have the same name.
    #[error("{scope:?} affordance `{name}` is defined twice")]
    DuplicateAffordance {
        /// The kind of affordance.
        scope: OperationScope,
        /// The repeated name.
        name: String,
    },
    /// An affordance has an empty name.
    #[error("a {scope:?} affordance has an empty name")]
    EmptyName {
        /// The kind of affordance.
        scope: OperationScope,
    },
    /// An affordance has no forms.
    #[error("{scope:?} affordance `{name}` has no forms")]
    NoForms {
        /// The kind of affordance.
        scope: OperationScope,
        /// The affordance's name.
        name: String,
    },
    /// A form uses an operation that doesn't belong to its affordance.
    #[error("operation `{op:?}` can't be used in the forms of {scope:?} `{name}`")]
    MisplacedOperation {
        /// Where the form is.
        scope: OperationScope,
        /// The affordance's name (empty for top-level forms).
        name: String,
        /// The misplaced operation.
        op: Operation,
    },
}
