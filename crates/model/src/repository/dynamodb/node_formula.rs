//! Formula items live in the node's own partition (`pk = <NodeId>`), indexed by
//! `gsi1pk = "F#HN2#<id>"` so a whole company's formulas are one query.

use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client;

use crate::domain::ids::NodeId;
use crate::domain::node_formula::NodeFormula;
use crate::domain::values::{EnergyType, Purpose};
use crate::errors::RepositoryError;
use crate::repository::dynamodb::codec;

const FORMULA_SK_PREFIX: &str = "formula#";

/// Upsert. `(node, energy_type, purpose)` is the identity, so re-declaring the
/// same triple replaces it rather than conflicting.
pub async fn put_node_formula(
    client: &Client,
    table: &str,
    f: &NodeFormula,
    node_path: &str,
    company_path: &str,
) -> Result<(), RepositoryError> {
    client
        .put_item()
        .table_name(table)
        .set_item(Some(codec::node_formula_to_item(f, node_path, company_path)))
        .send()
        .await
        .map(|_| ())
        .map_err(|e| RepositoryError::Aws(format!("put_node_formula: {e:?}")))
}

/// Delete one formula, reverting that `(energy_type, purpose)` to its default.
pub async fn delete_node_formula(
    client: &Client,
    table: &str,
    node: &NodeId,
    energy_type: EnergyType,
    purpose: Purpose,
) -> Result<(), RepositoryError> {
    client
        .delete_item()
        .table_name(table)
        .key("pk", AttributeValue::S(node.to_string()))
        .key(
            "sk",
            AttributeValue::S(format!("{FORMULA_SK_PREFIX}{energy_type}#{purpose}")),
        )
        .send()
        .await
        .map(|_| ())
        .map_err(|e| RepositoryError::Aws(format!("delete_node_formula: {e:?}")))
}

/// Every formula declared on one node.
pub async fn list_node_formulas(
    client: &Client,
    table: &str,
    node: &NodeId,
) -> Result<Vec<NodeFormula>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .key_condition_expression("#pk = :pk AND begins_with(#sk, :sk)")
        .expression_attribute_names("#pk", "pk")
        .expression_attribute_names("#sk", "sk")
        .expression_attribute_values(":pk", AttributeValue::S(node.to_string()))
        .expression_attribute_values(":sk", AttributeValue::S(FORMULA_SK_PREFIX.to_string()))
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_node_formulas: {e:?}")))?;

    Ok(rows
        .iter()
        .filter_map(|i| codec::node_formula_of_item(i).ok())
        .collect())
}

/// Every formula in a company — the input to `logic::formulas::flatten`.
pub async fn list_company_formulas(
    client: &Client,
    table: &str,
    company_path: &str,
) -> Result<Vec<NodeFormula>, RepositoryError> {
    let rows = client
        .query()
        .table_name(table)
        .index_name("gsi1")
        .key_condition_expression("#pk = :pk")
        .expression_attribute_names("#pk", "gsi1pk")
        .expression_attribute_values(
            ":pk",
            AttributeValue::S(codec::formula_gsi1pk(company_path)),
        )
        .into_paginator()
        .items()
        .send()
        .collect::<Result<Vec<_>, _>>()
        .await
        .map_err(|e| RepositoryError::Aws(format!("list_company_formulas: {e:?}")))?;

    Ok(rows
        .iter()
        .filter_map(|i| codec::node_formula_of_item(i).ok())
        .collect())
}
