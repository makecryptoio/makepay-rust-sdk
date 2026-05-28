# MakePay Rust SDK

Official async Rust SDK for MakePay payments, donations, invoices,
bookkeeping, subscriptions, POS terminals, products, Simple Shop storefronts,
branding, settings, and webhooks.

Public source: `https://github.com/makecryptoio/makepay-rust-sdk`

## Install

```toml
[dependencies]
makepay = "0.3"
```

The SDK uses `reqwest` with Rustls TLS by default and returns
`serde_json::Value` envelopes so new API response fields can ship without
breaking Rust integrations.

## Configure

Create a MakePay API key in MakeCrypto and keep the secret on your server only.

```rust
use makepay::MakePayClient;

let makepay = MakePayClient::new(
    std::env::var("MAKEPAY_KEY_ID")?,
    std::env::var("MAKEPAY_KEY_SECRET")?,
)?;
```

## Payment Links

```rust
use makepay::{CreatePaymentLinkOptions, MakePayClient};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), makepay::MakePayError> {
    let makepay = MakePayClient::new(
        std::env::var("MAKEPAY_KEY_ID").unwrap(),
        std::env::var("MAKEPAY_KEY_SECRET").unwrap(),
    )?;

    let response = makepay
        .create_payment_link(&json!({
            "title": "Order #1042",
            "description": "Checkout for order #1042",
            "amount": "129.99",
            "currency": "USDT",
            "orderId": "order_1042",
            "customerEmail": "buyer@example.com",
            "returnUrl": "https://merchant.example/orders/1042",
            "successUrl": "https://merchant.example/orders/1042/success",
            "failureUrl": "https://merchant.example/orders/1042/pay",
            "expirationTime": "12h"
        }))
        .await?;

    println!("{}", response["paymentLink"]["publicUrl"]);

    makepay
        .create_payment_link_with_options(
            &json!({ "amount": "49", "title": "Email invoice" }),
            CreatePaymentLinkOptions {
                send_payment_request_email: true,
                ..Default::default()
            },
        )
        .await?;

    makepay.list_payment_links(&[]).await?;
    makepay.get_payment_link("PAYMENT_LINK_UID").await?;
    makepay
        .send_payment_request_email("PAYMENT_LINK_UID", Some("buyer@example.com"))
        .await?;

    Ok(())
}
```

## Donations

Donation pages are flexible-amount payment links with public donation slugs.

```rust
use makepay::{build_embedded_donation_url, build_hosted_donation_url};
use serde_json::json;

let donation = makepay
    .create_donation_link(&json!({
        "title": "Spring campaign",
        "description": "Support the 2026 spring fundraiser.",
        "defaultAmountUsd": "25",
        "minimumAmountUsd": "5",
        "donationSlug": "spring-campaign"
    }))
    .await?;

println!("{}", donation["paymentLink"]["publicUrl"]);

makepay.list_donation_links().await?;
makepay.get_donation_link("DONATION_UID").await?;
makepay
    .update_donation_link("DONATION_UID", &json!({ "status": "paused" }))
    .await?;

let hosted = build_hosted_donation_url("spring-campaign", None)?;
let embedded = build_embedded_donation_url(
    "spring-campaign",
    None,
    Some("https://merchant.example"),
)?;
```

## Anonymous Payment Links

Anonymous links do not use a MakePay API key. They require an explicit
settlement route because MakePay cannot read merchant wallet settings.

```rust
use makepay::create_anonymous_payment_link;
use serde_json::json;

let response = create_anonymous_payment_link(&json!({
    "amount": "25",
    "settlement": {
        "currency": "USDT",
        "priorities": [
            {
                "chain": "ETH",
                "address": "0xYourSettlementWallet",
                "asset": "ETH.USDT-0xdAC17F958D2ee523a2206206994597C13D831ec7"
            }
        ]
    },
    "title": "Invoice #1042",
    "webhookUrl": "https://merchant.example/webhooks/makepay"
}))
.await?;
```

## Checkout URLs And Embeds

Use hosted checkout URLs for redirects, or embedded URLs/snippets when your
frontend keeps the shopper on the merchant page.

