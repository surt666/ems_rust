//! Cognito user-pool operations.
//!
//! Matches the behaviour of `infra/hierarchy/lambda/cognito-sync/main.go`:
//! - `create_cognito_user`: `AdminCreateUser` with invite e-mail; idempotent
//!   (`UsernameExistsException` → Ok).
//! - `add_user_to_group`: `AdminAddUserToGroup`.
//! - `delete_cognito_user`: `AdminDeleteUser`; idempotent
//!   (`UserNotFoundException` → Ok).

use aws_sdk_cognitoidentityprovider::{
    types::AttributeType, Client,
};

use crate::{domain::values::CognitoGroup, errors::RepositoryError};

/// Create a Cognito user with `username = email`.
///
/// Attributes written: `email` + `email_verified = "true"`.
/// No `MessageAction::Suppress` — invitation e-mail is sent, matching the
/// Go cognito-sync default (Go does not pass `MessageAction`).
///
/// Idempotent: if the user already exists (`UsernameExistsException`) the
/// error is swallowed and `Ok(())` is returned.
pub async fn create_cognito_user(
    client: &Client,
    pool_id: &str,
    email: &str,
    _name: &str,
) -> Result<(), RepositoryError> {
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

    let result = client
        .admin_create_user()
        .user_pool_id(pool_id)
        .username(email)
        .user_attributes(email_attr)
        .user_attributes(verified_attr)
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            // Typed check via as_service_error(): UsernameExistsException → idempotent.
            if e.as_service_error()
                .map(|svc| svc.is_username_exists_exception())
                .unwrap_or(false)
            {
                Ok(())
            } else {
                Err(RepositoryError::Aws(format!(
                    "AdminCreateUser({email}): {e}"
                )))
            }
        }
    }
}

/// Add a user to a Cognito group.
///
/// Uses `CognitoGroup::to_string()` which yields the capitalised form
/// (`"Admin"` / `"Writer"` / `"Reader"`), matching the pool group names.
pub async fn add_user_to_group(
    client: &Client,
    pool_id: &str,
    email: &str,
    group: CognitoGroup,
) -> Result<(), RepositoryError> {
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

    Ok(())
}

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
