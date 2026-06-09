use chrono::{DateTime, Utc};
use typed_builder::TypedBuilder;

use crate::domain::ids::UserId;
use crate::domain::values::{CognitoGroup, Currency, Language};

// ---------------------------------------------------------------------------
// User
// ---------------------------------------------------------------------------

/// A user in the hierarchy.
///
/// Ported 1:1 from `user.ml`.
///
/// Construct via `User::builder()` (TypedBuilder).  The `id` is derived from
/// `email` automatically (`UserId::of_email`); `language` defaults to
/// `Language::Danish`, `currency` to `Currency::Dkk`, and `created` to
/// `Utc::now()` — all matching the OCaml optional-argument defaults.
///
/// Every field can still be set explicitly via the builder (codec / tests
/// that need a specific timestamp or id simply call `.created(ts)` / `.id(id)`).
///
/// ```
/// # use model::domain::user::User;
/// # use model::domain::values::{CognitoGroup, Currency, Language};
/// let u = User::builder()
///     .email("alice@example.com".to_owned())
///     .name("Alice".to_owned())
///     .cognito_group(CognitoGroup::Admin)
///     .build();
/// assert_eq!(u.language, Language::Danish);
/// assert_eq!(u.id.to_string(), "U#alice@example.com");
/// ```
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct User {
    pub email: String,
    #[builder(default = UserId::of_email(&email))]
    pub id: UserId,
    pub name: String,
    pub cognito_group: CognitoGroup,
    #[builder(default = Language::default())]
    pub language: Language,
    #[builder(default = Currency::default())]
    pub currency: Currency,
    #[builder(default = chrono::Utc::now())]
    pub created: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests (port of test_domain_user.ml — user struct make + defaults)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    /// id is derived from email; name, cognito_group set correctly.
    #[test]
    fn make_sets_id_from_email() {
        let u = User::builder()
            .email("bob@example.com".to_owned())
            .name("Bob".to_owned())
            .cognito_group(CognitoGroup::Writer)
            .created(now())
            .build();
        assert_eq!(u.id.email(), "bob@example.com");
        assert_eq!(u.id.to_string(), "U#bob@example.com");
        assert_eq!(u.name, "Bob");
        assert_eq!(u.cognito_group, CognitoGroup::Writer);
    }

    /// Default language is Danish (matches OCaml `Language.default`).
    #[test]
    fn make_default_language_is_danish() {
        let u = User::builder()
            .email("a@b.com".to_owned())
            .name("A".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .build();
        assert_eq!(u.language, Language::Danish);
    }

    /// Default currency is DKK (matches OCaml `Currency.default`).
    #[test]
    fn make_default_currency_is_dkk() {
        let u = User::builder()
            .email("a@b.com".to_owned())
            .name("A".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .build();
        assert_eq!(u.currency, Currency::Dkk);
    }

    /// Explicit language / currency are preserved.
    #[test]
    fn make_explicit_language_and_currency() {
        let u = User::builder()
            .email("a@b.com".to_owned())
            .name("A".to_owned())
            .cognito_group(CognitoGroup::Admin)
            .language(Language::English)
            .currency(Currency::Eur)
            .build();
        assert_eq!(u.language, Language::English);
        assert_eq!(u.currency, Currency::Eur);
    }

    /// TypedBuilder: fields with defaults can be omitted; created defaults to now().
    #[test]
    fn builder_defaults() {
        let u = User::builder()
            .email("c@d.com".to_owned())
            .name("C".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .build();
        assert_eq!(u.language, Language::Danish);
        assert_eq!(u.currency, Currency::Dkk);
        // id is derived from email
        assert_eq!(u.id.to_string(), "U#c@d.com");
    }

    /// Explicit id overrides the derived default.
    #[test]
    fn builder_explicit_id_overrides_default() {
        let explicit_id = UserId::of_email("other@example.com");
        let u = User::builder()
            .email("x@y.com".to_owned())
            .id(explicit_id.clone())
            .name("X".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .build();
        assert_eq!(u.id, explicit_id);
    }

    /// Explicit created is preserved (codec / tests need specific timestamps).
    #[test]
    fn builder_explicit_created_preserved() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let u = User::builder()
            .email("a@b.com".to_owned())
            .name("A".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .created(ts)
            .build();
        assert_eq!(u.created, ts);
    }
}
