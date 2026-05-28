use std::time::{SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use reqwest::{
    header::{ACCEPT, CONTENT_TYPE, USER_AGENT},
    Method, StatusCode,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;
use url::{form_urlencoded::byte_serialize, Url};

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_BASE_URL: &str = "https://www.makecrypto.io";
pub const DEFAULT_CHECKOUT_BASE_URL: &str = "https://makepay.io";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum MakePayError {
    #[error("{message}")]
    Api {
        status: StatusCode,
        message: String,
        response_body: Value,
    },
    #[error("{0}")]
    InvalidInput(String),
    #[error("Invalid MakePay webhook JSON body: {0}")]
    InvalidWebhookJson(serde_json::Error),
    #[error("Invalid MakePay webhook signature.")]
    InvalidWebhookSignature,
    #[error("MakePay API HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("MakePay JSON handling failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid MakePay URL: {0}")]
    Url(#[from] url::ParseError),
}

impl MakePayError {
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn response_body(&self) -> Option<&Value> {
        match self {
            Self::Api { response_body, .. } => Some(response_body),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PaymentLinkStatus {
    Active,
    Paused,
    Archived,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CreatePaymentLinkOptions {
    pub status: PaymentLinkStatus,
    #[serde(rename = "sendPaymentRequestEmail")]
    pub send_payment_request_email: bool,
}

impl Default for CreatePaymentLinkOptions {
    fn default() -> Self {
        Self {
            status: PaymentLinkStatus::Active,
            send_payment_request_email: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MakePayPublicRequestOptions {
    pub base_url: String,
    pub http: reqwest::Client,
}

impl Default for MakePayPublicRequestOptions {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            http: reqwest::Client::default(),
        }
    }
}

impl MakePayPublicRequestOptions {
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WebhookVerificationOptions {
    pub tolerance_seconds: u64,
}

impl Default for WebhookVerificationOptions {
    fn default() -> Self {
        Self {
            tolerance_seconds: 300,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MakePayClient {
    base_url: String,
    checkout_base_url: String,
    key_id: String,
    key_secret: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone)]
pub struct MakePayClientBuilder {
    base_url: String,
    checkout_base_url: String,
    key_id: Option<String>,
    key_secret: Option<String>,
    http: Option<reqwest::Client>,
}

impl Default for MakePayClientBuilder {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            checkout_base_url: DEFAULT_CHECKOUT_BASE_URL.to_owned(),
            key_id: None,
            key_secret: None,
            http: None,
        }
    }
}

impl MakePayClientBuilder {
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn checkout_base_url(mut self, checkout_base_url: impl Into<String>) -> Self {
        self.checkout_base_url = checkout_base_url.into();
        self
    }

    pub fn key_id(mut self, key_id: impl Into<String>) -> Self {
        self.key_id = Some(key_id.into());
        self
    }

    pub fn key_secret(mut self, key_secret: impl Into<String>) -> Self {
        self.key_secret = Some(key_secret.into());
        self
    }

    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    pub fn build(self) -> Result<MakePayClient, MakePayError> {
        let base_url = normalize_base_url(&self.base_url);
        let checkout_base_url = normalize_base_url(&self.checkout_base_url);
        assert_non_empty(&base_url, "MakePay base URL is required.")?;
        assert_non_empty(&checkout_base_url, "MakePay checkout base URL is required.")?;
        Url::parse(&base_url)?;
        Url::parse(&checkout_base_url)?;

        let key_id = self.key_id.unwrap_or_default();
        let key_secret = self.key_secret.unwrap_or_default();
        assert_non_empty(&key_id, "MakePay API key ID is required.")?;
        assert_non_empty(&key_secret, "MakePay API key secret is required.")?;

        Ok(MakePayClient {
            base_url,
            checkout_base_url,
            key_id,
            key_secret,
            http: self.http.unwrap_or_default(),
        })
    }
}

impl MakePayClient {
    pub fn new(
        key_id: impl Into<String>,
        key_secret: impl Into<String>,
    ) -> Result<Self, MakePayError> {
        Self::builder()
            .key_id(key_id)
            .key_secret(key_secret)
            .build()
    }

    pub fn builder() -> MakePayClientBuilder {
        MakePayClientBuilder::default()
    }

    pub async fn create_payment_link<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.create_payment_link_with_options(payload, CreatePaymentLinkOptions::default())
            .await
    }

    pub async fn create_payment_link_with_options<T>(
        &self,
        payload: &T,
        options: CreatePaymentLinkOptions,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        let body = json!({
            "status": options.status,
            "sendPaymentRequestEmail": options.send_payment_request_email,
            "payload": serde_json::to_value(payload)?,
        });

        self.request(
            Method::POST,
            "/api/partner/v1/makepay/payment-links",
            Some(body),
            &[],
        )
        .await
    }

    pub async fn list_payment_links(&self, query: &[(&str, &str)]) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/payment-links",
            None,
            query,
        )
        .await
    }

    pub async fn get_payment_link(&self, uid: &str) -> Result<Value, MakePayError> {
        assert_non_empty(uid, "Payment link UID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/payment-links/{}",
                encode_path_segment(uid)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_payment_link<T>(
        &self,
        uid: &str,
        updates: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(uid, "Payment link UID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/payment-links/{}",
                encode_path_segment(uid)
            ),
            Some(serde_json::to_value(updates)?),
            &[],
        )
        .await
    }

    pub async fn update_payment_link_status(
        &self,
        uid: &str,
        status: PaymentLinkStatus,
    ) -> Result<Value, MakePayError> {
        self.update_payment_link(uid, &json!({ "status": status }))
            .await
    }

    pub async fn send_payment_request_email(
        &self,
        uid: &str,
        email: Option<&str>,
    ) -> Result<Value, MakePayError> {
        assert_non_empty(uid, "Payment link UID is required.")?;
        let body = match email {
            Some(email) => json!({ "email": email }),
            None => json!({}),
        };

        self.request(
            Method::POST,
            &format!(
                "/api/partner/v1/makepay/payment-links/{}/send-request-email",
                encode_path_segment(uid)
            ),
            Some(body),
            &[],
        )
        .await
    }

    pub async fn get_settings(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/settings", None, &[])
            .await
    }

    pub async fn update_settings<T>(&self, settings: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::PUT,
            "/api/partner/v1/makepay/settings",
            Some(serde_json::to_value(settings)?),
            &[],
        )
        .await
    }

    pub fn hosted_checkout_url(&self, uid: &str) -> Result<String, MakePayError> {
        build_hosted_checkout_url(uid, Some(&self.checkout_base_url))
    }

    pub fn embedded_checkout_url(
        &self,
        uid: &str,
        parent_origin: Option<&str>,
    ) -> Result<String, MakePayError> {
        build_embedded_checkout_url(uid, Some(&self.checkout_base_url), parent_origin)
    }

    pub fn modal_script_url(&self) -> Result<String, MakePayError> {
        build_modal_script_url(Some(&self.checkout_base_url))
    }

    pub fn embed_button_html(
        &self,
        uid: &str,
        button_label: Option<&str>,
    ) -> Result<String, MakePayError> {
        build_embed_button_html(uid, Some(&self.checkout_base_url), button_label)
    }

    pub fn iframe_html(
        &self,
        uid: &str,
        iframe_title: Option<&str>,
        parent_origin: Option<&str>,
    ) -> Result<String, MakePayError> {
        build_iframe_html(
            uid,
            Some(&self.checkout_base_url),
            iframe_title,
            parent_origin,
        )
    }

    pub fn hosted_donation_url(&self, donation_slug: &str) -> Result<String, MakePayError> {
        build_hosted_donation_url(donation_slug, Some(&self.checkout_base_url))
    }

    pub fn embedded_donation_url(
        &self,
        donation_slug: &str,
        parent_origin: Option<&str>,
    ) -> Result<String, MakePayError> {
        build_embedded_donation_url(donation_slug, Some(&self.checkout_base_url), parent_origin)
    }

    pub async fn create_donation_link<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.create_donation_link_with_options(payload, CreatePaymentLinkOptions::default())
            .await
    }

    pub async fn create_donation_link_with_options<T>(
        &self,
        payload: &T,
        options: CreatePaymentLinkOptions,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        let body = json!({
            "status": options.status,
            "sendPaymentRequestEmail": options.send_payment_request_email,
            "payload": donation_payload_value(payload)?,
        });

        self.request(
            Method::POST,
            "/api/partner/v1/makepay/donations",
            Some(body),
            &[],
        )
        .await
    }

    pub async fn list_donation_links(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/donations", None, &[])
            .await
    }

    pub async fn get_donation_link(&self, uid: &str) -> Result<Value, MakePayError> {
        assert_non_empty(uid, "Donation link UID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/donations/{}",
                encode_path_segment(uid)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_donation_link<T>(
        &self,
        uid: &str,
        updates: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(uid, "Donation link UID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/donations/{}",
                encode_path_segment(uid)
            ),
            Some(serde_json::to_value(updates)?),
            &[],
        )
        .await
    }

    pub async fn list_customers(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/customers", None, &[])
            .await
    }

    pub async fn upsert_customer<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/customers",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn create_customer_portal(&self, customer_id: &str) -> Result<Value, MakePayError> {
        self.create_customer_portal_with_payload(customer_id, &json!({}))
            .await
    }

    pub async fn create_customer_portal_with_payload<T>(
        &self,
        customer_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(customer_id, "Customer ID is required.")?;

        self.request(
            Method::POST,
            &format!(
                "/api/partner/v1/makepay/customers/{}/portal",
                encode_path_segment(customer_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_subscriptions(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/subscriptions",
            None,
            &[],
        )
        .await
    }

    pub async fn create_subscription<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/subscriptions",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_destination_assets(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/destination-assets",
            None,
            &[],
        )
        .await
    }

    pub async fn list_webhook_requests(
        &self,
        query: &[(&str, &str)],
    ) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/webhook-requests",
            None,
            query,
        )
        .await
    }

    pub async fn list_pos_terminals(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/pos-terminals",
            None,
            &[],
        )
        .await
    }

    pub async fn create_pos_terminal<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/pos-terminals",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_pos_terminal(&self, terminal_id: &str) -> Result<Value, MakePayError> {
        assert_non_empty(terminal_id, "POS terminal ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/pos-terminals/{}",
                encode_path_segment(terminal_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_pos_terminal<T>(
        &self,
        terminal_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(terminal_id, "POS terminal ID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/pos-terminals/{}",
                encode_path_segment(terminal_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_products(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/products", None, &[])
            .await
    }

    pub async fn create_product<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/products",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_product(&self, product_id: &str) -> Result<Value, MakePayError> {
        assert_non_empty(product_id, "Product ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/products/{}",
                encode_path_segment(product_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_product<T>(
        &self,
        product_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(product_id, "Product ID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/products/{}",
                encode_path_segment(product_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_product_downloads(&self, product_id: &str) -> Result<Value, MakePayError> {
        assert_non_empty(product_id, "Product ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/shop/products/{}/downloads",
                encode_path_segment(product_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn create_product_download<T>(
        &self,
        product_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(product_id, "Product ID is required.")?;

        self.request(
            Method::POST,
            &format!(
                "/api/partner/v1/makepay/shop/products/{}/downloads",
                encode_path_segment(product_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_shop(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/shop", None, &[])
            .await
    }

    pub async fn update_shop<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::PATCH,
            "/api/partner/v1/makepay/shop",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_shop_builder(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/shop/builder",
            None,
            &[],
        )
        .await
    }

    pub async fn update_shop_builder<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::PUT,
            "/api/partner/v1/makepay/shop/builder",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_shop_domain(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/shop/domains",
            None,
            &[],
        )
        .await
    }

    pub async fn update_shop_domain(&self, domain: Option<&str>) -> Result<Value, MakePayError> {
        self.update_shop_domain_with_payload(&json!({ "domain": domain }))
            .await
    }

    pub async fn update_shop_domain_with_payload<T>(
        &self,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::PUT,
            "/api/partner/v1/makepay/shop/domains",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn refresh_shop_domain(&self, domain: Option<&str>) -> Result<Value, MakePayError> {
        self.refresh_shop_domain_with_payload(&json!({ "domain": domain }))
            .await
    }

    pub async fn refresh_shop_domain_with_payload<T>(
        &self,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/shop/domains",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_shop_coupons(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/shop/coupons",
            None,
            &[],
        )
        .await
    }

    pub async fn create_shop_coupon<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/shop/coupons",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn update_shop_coupon<T>(
        &self,
        coupon_uid: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(coupon_uid, "Shop coupon UID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/shop/coupons/{}",
                encode_path_segment(coupon_uid)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn archive_shop_coupon(&self, coupon_uid: &str) -> Result<Value, MakePayError> {
        assert_non_empty(coupon_uid, "Shop coupon UID is required.")?;

        self.request(
            Method::DELETE,
            &format!(
                "/api/partner/v1/makepay/shop/coupons/{}",
                encode_path_segment(coupon_uid)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn list_shop_orders(&self, query: &[(&str, &str)]) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/shop/orders",
            None,
            query,
        )
        .await
    }

    pub async fn get_branding(&self) -> Result<Value, MakePayError> {
        self.request(Method::GET, "/api/partner/v1/makepay/branding", None, &[])
            .await
    }

    pub async fn update_branding<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::PATCH,
            "/api/partner/v1/makepay/branding",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn refresh_branding_domains(
        &self,
        kind: Option<&str>,
    ) -> Result<Value, MakePayError> {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/branding/domains/refresh",
            Some(json!({ "kind": kind.unwrap_or("all") })),
            &[],
        )
        .await
    }

    pub async fn get_bookkeeping_summary(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/bookkeeping",
            None,
            &[],
        )
        .await
    }

    pub async fn list_bookkeeping_invoices(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/bookkeeping/invoices",
            None,
            &[],
        )
        .await
    }

    pub async fn create_bookkeeping_invoice<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/bookkeeping/invoices",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_bookkeeping_invoice(&self, invoice_id: &str) -> Result<Value, MakePayError> {
        assert_non_empty(invoice_id, "Bookkeeping invoice ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/invoices/{}",
                encode_path_segment(invoice_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_bookkeeping_invoice<T>(
        &self,
        invoice_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(invoice_id, "Bookkeeping invoice ID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/invoices/{}",
                encode_path_segment(invoice_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn create_bookkeeping_invoice_payment_link(
        &self,
        invoice_id: &str,
    ) -> Result<Value, MakePayError> {
        self.create_bookkeeping_invoice_payment_link_with_options(invoice_id, &json!({}))
            .await
    }

    pub async fn create_bookkeeping_invoice_payment_link_with_options<T>(
        &self,
        invoice_id: &str,
        options: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(invoice_id, "Bookkeeping invoice ID is required.")?;

        self.request(
            Method::POST,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/invoices/{}/payment-link",
                encode_path_segment(invoice_id)
            ),
            Some(serde_json::to_value(options)?),
            &[],
        )
        .await
    }

    pub async fn list_bookkeeping_expenses(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/bookkeeping/expenses",
            None,
            &[],
        )
        .await
    }

    pub async fn create_bookkeeping_expense<T>(&self, payload: &T) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/bookkeeping/expenses",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn create_bookkeeping_expense_from_activity<T>(
        &self,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/bookkeeping/expenses/from-activity",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn get_bookkeeping_expense(&self, expense_id: &str) -> Result<Value, MakePayError> {
        assert_non_empty(expense_id, "Bookkeeping expense ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/expenses/{}",
                encode_path_segment(expense_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn update_bookkeeping_expense<T>(
        &self,
        expense_id: &str,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        assert_non_empty(expense_id, "Bookkeeping expense ID is required.")?;

        self.request(
            Method::PATCH,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/expenses/{}",
                encode_path_segment(expense_id)
            ),
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    pub async fn list_bookkeeping_documents(&self) -> Result<Value, MakePayError> {
        self.request(
            Method::GET,
            "/api/partner/v1/makepay/bookkeeping/documents",
            None,
            &[],
        )
        .await
    }

    pub async fn upload_bookkeeping_document(
        &self,
        form: reqwest::multipart::Form,
    ) -> Result<Value, MakePayError> {
        self.request_multipart(
            Method::POST,
            "/api/partner/v1/makepay/bookkeeping/documents",
            form,
            &[],
        )
        .await
    }

    pub async fn get_bookkeeping_document_download_url(
        &self,
        document_id: &str,
    ) -> Result<Value, MakePayError> {
        assert_non_empty(document_id, "Bookkeeping document ID is required.")?;

        self.request(
            Method::GET,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/documents/{}/download",
                encode_path_segment(document_id)
            ),
            None,
            &[],
        )
        .await
    }

    pub async fn run_bookkeeping_document_ocr(
        &self,
        document_id: &str,
    ) -> Result<Value, MakePayError> {
        assert_non_empty(document_id, "Bookkeeping document ID is required.")?;

        self.request(
            Method::POST,
            &format!(
                "/api/partner/v1/makepay/bookkeeping/documents/{}/ocr",
                encode_path_segment(document_id)
            ),
            Some(json!({})),
            &[],
        )
        .await
    }

    pub async fn create_bookkeeping_reconciliation<T>(
        &self,
        payload: &T,
    ) -> Result<Value, MakePayError>
    where
        T: Serialize + ?Sized,
    {
        self.request(
            Method::POST,
            "/api/partner/v1/makepay/bookkeeping/reconciliation",
            Some(serde_json::to_value(payload)?),
            &[],
        )
        .await
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        query: &[(&str, &str)],
    ) -> Result<Value, MakePayError> {
        let mut url = Url::parse(&format!("{}{}", self.base_url, path))?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        let mut request = self
            .http
            .request(method, url)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, format!("MakePayRust/{VERSION}"))
            .header("x-makecrypto-key-id", &self.key_id)
            .header("x-makecrypto-key-secret", &self.key_secret);

        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").json(&body);
        }

        decode_response(request.send().await?).await
    }

    async fn request_multipart(
        &self,
        method: Method,
        path: &str,
        form: reqwest::multipart::Form,
        query: &[(&str, &str)],
    ) -> Result<Value, MakePayError> {
        let mut url = Url::parse(&format!("{}{}", self.base_url, path))?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }

        let response = self
            .http
            .request(method, url)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, format!("MakePayRust/{VERSION}"))
            .header("x-makecrypto-key-id", &self.key_id)
            .header("x-makecrypto-key-secret", &self.key_secret)
            .multipart(form)
            .send()
            .await?;

        decode_response(response).await
    }
}

pub fn build_hosted_checkout_url(
    payment_uid: &str,
    base_url: Option<&str>,
) -> Result<String, MakePayError> {
    assert_non_empty(payment_uid, "Payment link UID is required.")?;
    build_checkout_url(
        base_url.unwrap_or(DEFAULT_CHECKOUT_BASE_URL),
        &["payment", payment_uid],
        None,
    )
}

pub fn build_hosted_donation_url(
    donation_slug: &str,
    base_url: Option<&str>,
) -> Result<String, MakePayError> {
    assert_non_empty(donation_slug, "Donation slug is required.")?;
    build_checkout_url(
        base_url.unwrap_or(DEFAULT_CHECKOUT_BASE_URL),
        &["donations", donation_slug],
        None,
    )
}

pub fn build_embedded_checkout_url(
    payment_uid: &str,
    base_url: Option<&str>,
    parent_origin: Option<&str>,
) -> Result<String, MakePayError> {
    assert_non_empty(payment_uid, "Payment link UID is required.")?;
    build_checkout_url(
        base_url.unwrap_or(DEFAULT_CHECKOUT_BASE_URL),
        &["embed", "payment", payment_uid],
        parent_origin.map(|origin| ("parentOrigin", origin)),
    )
}

pub fn build_embedded_donation_url(
    donation_slug: &str,
    base_url: Option<&str>,
    parent_origin: Option<&str>,
) -> Result<String, MakePayError> {
    assert_non_empty(donation_slug, "Donation slug is required.")?;
    build_checkout_url(
        base_url.unwrap_or(DEFAULT_CHECKOUT_BASE_URL),
        &["embed", "donations", donation_slug],
        parent_origin.map(|origin| ("parentOrigin", origin)),
    )
}

pub fn build_modal_script_url(base_url: Option<&str>) -> Result<String, MakePayError> {
    build_checkout_url(
        base_url.unwrap_or(DEFAULT_CHECKOUT_BASE_URL),
        &["modal", "makepay.js"],
        None,
    )
}

pub fn build_embed_button_html(
    payment_uid: &str,
    base_url: Option<&str>,
    button_label: Option<&str>,
) -> Result<String, MakePayError> {
    assert_non_empty(payment_uid, "Payment link UID is required.")?;
    let label = button_label.unwrap_or("Pay with crypto");

    Ok([
        format!(
            r#"<script src="{}"></script>"#,
            escape_html_attribute(&build_modal_script_url(base_url)?)
        ),
        format!(
            r#"<button type="button" data-makepay-payment-link="{}">"#,
            escape_html_attribute(payment_uid)
        ),
        format!("  {}", escape_html_text(label)),
        "</button>".to_owned(),
    ]
    .join("\n"))
}

pub fn build_iframe_html(
    payment_uid: &str,
    base_url: Option<&str>,
    iframe_title: Option<&str>,
    parent_origin: Option<&str>,
) -> Result<String, MakePayError> {
    Ok([
        "<iframe".to_owned(),
        format!(
            r#"  title="{}""#,
            escape_html_attribute(iframe_title.unwrap_or("MakePay checkout"))
        ),
        format!(
            r#"  src="{}""#,
            escape_html_attribute(&build_embedded_checkout_url(
                payment_uid,
                base_url,
                parent_origin
            )?)
        ),
        r#"  style="width:100%;min-height:720px;border:0;border-radius:12px;""#.to_owned(),
        r#"  allow="clipboard-read; clipboard-write""#.to_owned(),
        "></iframe>".to_owned(),
    ]
    .join("\n"))
}

pub async fn create_anonymous_payment_link<T>(payload: &T) -> Result<Value, MakePayError>
where
    T: Serialize + ?Sized,
{
    create_anonymous_payment_link_with_options(payload, MakePayPublicRequestOptions::default())
        .await
}

pub async fn create_anonymous_payment_link_with_options<T>(
    payload: &T,
    options: MakePayPublicRequestOptions,
) -> Result<Value, MakePayError>
where
    T: Serialize + ?Sized,
{
    let base_url = normalize_base_url(&options.base_url);
    assert_non_empty(&base_url, "MakePay base URL is required.")?;

    let url = Url::parse(&format!("{base_url}/api/partner/v1/makepay/payment-links"))?;
    let response = options
        .http
        .post(url)
        .header(ACCEPT, "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(payload)
        .send()
        .await?;

    decode_response(response).await
}

pub async fn create_anonymous_makepay_payment_link<T>(payload: &T) -> Result<Value, MakePayError>
where
    T: Serialize + ?Sized,
{
    create_anonymous_payment_link(payload).await
}

pub async fn create_anonymous_makepay_payment_link_with_options<T>(
    payload: &T,
    options: MakePayPublicRequestOptions,
) -> Result<Value, MakePayError>
where
    T: Serialize + ?Sized,
{
    create_anonymous_payment_link_with_options(payload, options).await
}

pub fn verify_webhook(
    raw_body: impl AsRef<[u8]>,
    signature_header: Option<&str>,
    secret: &str,
    options: Option<WebhookVerificationOptions>,
) -> bool {
    let Some(signature_header) = signature_header else {
        return false;
    };
    if signature_header.trim().is_empty() || secret.is_empty() {
        return false;
    }

    let mut timestamp = None;
    let mut signature = None;
    for part in signature_header.split(',') {
        let Some((key, value)) = part.trim().split_once('=') else {
            continue;
        };
        match key {
            "t" => timestamp = value.parse::<u64>().ok(),
            "v1" => signature = Some(value),
            _ => {}
        }
    }

    let Some(timestamp) = timestamp else {
        return false;
    };
    let Some(signature) = signature else {
        return false;
    };
    if signature.is_empty() || !signature.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }

    let tolerance_seconds = options.unwrap_or_default().tolerance_seconds;
    if tolerance_seconds > 0 {
        let Ok(now) = unix_timestamp() else {
            return false;
        };
        if now.abs_diff(timestamp) > tolerance_seconds {
            return false;
        }
    }

    let Ok(actual) = hex::decode(signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(format!("{timestamp}.").as_bytes());
    mac.update(raw_body.as_ref());
    let expected = mac.finalize().into_bytes();

    actual.len() == expected.len() && actual.ct_eq(expected.as_slice()).into()
}

pub fn parse_webhook<T>(
    raw_body: impl AsRef<[u8]>,
    signature_header: Option<&str>,
    secret: &str,
    options: Option<WebhookVerificationOptions>,
) -> Result<T, MakePayError>
where
    T: DeserializeOwned,
{
    let raw_body = raw_body.as_ref();
    if !verify_webhook(raw_body, signature_header, secret, options) {
        return Err(MakePayError::InvalidWebhookSignature);
    }

    serde_json::from_slice(raw_body).map_err(MakePayError::InvalidWebhookJson)
}

async fn decode_response(response: reqwest::Response) -> Result<Value, MakePayError> {
    let status = response.status();
    let text = response.text().await?;
    let decoded = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).unwrap_or_else(|_| json!({}))
    };

    if !status.is_success() {
        let message = decoded
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("MakePay API request failed with HTTP {status}."));

        return Err(MakePayError::Api {
            status,
            message,
            response_body: decoded,
        });
    }

    Ok(decoded)
}

fn donation_payload_value<T>(payload: &T) -> Result<Value, MakePayError>
where
    T: Serialize + ?Sized,
{
    let mut value = serde_json::to_value(payload)?;
    match value {
        Value::Object(ref mut object) => {
            object.insert("type".to_owned(), Value::String("donation".to_owned()));
            Ok(value)
        }
        _ => Ok(json!({
            "type": "donation",
            "value": value,
        })),
    }
}

fn build_checkout_url(
    base_url: &str,
    segments: &[&str],
    query: Option<(&str, &str)>,
) -> Result<String, MakePayError> {
    let mut url = Url::parse(&format!("{}/", normalize_base_url(base_url)))?;
    url.set_path("");
    {
        let mut path_segments = url
            .path_segments_mut()
            .map_err(|_| MakePayError::InvalidInput("MakePay URL cannot be a base URL.".into()))?;
        for segment in segments {
            path_segments.push(segment);
        }
    }

    if let Some((key, value)) = query {
        if !value.is_empty() {
            url.query_pairs_mut().append_pair(key, value);
        }
    }

    Ok(url.to_string())
}

fn assert_non_empty(value: &str, message: &str) -> Result<(), MakePayError> {
    if value.trim().is_empty() {
        return Err(MakePayError::InvalidInput(message.to_owned()));
    }

    Ok(())
}

fn normalize_base_url(value: &str) -> String {
    value.trim_end_matches('/').to_owned()
}

fn encode_path_segment(value: &str) -> String {
    byte_serialize(value.as_bytes()).collect()
}

fn unix_timestamp() -> Result<u64, MakePayError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| MakePayError::InvalidInput("System clock is before Unix epoch.".into()))
}

fn escape_html_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

fn escape_html_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature_header(raw_body: &[u8], secret: &str, timestamp: u64) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{timestamp}.").as_bytes());
        mac.update(raw_body);
        format!(
            "t={timestamp},v1={}",
            hex::encode(mac.finalize().into_bytes())
        )
    }

    #[test]
    fn verifies_and_parses_webhook() {
        let body = br#"{"event":{"type":"status_changed"}}"#;
        let secret = "whsec_test";
        let header = signature_header(body, secret, 1_776_556_800);
        let options = WebhookVerificationOptions {
            tolerance_seconds: 0,
        };

        assert!(verify_webhook(body, Some(&header), secret, Some(options)));
        assert!(!verify_webhook(body, Some(&header), "wrong", Some(options)));

        let parsed: Value = parse_webhook(body, Some(&header), secret, Some(options)).unwrap();
        assert_eq!(parsed["event"]["type"], "status_changed");
    }

    #[test]
    fn builds_checkout_urls() {
        assert_eq!(
            build_hosted_checkout_url("pay_123", None).unwrap(),
            "https://makepay.io/payment/pay_123"
        );
        assert_eq!(
            build_embedded_checkout_url(
                "pay_123",
                Some("https://pay.example/"),
                Some("https://merchant.example")
            )
            .unwrap(),
            "https://pay.example/embed/payment/pay_123?parentOrigin=https%3A%2F%2Fmerchant.example"
        );
        assert_eq!(
            build_hosted_donation_url("spring-campaign", None).unwrap(),
            "https://makepay.io/donations/spring-campaign"
        );
        assert_eq!(
            build_embedded_donation_url(
                "spring-campaign",
                Some("https://pay.example/"),
                Some("https://merchant.example")
            )
            .unwrap(),
            "https://pay.example/embed/donations/spring-campaign?parentOrigin=https%3A%2F%2Fmerchant.example"
        );
        assert_eq!(
            build_modal_script_url(Some("https://pay.example/")).unwrap(),
            "https://pay.example/modal/makepay.js"
        );
    }

    #[test]
    fn escapes_html_snippets() {
        let button = build_embed_button_html(r#"pay_"<&"#, None, Some("Pay <now>")).unwrap();
        assert!(button.contains(r#"data-makepay-payment-link="pay_&quot;&lt;&amp;""#));
        assert!(button.contains("Pay &lt;now&gt;"));

        let iframe = build_iframe_html("pay_123", None, Some("Secure checkout"), None).unwrap();
        assert!(iframe.contains(r#"src="https://makepay.io/embed/payment/pay_123""#));
    }

    #[test]
    fn validates_client_configuration() {
        let error = MakePayClient::new("", "").unwrap_err();
        assert!(matches!(error, MakePayError::InvalidInput(_)));

        let client = MakePayClient::builder()
            .key_id("mk_test")
            .key_secret("mksec_test")
            .checkout_base_url("https://checkout.example/")
            .build()
            .unwrap();
        assert_eq!(
            client.hosted_checkout_url("pay_123").unwrap(),
            "https://checkout.example/payment/pay_123"
        );
        assert_eq!(
            client.hosted_donation_url("spring-campaign").unwrap(),
            "https://checkout.example/donations/spring-campaign"
        );
    }

    #[test]
    fn prepares_donation_payloads() {
        let donation = donation_payload_value(&json!({
            "title": "Spring campaign",
            "defaultAmountUsd": "25"
        }))
        .unwrap();
        assert_eq!(donation["type"], "donation");
        assert_eq!(donation["title"], "Spring campaign");
    }
}
