# Payments (`djangors-contrib-payments`)

`djangors-contrib-payments` is a provider-agnostic payment integration crate, shipping a
Paystack implementation first. It's built around two hard requirements for anything touching
real money: **amounts are never a float**, and **a webhook redelivery or duplicate initiate call
can never double-process a charge**.

## Amounts: integer minor units, never a float

Every amount in this crate is an `i64` in the currency's minor unit (kobo for NGN, cents for
USD), never `f64` and never `rust_decimal::Decimal`. This is the same convention Paystack's and
Stripe's own APIs use on the wire, so there's no float-precision problem to solve in the first
place: `5000` means ₦50.00, not `50.0`.

## The `PaymentProvider` trait

```rust,compile
use djangors_contrib_payments::{
    InitiateChargeRequest, InitiateChargeResponse, PaymentError, PaymentProvider, RefundResult,
};

async fn charge_example(provider: &dyn PaymentProvider) -> Result<InitiateChargeResponse, PaymentError> {
    let req = InitiateChargeRequest {
        email: "customer@example.com".to_string(),
        amount_minor: 50_000, // NGN 500.00
        currency: "NGN".to_string(),
        reference: "order-1234".to_string(),
        callback_url: Some("https://example.com/callback".to_string()),
    };
    provider.initiate(&req).await
}
```

Every provider implements four operations: `initiate` (start a charge, get a redirect URL back),
`verify` (confirm a transaction's real status directly with the provider; never trust a
client-side "success" redirect alone), `verify_webhook_signature`, and `refund`.

## `PaystackProvider`

```rust,compile
use djangors_contrib_payments::PaystackProvider;

fn make_provider(secret_key: &str) -> PaystackProvider {
    PaystackProvider::new(secret_key.to_string())
}
```

`PaystackProvider::new(secret_key)` talks to the real Paystack API
(`https://api.paystack.co`). Get your secret key from the Paystack dashboard and load it via
`#[derive(Settings)]` (see the [Settings guide](settings.md)) rather than hardcoding it.

## Idempotent transaction recording

`Transaction`'s `reference` column has a real **database-level UNIQUE constraint**, not an
application-level check-then-insert, which would race under a concurrent webhook redelivery or a
double-clicked "pay now" button. `record_charge_initiated` relies on that constraint directly:

```rust,illustrative
use djangors_contrib_payments::{record_charge_initiated, Transaction};
use djangors_orm::djangors_db::Database;

async fn start_checkout(db: &Database, reference: &str) -> Result<Transaction, djangors_contrib_payments::PaymentError> {
    // Called twice with the same `reference` (a retry, a duplicate webhook) returns the SAME
    // row both times - it never creates a second one and never errors.
    record_charge_initiated(db, reference, "paystack", 50_000, "NGN", None).await
}
```

## Webhook handling

`handle_paystack_webhook` replicates the real, production-proven order of operations for
processing an incoming webhook safely:

1. Verify the `x-paystack-signature` header (HMAC-SHA512, constant-time) against the **raw
   request body bytes**, before ever parsing any JSON. A bad signature is rejected without the
   payload being touched at all.
2. Only then parse the JSON body.
3. Require **both** `event == "charge.success"` **and** `data.status == "success"` before
   treating the charge as confirmed. Checking only one of these would be a real security gap.
4. Idempotently record or update the transaction, so a genuine webhook redelivery is a harmless
   no-op, not a duplicate credit.

```rust,illustrative
use djangors_contrib_payments::{handle_paystack_webhook, PaystackProvider};
use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
use djangors_orm::djangors_db::Database;

pub async fn paystack_webhook_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("no database in request state".into()))?;
    let provider = req
        .state::<PaystackProvider>()
        .ok_or_else(|| DjangorsError::Internal("no PaystackProvider in request state".into()))?;

    let signature = req
        .header("x-paystack-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| DjangorsError::Unauthorized("missing signature header".into()))?;
    let body = req.body_bytes().await;

    handle_paystack_webhook(provider, db, body, signature)
        .await
        .map_err(|e| DjangorsError::Unauthorized(e.to_string()))?;

    Ok(Response::text(StatusCode::OK, "ok"))
}
```

## Not yet included

Stripe/Anchor/Moniepoint providers, HTTP route wiring into any example app (this crate exposes
functions your own handler calls, matching how `djangors-auth` exposes backends rather than
routes; not an oversight), and generic idempotency-key middleware for arbitrary POST APIs (a
separate, broader concern than this crate's own reference-based idempotency) are all deliberately
out of scope for this first slice.
