//! §1.1, §1.5 and §1.6 — lineage, association kinds, merge and redirect. **Plan 08's module.**
//!
//! Only [`store::LocationInput`] exists so far: plan 07's `ScanStore` seam names it and R1
//! forbids a second copy, so the type lands with the plan that consumes it and the writer stays
//! with the plan that owns the transaction.

pub mod store;
