#![deny(missing_docs)]
//! Payment provider integration for Djangors. Ships a `PaymentProvider` trait and a Paystack
//! implementation. Amounts are always integer minor units (kobo/cents) - never a float, never
//! `rust_decimal::Decimal` (djangors-orm's derive(Model) currently cannot generate INSERT/UPDATE
//! code for Decimal fields, and integer minor units is the correct money representation anyway -
//! it's the same convention Paystack's and Stripe's own APIs use on the wire).

mod paystack;
mod provider;
mod transaction;

pub use paystack::PaystackProvider;
pub use provider::{
    InitiateChargeRequest, InitiateChargeResponse, PaymentError, PaymentProvider, RefundResult,
    TransactionStatus,
};
pub use transaction::{
    find_by_reference, handle_paystack_webhook, mark_transaction_status, record_charge_initiated,
    Transaction,
};
