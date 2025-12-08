use codex_client::Request;

use crate::provider::WireApi;

/// Provides bearer and account identity information for API requests.
///
/// Implementations should be cheap and non-blocking; any asynchronous
/// refresh or I/O should be handled by higher layers before requests
/// reach this interface.
pub trait AuthProvider: Send + Sync {
    fn bearer_token(&self) -> Option<String>;
    fn account_id(&self) -> Option<String> {
        None
    }
}

pub(crate) fn add_auth_headers<A: AuthProvider>(
    auth: &A,
    mut req: Request,
    wire_api: &WireApi,
) -> Request {
    if let Some(token) = auth.bearer_token() {
        match wire_api {
            WireApi::Anthropic => {
                // Anthropic uses x-api-key header
                if let Ok(header) = token.parse() {
                    let _ = req.headers.insert("x-api-key", header);
                }
            }
            WireApi::Gemini => {
                // Gemini uses API key as query parameter, handled elsewhere
                // But also support x-goog-api-key header
                if let Ok(header) = token.parse() {
                    let _ = req.headers.insert("x-goog-api-key", header);
                }
            }
            _ => {
                // OpenAI and others use Bearer token
                if let Ok(header) = format!("Bearer {token}").parse() {
                    let _ = req.headers.insert(http::header::AUTHORIZATION, header);
                }
            }
        }
    }
    if let Some(account_id) = auth.account_id()
        && let Ok(header) = account_id.parse()
    {
        let _ = req.headers.insert("ChatGPT-Account-ID", header);
    }
    req
}
