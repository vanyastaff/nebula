//! Shared structural budgets for value custody boundaries.

use std::cell::Cell;

use nebula_validator::foundation::FieldPath as ValuePath;

use crate::ValidationError;

use super::{
    MAX_EXPRESSION_ENTRIES, MAX_EXPRESSION_TEXT_BYTES, MAX_VALUE_NODES, MAX_VALUE_TEXT_BYTES,
};

#[derive(Debug, Clone, Copy)]
enum BudgetResource {
    DataNodes,
    DataTextBytes,
    ExpressionEntries,
    ExpressionTextBytes,
}

impl BudgetResource {
    const fn name(self) -> &'static str {
        match self {
            Self::DataNodes => "data nodes",
            Self::DataTextBytes => "data text bytes",
            Self::ExpressionEntries => "expression entries",
            Self::ExpressionTextBytes => "expression text bytes",
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ValueBudget {
    data_nodes: Cell<usize>,
    data_text_bytes: Cell<usize>,
    expression_entries: Cell<usize>,
    expression_text_bytes: Cell<usize>,
}

impl ValueBudget {
    pub(super) fn charge_data_node(&self, path: &ValuePath) -> Result<(), ValidationError> {
        self.charge(
            &self.data_nodes,
            1,
            BudgetResource::DataNodes,
            MAX_VALUE_NODES,
            path,
        )
    }

    pub(super) fn charge_data_text(
        &self,
        bytes: usize,
        path: &ValuePath,
    ) -> Result<(), ValidationError> {
        self.charge(
            &self.data_text_bytes,
            bytes,
            BudgetResource::DataTextBytes,
            MAX_VALUE_TEXT_BYTES,
            path,
        )
    }

    pub(super) fn charge_expression(
        &self,
        path: &ValuePath,
        source: &str,
    ) -> Result<(), ValidationError> {
        self.charge_expression_entry(path)?;
        self.charge_expression_text(path.as_str().len().saturating_add(source.len()), path)
    }

    pub(super) fn charge_expression_entry(&self, path: &ValuePath) -> Result<(), ValidationError> {
        self.charge(
            &self.expression_entries,
            1,
            BudgetResource::ExpressionEntries,
            MAX_EXPRESSION_ENTRIES,
            path,
        )
    }

    pub(super) fn charge_expression_text(
        &self,
        bytes: usize,
        path: &ValuePath,
    ) -> Result<(), ValidationError> {
        self.charge(
            &self.expression_text_bytes,
            bytes,
            BudgetResource::ExpressionTextBytes,
            MAX_EXPRESSION_TEXT_BYTES,
            path,
        )
    }

    fn charge(
        &self,
        consumed: &Cell<usize>,
        amount: usize,
        resource: BudgetResource,
        limit: usize,
        path: &ValuePath,
    ) -> Result<(), ValidationError> {
        let updated = consumed.get().saturating_add(amount);
        if updated > limit {
            return Err(limit_exceeded(resource, limit, path));
        }
        consumed.set(updated);
        Ok(())
    }
}

fn limit_exceeded(resource: BudgetResource, limit: usize, path: &ValuePath) -> ValidationError {
    let resource = resource.name();
    tracing::warn!(
        target: "nebula_schema::dos",
        resource,
        limit,
        path = %path,
        "value custody budget exceeded"
    );
    ValidationError::builder("value.limit_exceeded")
        .at(path.clone())
        .param("resource", resource)
        .param("limit", limit)
        .message(format!("value {resource} exceeds the {limit}-unit limit"))
        .build()
}
