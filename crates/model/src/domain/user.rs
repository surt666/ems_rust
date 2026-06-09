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
/// Construct via `User::make` (mirrors OCaml `User.make`) or via
/// `User::builder()` (TypedBuilder).  The TypedBuilder `language` and
/// `currency` fields default to `Language::Danish` and `Currency::Dkk`
/// respectively, matching the OCaml optional-argument defaults.
#[derive(Clone, Debug, PartialEq, TypedBuilder)]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub cognito_group: CognitoGroup,
    #[builder(default = Language::default())]
    pub language: Language,
    #[builder(default = Currency::default())]
    pub currency: Currency,
    pub created: DateTime<Utc>,
}

impl User {
    /// Functional constructor mirroring OCaml `User.make`.
    ///
    /// ```
    /// # use model::domain::user::User;
    /// # use model::domain::values::{CognitoGroup, Currency, Language};
    /// # use chrono::Utc;
    /// let u = User::make(
    ///     "alice@example.com",
    ///     "Alice",
    ///     CognitoGroup::Admin,
    ///     None,          // language → Danish
    ///     None,          // currency → DKK
    ///     Utc::now(),
    /// );
    /// assert_eq!(u.language, Language::Danish);
    /// ```
    pub fn make(
        email: &str,
        name: &str,
        cognito_group: CognitoGroup,
        language: Option<Language>,
        currency: Option<Currency>,
        created: DateTime<Utc>,
    ) -> User {
        User {
            id: UserId::of_email(email),
            name: name.to_owned(),
            cognito_group,
            language: language.unwrap_or_default(),
            currency: currency.unwrap_or_default(),
            created,
        }
    }
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

    /// make uses email for id, name, cognito_group, created.
    #[test]
    fn make_sets_id_from_email() {
        let u = User::make("bob@example.com", "Bob", CognitoGroup::Writer, None, None, now());
        assert_eq!(u.id.email(), "bob@example.com");
        assert_eq!(u.id.to_string(), "U#bob@example.com");
        assert_eq!(u.name, "Bob");
        assert_eq!(u.cognito_group, CognitoGroup::Writer);
    }

    /// Default language is Danish (matches OCaml `Language.default`).
    #[test]
    fn make_default_language_is_danish() {
        let u = User::make("a@b.com", "A", CognitoGroup::Reader, None, None, now());
        assert_eq!(u.language, Language::Danish);
    }

    /// Default currency is DKK (matches OCaml `Currency.default`).
    #[test]
    fn make_default_currency_is_dkk() {
        let u = User::make("a@b.com", "A", CognitoGroup::Reader, None, None, now());
        assert_eq!(u.currency, Currency::Dkk);
    }

    /// Explicit language / currency are preserved.
    #[test]
    fn make_explicit_language_and_currency() {
        let u = User::make(
            "a@b.com",
            "A",
            CognitoGroup::Admin,
            Some(Language::English),
            Some(Currency::Eur),
            now(),
        );
        assert_eq!(u.language, Language::English);
        assert_eq!(u.currency, Currency::Eur);
    }

    /// TypedBuilder: fields with defaults can be omitted.
    #[test]
    fn builder_defaults() {
        let u = User::builder()
            .id(UserId::of_email("c@d.com"))
            .name("C".to_owned())
            .cognito_group(CognitoGroup::Reader)
            .created(now())
            .build();
        assert_eq!(u.language, Language::Danish);
        assert_eq!(u.currency, Currency::Dkk);
    }
}
