//! The materialised coefficient matrix — what the Glue roll-up job reads.
//!
//! Replaced wholesale per company: it is derived data, so a delete-then-write is
//! simpler and safer than diffing, and it cannot leave orphaned rows behind when
//! a formula or a sensor disappears.

use aws_sdk_dynamodb::types::{AttributeValue, DeleteRequest, PutRequest, WriteRequest};
use aws_sdk_dynamodb::Client;

use crate::errors::RepositoryError;
use crate::logic::formulas::MatrixRow;
use crate::repository::dynamodb::codec;

const WEIGHT_SK_PREFIX: &str = "weight#";

/// DynamoDB caps a `BatchWriteItem` at 25 requests.
const BATCH: usize = 25;

/// Delete every weight row for the company, then write the new matrix.
pub async fn replace_company_matrix(
    client: &Client,
    table: &str,
    company_path: &str,
    matrix: &[MatrixRow],
) -> Result<(), RepositoryError> {
    let existing = list_weight_keys(client, table, company_path).await?;

    let deletes = existing.into_iter().map(|(pk, sk)| {
        WriteRequest::builder()
            .delete_request(
                DeleteRequest::builder()
                    .key("pk", AttributeValue::S(pk))
                    .key("sk", AttributeValue::S(sk))
                    .build()
                    .expect("delete key is complete"),
            )
            .build()
    });

    let puts = matrix.iter().map(|r| {
        WriteRequest::builder()
            .put_request(
                PutRequest::builder()
                    .set_item(Some(codec::matrix_row_to_item(r, company_path)))
                    .build()
                    .expect("put item is complete"),
            )
            .build()
    });

    let requests: Vec<WriteRequest> = deletes.chain(puts).collect();
    for chunk in requests.chunks(BATCH) {
        client
            .batch_write_item()
            .request_items(table, chunk.to_vec())
            .send()
            .await
            .map_err(|e| RepositoryError::Aws(format!("replace_company_matrix: {e:?}")))?;
    }
    Ok(())
}

/// Every `(pk, sk)` currently in the company's weight partition.
async fn list_weight_keys(
    client: &Client,
    table: &str,
    company_path: &str,
) -> Result<Vec<(String, String)>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_values(
            ":pk",
            AttributeValue::S(codec::weight_gsi1pk(company_path)),
        )
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_weight_keys: {e:?}")))?;

    Ok(rows
        .iter()
        .filter_map(|i| {
            let pk = i.get("pk")?.as_s().ok()?.clone();
            let sk = i.get("sk")?.as_s().ok()?.clone();
            sk.starts_with(WEIGHT_SK_PREFIX).then_some((pk, sk))
        })
        .collect())
}
