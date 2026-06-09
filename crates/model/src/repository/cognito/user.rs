//! Cognito user-pool operations.
//!
//! - `provision_cognito_user`: generates a strong random password, calls
//!   `AdminCreateUser` (Cognito emails the temp password), adds the user to
//!   the group, then `AdminSetUserPassword(Permanent=true)` so the account
//!   goes straight to `CONFIRMED` — no forced-change-password flow.
//! - `delete_cognito_user`: `AdminDeleteUser`; idempotent
//!   (`UserNotFoundException` → Ok).

use aws_sdk_cognitoidentityprovider::{
    types::AttributeType, Client,
};
use rand::Rng;

use crate::{domain::values::CognitoGroup, errors::RepositoryError};

// ---------------------------------------------------------------------------
// Password generation
// ---------------------------------------------------------------------------

/// Generate a 16-character random password that satisfies Cognito's default
/// policy: at least one uppercase letter, one lowercase letter, one digit,
/// and one symbol.  The password is never logged.
fn generate_password() -> String {
    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const DIGIT: &[u8] = b"0123456789";
    const SYMBOL: &[u8] = b"!@#$%^&*()-_=+[]{}|;:,.<>?";
    const ALL: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()-_=+[]{}|;:,.<>?";

    let mut rng = rand::thread_rng();

    // Guarantee at least one of each required class (indices 0..3).
    let mut chars: Vec<u8> = vec![
        UPPER[rng.gen_range(0..UPPER.len())],
        LOWER[rng.gen_range(0..LOWER.len())],
        DIGIT[rng.gen_range(0..DIGIT.len())],
        SYMBOL[rng.gen_range(0..SYMBOL.len())],
    ];

    // Fill the remaining 12 positions with any character from the full set.
    for _ in 0..12 {
        chars.push(ALL[rng.gen_range(0..ALL.len())]);
    }

    // Shuffle so the mandatory chars are not always in positions 0-3.
    use rand::seq::SliceRandom;
    chars.shuffle(&mut rng);

    String::from_utf8(chars).expect("password chars are all ASCII")
}

// ---------------------------------------------------------------------------
// set_user_password (private helper)
// ---------------------------------------------------------------------------

async fn set_user_password(
    client: &Client,
    pool_id: &str,
    email: &str,
    password: &str,
    permanent: bool,
) -> Result<(), RepositoryError> {
    client
        .admin_set_user_password()
        .user_pool_id(pool_id)
        .username(email)
        .password(password)
        .permanent(permanent)
        .send()
        .await
        .map_err(|e| {
            RepositoryError::Aws(format!("AdminSetUserPassword({email}): {e}"))
        })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// provision_cognito_user (public entry-point)
// ---------------------------------------------------------------------------

/// Provision a Cognito user in four steps:
///
/// 1. Generate a strong random password (16 chars, upper+lower+digit+symbol).
/// 2. `AdminCreateUser` with `TemporaryPassword` set and no `MessageAction::Suppress`
///    — Cognito emails the password to the user.
///    `UsernameExistsException` is treated as idempotent (Ok).
/// 3. `AdminAddUserToGroup` (capitalised group name).
/// 4. `AdminSetUserPassword(Permanent=true)` with the same password — moves
///    the account from `FORCE_CHANGE_PASSWORD` to `CONFIRMED` so the emailed
///    password can be used directly without a forced change.
pub async fn provision_cognito_user(
    client: &Client,
    pool_id: &str,
    email: &str,
    _name: &str,
    group: CognitoGroup,
) -> Result<(), RepositoryError> {
    let password = generate_password();

    // Step 1+2: AdminCreateUser with TemporaryPassword (no Suppress).
    let email_attr = AttributeType::builder()
        .name("email")
        .value(email)
        .build()
        .map_err(|e| RepositoryError::Aws(format!("build email attr: {e}")))?;

    let verified_attr = AttributeType::builder()
        .name("email_verified")
        .value("true")
        .build()
        .map_err(|e| RepositoryError::Aws(format!("build email_verified attr: {e}")))?;

    let create_result = client
        .admin_create_user()
        .user_pool_id(pool_id)
        .username(email)
        .user_attributes(email_attr)
        .user_attributes(verified_attr)
        .temporary_password(&password)
        .send()
        .await;

    match create_result {
        Ok(_) => {}
        Err(e) => {
            if e.as_service_error()
                .map(|svc| svc.is_username_exists_exception())
                .unwrap_or(false)
            {
                // Already exists — skip the rest of provisioning idempotently.
                return Ok(());
            }
            return Err(RepositoryError::Aws(format!(
                "AdminCreateUser({email}): {e}"
            )));
        }
    }

    // Step 3: AdminAddUserToGroup.
    client
        .admin_add_user_to_group()
        .user_pool_id(pool_id)
        .username(email)
        .group_name(group.to_string())
        .send()
        .await
        .map_err(|e| {
            RepositoryError::Aws(format!("AdminAddUserToGroup({email}, {group}): {e}"))
        })?;

    // Step 4: AdminSetUserPassword(Permanent=true) → CONFIRMED.
    set_user_password(client, pool_id, email, &password, true).await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// delete_cognito_user
// ---------------------------------------------------------------------------

/// Delete a Cognito user.
///
/// Idempotent: if the user does not exist (`UserNotFoundException`) the error
/// is swallowed and `Ok(())` is returned, matching the Go cognito-sync
/// `deleteUser` function.
pub async fn delete_cognito_user(
    client: &Client,
    pool_id: &str,
    email: &str,
) -> Result<(), RepositoryError> {
    let result = client
        .admin_delete_user()
        .user_pool_id(pool_id)
        .username(email)
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // Typed check via as_service_error(): UserNotFoundException → idempotent.
            if e.as_service_error()
                .map(|svc| svc.is_user_not_found_exception())
                .unwrap_or(false)
            {
                Ok(())
            } else {
                Err(RepositoryError::Aws(format!(
                    "AdminDeleteUser({email}): {e}"
                )))
            }
        }
    }
}