```rust
use makepay::{build_embed_button_html, build_embedded_checkout_url, build_hosted_checkout_url};

let payment_uid = "PAYMENT_LINK_UID";
let hosted_url = build_hosted_checkout_url(payment_uid, None)?;
let embed_url = build_embedded_checkout_url(
    payment_uid,
    Some("https://makepay.io"),
    Some("https://merchant.example"),
)?;
let button_html = build_embed_button_html(payment_uid, None, Some("Pay with crypto"))?;
```

## Customers And Subscriptions

```rust
use serde_json::json;

makepay
    .upsert_customer(&json!({
        "email": "buyer@example.com",
        "name": "Buyer Example",
        "clientId": "crm_123"
    }))
    .await?;

makepay
    .create_customer_portal_with_payload(
        "CUSTOMER_ID",
        &json!({ "returnUrl": "https://merchant.example/account" }),
    )
    .await?;

makepay
    .create_subscription(&json!({
        "amountUsd": "29",
        "customerEmail": "buyer@example.com",
        "label": "Monthly plan",
        "billingIntervalUnit": "month",
        "billingIntervalCount": 1
    }))
    .await?;
```

## POS, Products, And Simple Shop

```rust
use serde_json::json;

let terminal = makepay
    .create_pos_terminal(&json!({
        "name": "Front counter",
        "pin": "1234",
        "catalogEnabled": true
    }))
    .await?;

makepay.list_pos_terminals().await?;
makepay.get_pos_terminal(terminal["terminal"]["uid"].as_str().unwrap_or("")).await?;

makepay
    .create_product(&json!({
        "name": "Digital guide",
        "productType": "digital",
        "basePriceUsd": "19",
        "shopSlug": "digital-guide"
    }))
    .await?;

makepay
    .create_product_download(
        "PRODUCT_UID",
        &json!({
            "fileName": "guide.pdf",
            "contentType": "application/pdf",
            "url": "https://merchant.example/downloads/guide.pdf"
        }),
    )
    .await?;

makepay
    .update_shop(&json!({
        "slug": "merchant-shop",
        "displayCurrency": "USD",
        "checkoutMode": "hosted"
    }))
    .await?;

makepay.update_shop_domain(Some("shop.merchant.example")).await?;
makepay.refresh_shop_domain(None).await?;
makepay
    .create_shop_coupon(&json!({
        "code": "SPRING10",
        "discountType": "percent",
        "value": "10"
    }))
    .await?;
makepay.list_shop_orders(&[("status", "paid"), ("limit", "25")]).await?;
```

## Invoices And Bookkeeping

Bookkeeping APIs manage merchant invoices, expenses, supporting documents, OCR,
and reconciliation links.

```rust
use serde_json::json;

let created = makepay
    .create_bookkeeping_invoice(&json!({
        "title": "Invoice #1042",
        "currency": "USD",
        "issueDate": "2026-05-15",
        "dueDate": "2026-05-30",
        "counterparty": {
            "name": "Buyer Example",
            "email": "buyer@example.com",
            "clientId": "crm_123"
        },
        "lineItems": [
            {
                "description": "Implementation services",
                "quantity": "1",
                "unitAmount": "500",
                "taxAmount": "0"
            }
        ],
        "metadata": { "orderId": "order_1042" }
    }))
    .await?;

makepay
    .create_bookkeeping_invoice_payment_link_with_options(
        "INVOICE_UID",
        &json!({ "sendPaymentRequestEmail": true }),
    )
    .await?;

makepay.list_bookkeeping_invoices().await?;
makepay.get_bookkeeping_invoice("INVOICE_UID").await?;
makepay
    .update_bookkeeping_invoice("INVOICE_UID", &json!({ "status": "open" }))
    .await?;

makepay
    .create_bookkeeping_expense(&json!({
        "title": "Hosting",
        "amount": "49",
        "currency": "USD",
        "incurredOn": "2026-05-15",
        "category": "Infrastructure",
        "counterparty": { "name": "Vendor Example", "type": "vendor" }
    }))
    .await?;

makepay
    .create_bookkeeping_reconciliation(&json!({
        "invoiceId": "INVOICE_UID",
        "paymentSessionId": "PAYMENT_SESSION_ID",
        "linkType": "payment"
    }))
    .await?;
```

