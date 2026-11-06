use super::{ApiError, Operation, Reply, Session, uuid_like};
use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Clone)]
pub(super) struct Api {
    pub(super) base: String,
    csrf: String,
    client: reqwest::Client,
}

impl Api {
    pub(super) fn new() -> Result<Self, String> {
        let window = web_sys::window().ok_or("Browser window is unavailable.")?;
        let config = window
            .document()
            .and_then(|document| document.get_element_by_id("makibishi-config"))
            .ok_or("Makibishi page configuration is missing.")?;
        let csrf = config
            .get_attribute("data-csrf")
            .filter(|value| !value.is_empty())
            .ok_or("Makibishi CSRF configuration is missing.")?;
        Ok(Self {
            base: window
                .location()
                .origin()
                .map_err(|error| format!("{error:?}"))?,
            csrf,
            client: reqwest::Client::new(),
        })
    }

    pub(super) async fn execute(&self, operation: Operation) -> Result<Reply, ApiError> {
        let request = match operation {
            Operation::Restore => {
                #[derive(Deserialize)]
                struct Account {
                    username: Option<String>,
                }
                let account: Account = response(
                    self.client
                        .get(format!("{}/api/navigation/account", self.base)),
                )
                .await?;
                return Ok(Reply::Restored(account.username.map(|name| Session {
                    token: String::new(),
                    display_name: name,
                })));
            }
            Operation::Login { .. } | Operation::Logout => {
                return Err("Use the website navigation to sign in or out."
                    .to_string()
                    .into());
            }
            Operation::Scores => {
                return Ok(Reply::Scores(
                    response(
                        self.client
                            .get(format!("{}/games/minesweeper/api/scores", self.base)),
                    )
                    .await?,
                ));
            }
            Operation::Start { mode } => self
                .client
                .post(format!("{}/games/minesweeper/api/games", self.base))
                .json(&serde_json::json!({
                    "mode": mode, "csrf": self.csrf, "require_signed_in": true
                })),
            Operation::Move {
                id,
                action,
                row,
                column,
            } => self.game_request(&id, "/move")?.json(&serde_json::json!({
                "csrf": self.csrf, "action": action, "row": row, "column": column
            })),
            Operation::Refresh { id } => {
                if uuid_like(&id).is_none() {
                    return Err("Invalid game ID.".to_string().into());
                }
                self.client
                    .get(format!("{}/games/minesweeper/api/games/{id}", self.base))
            }
            Operation::Submit { id } => {
                let _: serde_json::Value = response(
                    self.game_request(&id, "/score")?
                        .json(&serde_json::json!({ "csrf": self.csrf })),
                )
                .await?;
                return Ok(Reply::Submitted);
            }
        };
        Ok(Reply::Board(response(request).await?))
    }

    fn game_request(&self, id: &str, suffix: &str) -> Result<reqwest::RequestBuilder, ApiError> {
        if uuid_like(id).is_none() {
            return Err("Invalid game ID.".to_string().into());
        }
        Ok(self.client.post(format!(
            "{}/games/minesweeper/api/games/{id}{suffix}",
            self.base
        )))
    }
}

async fn response<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T, ApiError> {
    let response = request
        .fetch_credentials_same_origin()
        .fetch_cache_no_store()
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| ApiError {
            message: format!("Cannot reach the server: {error}"),
            unauthorized: false,
            retryable: true,
        })?;
    let status = response.status();
    if !status.is_success() {
        let message = response
            .text()
            .await
            .map_err(|error| ApiError::from(error.to_string()))?;
        return Err(ApiError {
            message: format!(
                "Server error {status}: {}",
                message.chars().take(300).collect::<String>()
            ),
            unauthorized: matches!(
                status,
                reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
            ),
            retryable: status.is_server_error()
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::REQUEST_TIMEOUT,
        });
    }
    response
        .json()
        .await
        .map_err(|error| format!("Invalid server response: {error}").into())
}

pub(crate) fn navigate(path: &str) -> Result<(), String> {
    web_sys::window()
        .ok_or("Browser window is unavailable.")?
        .location()
        .set_href(path)
        .map_err(|error| format!("Cannot open website: {error:?}"))
}

pub(crate) fn open(url: &str) -> Result<(), String> {
    web_sys::window()
        .ok_or("Browser window is unavailable.")?
        .open_with_url_and_target(url, "_blank")
        .map_err(|error| format!("Cannot open winning board: {error:?}"))?
        .ok_or("The browser blocked the winning-board window.")?;
    Ok(())
}
