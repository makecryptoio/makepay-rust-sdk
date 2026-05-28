use makepay::MakePayClient;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), makepay::MakePayError> {
    let makepay = MakePayClient::new(
        std::env::var("MAKEPAY_KEY_ID").expect("MAKEPAY_KEY_ID is required"),
        std::env::var("MAKEPAY_KEY_SECRET").expect("MAKEPAY_KEY_SECRET is required"),
    )?;

    let response = makepay
        .create_payment_link(&json!({
            "title": "Order #1042",
            "amount": "129.99",
            "currency": "USDT",
            "orderId": "order_1042",
            "customerEmail": "buyer@example.com"
        }))
        .await?;

    println!("{response:#}");
    Ok(())
}
