use masonwing_contracts::{AdapterQualification, TenantId};
use masonwing_kernel::ports::{AdapterBoundary, BudgetPort, PortError};

#[derive(Debug, Default)]
pub struct BudgetLedgerAdapter;

impl AdapterBoundary for BudgetLedgerAdapter {
    fn adapter_name(&self) -> &'static str {
        "budget"
    }

    fn qualification(&self) -> AdapterQualification {
        AdapterQualification::Unqualified
    }
}

impl BudgetPort for BudgetLedgerAdapter {
    fn reserve_microunits(
        &self,
        _tenant_id: &TenantId,
        _upper_bound: u64,
    ) -> Result<(), PortError> {
        Err(PortError::NotQualified {
            adapter: self.adapter_name(),
        })
    }
}
