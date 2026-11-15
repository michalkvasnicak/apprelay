use utoipa::{
    Modify, OpenApi,
    openapi::{
        OpenApi as Document,
        security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme},
    },
};

#[derive(OpenApi)]
#[openapi(
    info(title = "AppRelay API", version = "1.0.0"),
    paths(
        super::health,
        super::register_device,
        super::pairing_codes::get,
        super::pairing_attempts::create,
        super::pairing_attempts::get,
        super::pairing_attempts::start,
        super::push_tokens::put,
        super::push_tokens::delete
    ),
    modifiers(&Security)
)]
struct Api;

struct Security;

impl Modify for Security {
    fn modify(&self, openapi: &mut Document) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "app_key",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-App-Key"))),
        );
        components.add_security_scheme(
            "device_token",
            SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
        );
    }
}

/// Generate the contract without initializing application infrastructure.
pub fn document() -> Document {
    Api::openapi()
}