Document uploads use `reqwest::multipart::Form`.

```rust
let form = reqwest::multipart::Form::new()
    .text("documentType", "receipt")
    .text("expenseId", "EXPENSE_UID")
    .file("file", "receipt.pdf")
    .await?;

makepay.upload_bookkeeping_document(form).await?;
makepay.list_bookkeeping_documents().await?;
makepay.get_bookkeeping_document_download_url("DOCUMENT_UID").await?;
makepay.run_bookkeeping_document_ocr("DOCUMENT_UID").await?;
makepay.get_bookkeeping_summary().await?;
```

## Branding, Settings, And Operational APIs

```rust
use serde_json::json;

makepay
    .update_branding(&json!({
        "brandName": "Merchant",
        "supportEmail": "support@merchant.example",
        "brandingBrandColor": "#111827",
        "brandingAccentColor": "#14b8a6",
        "paymentLinkTheme": "system",
        "paymentLinkDomain": "pay.merchant.example",
        "emailSendingDomain": "mail.merchant.example"
    }))
    .await?;

makepay.refresh_branding_domains(Some("all")).await?;
makepay.get_settings().await?;
makepay
    .update_settings(&json!({
        "callbackUrl": "https://merchant.example/webhooks/makepay"
    }))
    .await?;
makepay.list_destination_assets().await?;
makepay.list_webhook_requests(&[("limit", "25")]).await?;
```

## Verify Webhooks

Read the raw request body before parsing JSON.

```rust
use makepay::parse_webhook;
use serde_json::Value;

let raw_body = br#"{"event":{"type":"status_changed"}}"#;
let signature = request_headers
    .get("x-makepay-signature")
    .and_then(|value| value.to_str().ok());

let event: Value = parse_webhook(
    raw_body,
    signature,
    std::env::var("MAKEPAY_WEBHOOK_SECRET")?.as_str(),
    None,
)?;
```

Use `verify_webhook` when you only need a boolean result.

## Errors

API calls return `MakePayError`. API errors include the HTTP status and decoded
response body.

```rust
match makepay.get_payment_link("PAYMENT_LINK_UID").await {
    Ok(link) => println!("{link:#}"),
    Err(error) => {
        if let Some(status) = error.status() {
            eprintln!("MakePay returned {status}");
        }
    }
}
```

## Method Coverage

| Area            | SDK methods                                                                                                                              |
| --------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Payment links   | `create_payment_link`, `list_payment_links`, `get_payment_link`, `update_payment_link`, `send_payment_request_email`                     |
| Donations       | `create_donation_link`, `list_donation_links`, `get_donation_link`, `update_donation_link`                                               |
| Anonymous links | `create_anonymous_payment_link`                                                                                                          |
| Checkout        | hosted, embedded, modal, button, and iframe helpers, plus donation URL helpers                                                            |
| Customers       | `list_customers`, `upsert_customer`, `create_customer_portal`                                                                            |
| Subscriptions   | `list_subscriptions`, `create_subscription`                                                                                              |
| POS terminals   | `list_pos_terminals`, `create_pos_terminal`, `get_pos_terminal`, `update_pos_terminal`                                                    |
| Products        | `list_products`, `create_product`, `get_product`, `update_product`, `list_product_downloads`, `create_product_download`                  |
| Simple Shop     | `get_shop`, `update_shop`, `get_shop_builder`, `update_shop_builder`, domain, coupon, and order helpers                                  |
| Bookkeeping     | summary, invoice, expense, document upload/OCR, and reconciliation methods                                                               |
| Branding        | `get_branding`, `update_branding`, `refresh_branding_domains`                                                                            |
| Operations      | `get_settings`, `update_settings`, `list_destination_assets`, `list_webhook_requests`                                                     |
| Webhooks        | `verify_webhook`, `parse_webhook`                                                                                                        |

## Source Layout

The canonical monorepo source lives in `apps/plugins/rust-sdk`. The public
repository at `https://github.com/makecryptoio/makepay-rust-sdk` mirrors only
the SDK files so developers can install or inspect it without the full
MakeCrypto workspace.
