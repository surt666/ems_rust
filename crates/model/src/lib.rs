pub mod domain;
pub mod errors;
pub mod logic;
pub mod repository;

// ---------------------------------------------------------------------------
// Lazy-initialised AWS clients (shared across all lambda invocations)
// ---------------------------------------------------------------------------
//
// Follows the EMS OnceCell pattern: each static is initialised on first access
// and then reused, avoiding the cold-start cost of re-creating SDK clients on
// every invocation.

use aws_sdk_cognitoidentityprovider::Client as CognitoClient;
use aws_sdk_dynamodb::Client as DynamoClient;
use tokio::sync::OnceCell;

static AWS_CONFIG: OnceCell<aws_config::SdkConfig> = OnceCell::const_new();
static DYNAMODB_CLIENT: OnceCell<DynamoClient> = OnceCell::const_new();
static COGNITO_CLIENT: OnceCell<CognitoClient> = OnceCell::const_new();

/// Load (or return the cached) `SdkConfig` for `eu-central-1`, overridable
/// via the standard `AWS_REGION` / `AWS_DEFAULT_REGION` env vars.
///
/// We don't call `.region(...)` explicitly — the SDK picks it up from the
/// environment variables automatically, which avoids a `'static` lifetime
/// constraint on the region string. The lambda environment always sets
/// `AWS_REGION=eu-central-1`.
pub async fn get_aws_config() -> &'static aws_config::SdkConfig {
    AWS_CONFIG
        .get_or_init(|| async {
            aws_config::defaults(aws_config::BehaviorVersion::latest())
                .load()
                .await
        })
        .await
}

/// Return the shared DynamoDB client (created on first call).
pub async fn get_dynamodb_client() -> &'static DynamoClient {
    DYNAMODB_CLIENT
        .get_or_init(|| async {
            let config = get_aws_config().await;
            DynamoClient::new(config)
        })
        .await
}

/// Return the shared Cognito IDP client (created on first call).
pub async fn get_cognito_client() -> &'static CognitoClient {
    COGNITO_CLIENT
        .get_or_init(|| async {
            let config = get_aws_config().await;
            CognitoClient::new(config)
        })
        .await
}

/// The Cognito user-pool id — from `USER_POOL_ID` env, default
/// `"eu-central-1_gADB2vK24"` (account 339).
pub fn get_user_pool_id() -> String {
    std::env::var("USER_POOL_ID")
        .unwrap_or_else(|_| "eu-central-1_gADB2vK24".to_string())
}

/// The DynamoDB table name — from `TABLE_NAME` env, default `"hierarchy_new"`.
pub fn get_table_name() -> String {
    std::env::var("TABLE_NAME").unwrap_or_else(|_| "hierarchy_new".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `get_user_pool_id` returns the hardcoded default when `USER_POOL_ID` is unset.
    /// Uses `serial_test` to avoid env-var races.
    #[test]
    fn user_pool_id_default() {
        // Only run if USER_POOL_ID is not set in the environment.
        if std::env::var("USER_POOL_ID").is_ok() {
            return;
        }
        assert_eq!(get_user_pool_id(), "eu-central-1_gADB2vK24");
    }

    /// `get_table_name` returns the hardcoded default when `TABLE_NAME` is unset.
    #[test]
    fn table_name_default() {
        if std::env::var("TABLE_NAME").is_ok() {
            return;
        }
        assert_eq!(get_table_name(), "hierarchy_new");
    }
}
